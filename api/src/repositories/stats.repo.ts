import type pg from "pg";

import type { CrawlSnapshotRow } from "../types/api.js";

/**
 * Pure SQL access to `crawl_stats_history`. Windowed rates (verified/hr,
 * unique/hr, bandwidth bps) are derived with LAG() over consecutive rows —
 * no app-side math drift.
 */
export class StatsRepository {
  constructor(private readonly pool: pg.Pool) {}

  async latest(): Promise<CrawlSnapshotRow | null> {
    const { rows } = await this.pool.query<CrawlSnapshotRow>(
      "SELECT * FROM crawl_stats_history ORDER BY ts DESC LIMIT 1",
    );
    return rows[0] ?? null;
  }

  /**
   * History for a metric over a range, with per-row derived rates (delta vs
   * the previous row) for cumulative counters.
   *
   * @param metric   a column name on crawl_stats_history
   * @param rangeSecs lookback window in seconds
   * @param stepSecs  optional downsampling bucket (0 = raw rows)
   */
  async history(
    metric: string,
    rangeSecs: number,
    stepSecs = 0,
  ): Promise<Array<{ ts: string; value: number | null }>> {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const sql = `
      SELECT ts, "${metric}" AS value
      FROM crawl_stats_history
      WHERE ts >= ${since}
      ORDER BY ts ASC
    `;
    const { rows } = await this.pool.query<{
      ts: string;
      value: number | null;
    }>(sql);
    if (stepSecs <= 0) {
      return rows;
    }
    return downsample(rows, stepSecs);
  }

  /**
   * Rate history for a cumulative counter: delta vs previous row / time.
   *
   * The crawler persists `process_start_ts` with every row; when it changes
   * between two consecutive rows, the counters reset to zero on restart, so the
   * inter-row delta is meaningless — emit NULL for that boundary instead of a
   * bogus spike, and for the first row(s) after a restart. Rows are then
   * smoothed with a trailing moving average (default window 3 rows) so the
   * discrete 30s-tick increments of counters like `records_persisted` do not
   * render as wild per-tick spikes (0/hr, 120/hr, 600/hr).
   */
  async rateHistory(
    metric: string,
    rangeSecs: number,
    perSecs = 3600,
    smoothRows = 3,
  ): Promise<Array<{ ts: string; value: number | null }>> {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const sql = `
      SELECT ts,
        CASE
          WHEN smooth_cnt = 0 THEN NULL
          ELSE avg_value
        END AS value
      FROM (
        SELECT ts,
               AVG(bound) OVER (ORDER BY ts ROWS BETWEEN
                 ${smoothRows - 1} PRECEDING AND CURRENT ROW) AS avg_value,
               COUNT(bound) OVER (ORDER BY ts ROWS BETWEEN
                 ${smoothRows - 1} PRECEDING AND CURRENT ROW) AS smooth_cnt
        FROM (
          SELECT ts,
            CASE
              WHEN prev_ts IS NULL OR prev_ts = ts OR prev_val IS NULL
                   OR "${metric}" < prev_val
                   OR prev_start IS NULL
                   OR prev_start != process_start_ts
              THEN NULL
              ELSE ("${metric}" - prev_val)::float8
                   / (EXTRACT(EPOCH FROM (ts - prev_ts)))
                   * ${perSecs}
            END AS bound
          FROM (
            SELECT ts, "${metric}", process_start_ts,
                   LAG("${metric}") OVER (ORDER BY ts) AS prev_val,
                   LAG(process_start_ts) OVER (ORDER BY ts) AS prev_start,
                   LAG(ts) OVER (ORDER BY ts) AS prev_ts
            FROM crawl_stats_history
            WHERE ts >= ${since}
          ) t
        ) r
      ) s
      ORDER BY ts ASC
    `;
    const { rows } = await this.pool.query<{
      ts: string;
      value: number | null;
    }>(sql);
    return rows;
  }

  /** Failure breakdown over a range, by failure_reason. */
  async failures(rangeSecs: number): Promise<Array<{ reason: string; count: string }>> {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const sql = `
      SELECT COALESCE(failure_reason, 'unknown') AS reason, COUNT(*)::text AS count
      FROM scanned
      WHERE status = 'failed' AND last_attempt >= EXTRACT(EPOCH FROM now() - interval '${rangeSecs} seconds')::bigint
      GROUP BY COALESCE(failure_reason, 'unknown')
      ORDER BY COUNT(*) DESC
    `;
    const { rows } = await this.pool.query<{ reason: string; count: string }>(sql);
    return rows;
  }
}

function downsample(
  rows: Array<{ ts: string; value: number | null }>,
  stepSecs: number,
): Array<{ ts: string; value: number | null }> {
  const out: Array<{ ts: string; value: number | null }> = [];
  let current: { ts: string; sum: number; n: number } | null = null;
  for (const r of rows) {
    const bucket = bucketStart(r.ts, stepSecs);
    if (!current || bucket !== current.ts) {
      if (current) {
        out.push({ ts: current.ts, value: current.n ? current.sum / current.n : null });
      }
      current = { ts: bucket, sum: 0, n: 0 };
    }
    if (r.value !== null) {
      current.sum += r.value;
      current.n += 1;
    }
  }
  if (current) {
    out.push({ ts: current.ts, value: current.n ? current.sum / current.n : null });
  }
  return out;
}

function bucketStart(iso: string, stepSecs: number): string {
  const t = new Date(iso).getTime();
  const bucketed = Math.floor(t / (stepSecs * 1000)) * stepSecs * 1000;
  return new Date(bucketed).toISOString();
}
