import type pg from "pg";

/**
 * Pure SQL access to system resource time series (network, cpu, memory, disk)
 * from `crawl_stats_history`. Bandwidth rates come pre-computed from the
 * crawler (net_rx_rate_bps / net_tx_rate_bps).
 */
export class SystemRepository {
  constructor(private readonly pool: pg.Pool) {}

  async network(rangeSecs: number): Promise<
    Array<{
      ts: string;
      rx_rate: number | null;
      tx_rate: number | null;
      rx_bytes: number;
      tx_bytes: number;
    }>
  > {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const { rows } = await this.pool.query(
      `SELECT ts, net_rx_rate_bps AS rx_rate, net_tx_rate_bps AS tx_rate,
              net_rx_bytes AS rx_bytes, net_tx_bytes AS tx_bytes
       FROM crawl_stats_history
       WHERE ts >= ${since}
       ORDER BY ts ASC`,
    );
    return rows;
  }

  async memory(rangeSecs: number): Promise<
    Array<{
      ts: string;
      host_total: number;
      host_available: number;
      container_current: number;
    }>
  > {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const { rows } = await this.pool.query(
      `SELECT ts, host_mem_total AS host_total, host_mem_available AS host_available,
              container_mem_current AS container_current
       FROM crawl_stats_history
       WHERE ts >= ${since}
       ORDER BY ts ASC`,
    );
    return rows;
  }

  async cpu(rangeSecs: number): Promise<Array<{ ts: string; cpu_percent: number | null }>> {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const { rows } = await this.pool.query(
      `SELECT ts, cpu_percent FROM crawl_stats_history WHERE ts >= ${since} ORDER BY ts ASC`,
    );
    return rows;
  }

  async disk(rangeSecs: number): Promise<
    Array<{ ts: string; total: number; free: number }>
  > {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const { rows } = await this.pool.query(
      `SELECT ts, disk_total_bytes AS total, disk_free_bytes AS free
       FROM crawl_stats_history WHERE ts >= ${since} ORDER BY ts ASC`,
    );
    return rows;
  }

  async loadavg(rangeSecs: number): Promise<
    Array<{ ts: string; load1: number | null; load5: number | null; load15: number | null }>
  > {
    const since = `now() - interval '${rangeSecs} seconds'`;
    const { rows } = await this.pool.query(
      `SELECT ts, loadavg_1 AS load1, loadavg_5 AS load5, loadavg_15 AS load15
       FROM crawl_stats_history WHERE ts >= ${since} ORDER BY ts ASC`,
    );
    return rows;
  }
}
