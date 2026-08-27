import express from 'express';
import cors from 'cors';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import pg from 'pg';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const { Pool } = pg;

const pool = new Pool({
  connectionString:
    process.env.DATABASE_URL ||
    'postgres://crawler:change-me@127.0.0.1:55432/craw?sslmode=disable',
  max: 10,
  idleTimeoutMillis: 30000,
  connectionTimeoutMillis: 5000,
});

const app = express();
app.use(cors());
app.use(express.json());

const HOST = process.env.HOST || '0.0.0.0';
const PORT = parseInt(process.env.PORT || '3000', 10);
const METRICS_CACHE_MS = parseInt(process.env.METRICS_CACHE_MS || '15000', 10);
const STATS_CACHE_MS = parseInt(process.env.STATS_CACHE_MS || '30000', 10);
let metricsCache = { ts: 0, data: null };
let statsCache = { ts: 0, data: null };

const SORTS = { verified_at: 'verified_at', size: 'total_size', files: 'file_count', name: 'name' };
const INTERVALS = ['minute', 'hour', 'day'];

function escapeLike(s) {
  return s.replace(/[\\%_]/g, (c) => '\\' + c);
}

async function query(text, params) {
  try {
    return await pool.query(text, params);
  } catch (err) {
    console.error('db error:', err.message, '\n', text.slice(0, 300));
    throw err;
  }
}

// GET /api/torrents?search=&sort=verified_at|size|files|name&order=asc|desc&page=&limit=
app.get('/api/torrents', async (req, res) => {
  try {
    const search = (req.query.search || '').trim();
    const sort = SORTS[req.query.sort] || null;
    const orderDir = req.query.order === 'asc' ? 'ASC' : 'DESC';
    const page = Math.max(1, parseInt(req.query.page, 10) || 1);
    const limit = Math.min(100, Math.max(1, parseInt(req.query.limit, 10) || 25));
    const offset = (page - 1) * limit;
    const hasSearch = search.length > 0;

    const params = [];
    let where = '';
    let orderBy = sort ? `ORDER BY ${sort} ${orderDir}` : 'ORDER BY verified_at DESC';
    if (hasSearch) {
      params.push(`%${escapeLike(search)}%`, search);
      where = `WHERE (name ILIKE $1 ESCAPE '\\' OR name % $2)`;
      if (!sort) orderBy = 'ORDER BY similarity(name, $2) DESC';
    }

    const rowsRes = await query(
      `SELECT encode(infohash, 'hex') AS infohash, name, total_size, file_count, verified_at
       FROM torrents ${where} ${orderBy}
       LIMIT ${limit} OFFSET ${offset}`,
      params
    );
    const totalRes = await query(`SELECT count(*) AS total FROM torrents ${where}`, params);

    res.json({
      data: rowsRes.rows,
      page,
      limit,
      total: parseInt(totalRes.rows[0].total, 10),
      pages: Math.max(1, Math.ceil(totalRes.rows[0].total / limit)),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/torrents/:infohash
app.get('/api/torrents/:infohash', async (req, res) => {
  const ih = String(req.params.infohash || '').toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(ih)) {
    return res.status(400).json({ error: 'infohash must be 40 hex chars' });
  }
  try {
    const r = await query(
      `SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.piece_length, t.total_size,
              t.file_count, t.files, t.fetch_attempts, t.verified_at,
              s.first_seen, s.last_seen, s.total_seen, s.source_counts
       FROM torrents t
       LEFT JOIN infohash_sightings s ON s.infohash = t.infohash
       WHERE t.infohash = decode($1, 'hex')`,
      [ih]
    );
    if (r.rows.length === 0) return res.status(404).json({ error: 'not found' });
    res.json(r.rows[0]);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/torrents/:infohash/magnet
app.get('/api/torrents/:infohash/magnet', async (req, res) => {
  const ih = String(req.params.infohash || '').toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(ih)) {
    return res.status(400).json({ error: 'infohash must be 40 hex chars' });
  }
  try {
    const r = await query(
      `SELECT encode(infohash, 'hex') AS ih, name FROM torrents WHERE infohash = decode($1, 'hex')`,
      [ih]
    );
    if (r.rows.length === 0) return res.status(404).json({ error: 'not found' });
    const { ihhex, name } = { ihhex: r.rows[0].ih, ...r.rows[0] };
    const magnet = `magnet:?xt=urn:btih:${ihhex}${
      name ? `&dn=${encodeURIComponent(name)}` : ''
    }`;
    res.json({ magnet });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/metrics/current
app.get('/api/metrics/current', async (req, res) => {
  const now = Date.now();
  if (metricsCache.ts && now - metricsCache.ts < METRICS_CACHE_MS) {
    return res.json(metricsCache.data);
  }
  try {
    const r = await query(
       `WITH session_start AS (
         SELECT ts FROM metrics WHERE metric_name = '_session_start'
         ORDER BY ts DESC LIMIT 1
       ),
       cur AS (
         SELECT DISTINCT ON (metric_name) metric_name, metric_value, ts
         FROM metrics
         WHERE metric_name != '_session_start'
           AND ts >= (SELECT ts FROM session_start)
         ORDER BY metric_name, ts DESC
       )
       SELECT c.metric_name,
              c.metric_value AS current_value,
              c.ts,
              COALESCE(prev.metric_value, 0) AS value_1h_ago,
              EXTRACT(EPOCH FROM (c.ts - prev.ts)) / 3600.0 AS hours_elapsed
       FROM cur c
       LEFT JOIN LATERAL (
           SELECT metric_value, ts FROM metrics m
           WHERE m.metric_name = c.metric_name
             AND m.ts <= c.ts - interval '1 hour'
           ORDER BY m.ts DESC LIMIT 1
         ) prev ON true
       ORDER BY c.metric_name`
    );
    const snapshot = {};
    const rates = {};
    r.rows.forEach((row) => {
      snapshot[row.metric_name] = Number(row.current_value);
      if (
        row.hours_elapsed &&
        row.hours_elapsed > 0 &&
        row.current_value >= row.value_1h_ago
      ) {
        rates[row.metric_name] = Number(
          (row.current_value - row.value_1h_ago) / row.hours_elapsed
        );
      } else {
        rates[row.metric_name] = null;
      }
    });
    metricsCache = {
      ts: Date.now(),
      data: { ts: r.rows[0]?.ts ?? null, snapshot, rates },
    };
    res.json(metricsCache.data);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/metrics/history?metric=&from=&to=&interval=
app.get('/api/metrics/history', async (req, res) => {
  const metric = String(req.query.metric || '');
  if (!metric || metric.startsWith('_')) return res.status(400).json({ error: 'metric is required' });
  const interval = INTERVALS.includes(req.query.interval) ? req.query.interval : 'minute';
  const to = req.query.to ? new Date(req.query.to) : new Date();
  const from = req.query.from
    ? new Date(req.query.from)
    : new Date(to.getTime() - 3600 * 1000);
  try {
    const r = await query(
      `WITH session_start AS (
         SELECT ts FROM metrics WHERE metric_name = '_session_start'
         ORDER BY ts DESC LIMIT 1
       )
       SELECT extract(epoch FROM date_trunc('${interval}', m.ts)) * 1000 AS t,
              (array_agg(m.metric_value ORDER BY m.ts DESC))[1] AS value
       FROM metrics m
       JOIN session_start s ON m.ts >= s.ts
       WHERE m.metric_name = $1 AND m.ts >= $2 AND m.ts <= $3
       GROUP BY 1 ORDER BY 1`,
      [metric, from, to]
    );
    res.json({ metric, interval, data: r.rows.map((row) => ({ t: Number(row.t), value: Number(row.value) })) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/stats
app.get('/api/stats', async (req, res) => {
  const now = Date.now();
  if (statsCache.ts && now - statsCache.ts < STATS_CACHE_MS) {
    return res.json(statsCache.data);
  }
  try {
    const [total, v1h, v24h, seen1h, new1h, jobs, heart, sessionUp] = await Promise.all([
      query(`SELECT count(*) AS n FROM torrents`),
      query(`SELECT count(*) AS n FROM torrents WHERE verified_at > now() - interval '1 hour'`),
      query(`SELECT count(*) AS n FROM torrents WHERE verified_at > now() - interval '24 hours'`),
      query(`SELECT count(*) AS n FROM infohash_sightings WHERE last_seen > now() - interval '1 hour'`),
      query(`SELECT count(*) AS n FROM infohash_sightings WHERE first_seen > now() - interval '1 hour'`),
      query(
        `SELECT count(*) FILTER (WHERE status IN ('pending', 'verifying', 'failed')) AS backlog,
                count(*) FILTER (WHERE status = 'verifying') AS verifying
         FROM verification_jobs`
      ),
      query(`SELECT max(ts) AS ts FROM metrics`),
      query(`SELECT EXTRACT(EPOCH FROM (now() - ts))::int AS uptime_s
             FROM metrics WHERE metric_name = '_session_start' ORDER BY ts DESC LIMIT 1`),
    ]);

    const heartbeat = heart.rows[0].ts ? new Date(heart.rows[0].ts) : null;
    const data = {
      total_torrents: parseInt(total.rows[0].n, 10),
      verified_last_1h: parseInt(v1h.rows[0].n ?? 0, 10),
      verified_last_24h: parseInt(v24h.rows[0].n ?? 0, 10),
      seen_last_1h: parseInt(seen1h.rows[0].n ?? 0, 10),
      new_last_1h: parseInt(new1h.rows[0].n ?? 0, 10),
      queue_backlog: parseInt(jobs.rows[0].backlog, 10),
      verifying: parseInt(jobs.rows[0].verifying, 10),
      crawler_heartbeat_ts: heartbeat,
      crawler_stale_s: heartbeat ? Math.round((Date.now() - heartbeat.getTime()) / 1000) : null,
      session_uptime_s: sessionUp.rows[0]?.uptime_s ?? null,
    };
    statsCache = { ts: Date.now(), data };
    res.json(data);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/health', (req, res) => res.json({ ok: true, now: new Date().toISOString() }));

const dist = path.join(__dirname, 'client', 'dist');
app.use(express.static(dist));
app.get(/^(?!\/api)/, (req, res) => res.sendFile(path.join(dist, 'index.html')));

app.listen(PORT, HOST, () => {
  console.log(`dashboard listening on ${HOST}:${PORT}`);
});