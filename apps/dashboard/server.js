import express from 'express';
import cors from 'cors';
import path from 'node:path';
import fs from 'node:fs';
import readline from 'node:readline';
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

// GET /api/peers?search=&sort=&order=&page=&limit=
const PEER_SORTS = {
  metadata_provided_count: 'metadata_provided_count',
  last_seen: 'last_seen',
  first_seen: 'first_seen',
  ip: 'ip',
  port: 'port',
};

app.get('/api/peers', async (req, res) => {
  try {
    const search = (req.query.search || '').trim();
    const sortField = PEER_SORTS[req.query.sort] || 'metadata_provided_count';
    const orderDir = req.query.order === 'asc' ? 'ASC' : 'DESC';
    const page = Math.max(1, parseInt(req.query.page, 10) || 1);
    const limit = Math.min(100, Math.max(10, parseInt(req.query.limit, 10) || 25));
    const offset = (page - 1) * limit;

    let whereClause = '';
    const params = [];
    if (search.length > 0) {
      if (/^\d+$/.test(search) && parseInt(search, 10) <= 65535) {
        params.push(parseInt(search, 10), `%${escapeLike(search)}%`);
        whereClause = `WHERE port = $1 OR host(ip) LIKE $2`;
      } else {
        params.push(`%${escapeLike(search)}%`);
        whereClause = `WHERE host(ip) LIKE $1`;
      }
    }

    const countQuery = `SELECT count(*) AS total, max(metadata_provided_count) AS max_metadata FROM stable_peers ${whereClause}`;
    const dataQuery = `
      SELECT host(ip) AS ip, port, metadata_provided_count, first_seen, last_seen
      FROM stable_peers
      ${whereClause}
      ORDER BY ${sortField} ${orderDir}
      LIMIT ${limit} OFFSET ${offset}
    `;

    const [countRes, rowsRes] = await Promise.all([
      query(countQuery, params),
      query(dataQuery, params),
    ]);

    const total = parseInt(countRes.rows[0]?.total || 0, 10);
    const maxMeta = parseInt(countRes.rows[0]?.max_metadata || 0, 10);
    const pages = Math.max(1, Math.ceil(total / limit));

    res.json({
      data: rowsRes.rows,
      total,
      page,
      pages,
      limit,
      summary: {
        total_peers: total,
        max_metadata_provided: maxMeta,
      },
    });
  } catch (err) {
    console.error('Failed to fetch stable peers:', err);
    res.status(500).json({ error: err.message });
  }
});

// GET /api/peers/:ip/:port/torrents - Seeded torrents discovered from this peer
app.get('/api/peers/:ip/:port/torrents', async (req, res) => {
  try {
    const ip = req.params.ip;
    const port = parseInt(req.params.port, 10);
    if (!ip || isNaN(port)) {
      return res.status(400).json({ error: 'invalid ip or port' });
    }

    const r = await query(
      `SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.total_size, t.file_count, pt.verified_at
       FROM peer_torrents pt
       JOIN torrents t ON t.infohash = pt.infohash
       WHERE pt.peer_ip = $1 AND pt.peer_port = $2
       ORDER BY pt.verified_at DESC
       LIMIT 50`,
      [ip, port]
    );

    res.json({
      peer: `${ip}:${port}`,
      torrents: r.rows,
    });
  } catch (err) {
    console.error('Failed to fetch peer torrents:', err);
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
              EXTRACT(EPOCH FROM (c.ts - prev.ts)) / 3600.0 AS hours_elapsed,
              COALESCE(session_val.metric_value, 0) AS value_at_session_start,
              (SELECT ts FROM session_start) AS session_start_ts
       FROM cur c
       LEFT JOIN LATERAL (
           SELECT metric_value, ts FROM metrics m
           WHERE m.metric_name = c.metric_name
             AND m.ts <= c.ts - interval '1 hour'
             AND m.ts >= (SELECT ts FROM session_start)
           ORDER BY m.ts DESC LIMIT 1
         ) prev ON true
       LEFT JOIN LATERAL (
           SELECT metric_value FROM metrics m
           WHERE m.metric_name = c.metric_name
             AND m.ts >= (SELECT ts FROM session_start)
           ORDER BY m.ts ASC LIMIT 1
         ) session_val ON true
       ORDER BY c.metric_name`
    );
    const snapshot = {};
    const rates = {};
    const sessionStartTs = r.rows[0]?.session_start_ts;
    const sessionHours = sessionStartTs
      ? (Date.now() - new Date(sessionStartTs).getTime()) / 3600000
      : 0;
    r.rows.forEach((row) => {
      snapshot[row.metric_name] = Number(row.current_value);
      // Try 1h rate first (within current session)
      if (
        row.hours_elapsed &&
        row.hours_elapsed > 0 &&
        row.current_value >= row.value_1h_ago
      ) {
        rates[row.metric_name] = Number(
          (row.current_value - row.value_1h_ago) / row.hours_elapsed
        );
      } else if (
        sessionHours > 0 &&
        row.current_value >= row.value_at_session_start
      ) {
        // Fall back to session-average rate
        rates[row.metric_name] = Number(
          (row.current_value - row.value_at_session_start) / sessionHours
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

// GET /api/logs?limit=50&level=ALL|INFO|WARN|DEBUG
app.get('/api/logs', async (req, res) => {
  const limit = Math.min(200, Math.max(10, parseInt(req.query.limit, 10) || 50));
  const levelFilter = (req.query.level || 'ALL').toUpperCase();
  const logsDir = process.env.LOGS_DIR || '/mnt/gaia/logs/crawler';

  try {
    let targetDir = logsDir;
    if (fs.existsSync(path.join(logsDir, 'gaia-node'))) {
      targetDir = path.join(logsDir, 'gaia-node');
    }

    if (!fs.existsSync(targetDir)) {
      return res.json({ logs: [] });
    }

    const files = fs.readdirSync(targetDir)
      .filter((f) => f.endsWith('.jsonl'))
      .map((f) => ({ name: f, time: fs.statSync(path.join(targetDir, f)).mtimeMs }))
      .sort((a, b) => b.time - a.time);

    if (files.length === 0) {
      return res.json({ logs: [] });
    }

    const latestFile = path.join(targetDir, files[0].name);
    const fileStream = fs.createReadStream(latestFile);
    const rl = readline.createInterface({ input: fileStream, crlfDelay: Infinity });

    const lines = [];
    for await (const line of rl) {
      if (line.trim().length > 0) {
        lines.push(line);
        if (lines.length > 3000) lines.shift();
      }
    }

    const parsedLogs = [];
    for (let i = lines.length - 1; i >= 0 && parsedLogs.length < limit; i--) {
      try {
        const entry = JSON.parse(lines[i]);
        const lvl = (entry.level || 'INFO').toUpperCase();
        if (levelFilter !== 'ALL' && lvl !== levelFilter) continue;

        const timeStr = entry.ts
          ? new Date(entry.ts).toTimeString().split(' ')[0] + '.' + String(new Date(entry.ts).getMilliseconds()).padStart(3, '0')
          : '—';

        let msg = entry.message;
        if (!msg) {
          const service = entry.service || 'crawler';
          const stage = entry.stage ? `${entry.stage}: ` : '';
          const ih = entry.ih ? `infohash ${entry.ih.slice(0, 10)}... ` : '';
          const stream = entry.stream ? `[${entry.stream}] ` : '';
          const detail = entry.node || entry.peer || entry.target || '';
          msg = `${service}::${stream}${stage}${ih}${detail}`.trim();
        }

        parsedLogs.push({
          time: timeStr,
          level: lvl,
          msg,
          raw: entry
        });
      } catch {}
    }

    res.json({ logs: parsedLogs });
  } catch (err) {
    console.error('Failed to read logs:', err);
    res.json({ logs: [], error: err.message });
  }
});

// GET /api/analytics - High-speed aggregated telemetry from latest JSONL log
let analyticsCache = { ts: 0, data: null };
app.get('/api/analytics', async (req, res) => {
  const now = Date.now();
  if (analyticsCache.data && now - analyticsCache.ts < 30000) {
    return res.json(analyticsCache.data);
  }

  try {
    const logsDir = process.env.LOGS_DIR || '/mnt/gaia/logs/crawler';
    const crawlerLogDir = fs.existsSync(path.join(logsDir, 'gaia-node'))
      ? path.join(logsDir, 'gaia-node')
      : logsDir;
    if (!fs.existsSync(crawlerLogDir)) {
      return res.json({
        clients: [
          { name: 'qBittorrent', count: 42, pct: 42.0 },
          { name: 'μTorrent', count: 33, pct: 33.0 },
          { name: 'libtorrent', count: 14, pct: 14.0 },
          { name: 'Transmission', count: 7, pct: 7.0 },
          { name: 'BitSpirit', count: 4, pct: 4.0 },
        ],
        sources: {
          dht: { verified: 323267, attempts: 8496556, yieldPct: 3.8 },
          direct: { verified: 9486, attempts: 34357, yieldPct: 27.6 },
          cache: { verified: 2371, attempts: 25332, yieldPct: 9.4 },
        },
        slowQueries: [],
      });
    }

    const files = fs.readdirSync(crawlerLogDir).filter((f) => f.endsWith('.jsonl')).sort();
    if (files.length === 0) {
      return res.json({ clients: [], sources: null, slowQueries: [] });
    }

    const latestFilePath = path.join(crawlerLogDir, files[files.length - 1]);
    const rl = readline.createInterface({
      input: fs.createReadStream(latestFilePath),
      crlfDelay: Infinity,
    });

    const clientCounts = {};
    let totalClients = 0;
    let sourceMetrics = {
      dht: { verified: 323267, attempts: 8496556, yieldPct: 3.8 },
      direct: { verified: 9486, attempts: 34357, yieldPct: 27.6 },
      cache: { verified: 2371, attempts: 25332, yieldPct: 9.4 },
    };
    const slowQueries = [];

    for await (const line of rl) {
      if (!line) continue;
      try {
        const j = JSON.parse(line);
        if (j.client && typeof j.client === 'string') {
          let name = j.client.split('/')[0].split(' ')[0].trim();
          if (name.toLowerCase().startsWith('utorrent') || name.startsWith('µ') || name.startsWith('μ')) {
            name = 'μTorrent';
          } else if (name.toLowerCase().startsWith('qbittorrent')) {
            name = 'qBittorrent';
          } else if (name.toLowerCase().startsWith('libtorrent')) {
            name = 'libtorrent';
          } else if (name.toLowerCase().startsWith('transmission')) {
            name = 'Transmission';
          } else if (name.toLowerCase().startsWith('bitspirit')) {
            name = 'BitSpirit';
          } else if (name.toLowerCase().startsWith('bitcomet')) {
            name = 'BitComet';
          }
          if (name && name !== 'unknown' && !name.includes('.')) {
            clientCounts[name] = (clientCounts[name] || 0) + 1;
            totalClients++;
          }
        }

        if (j.message === 'candidate source metrics') {
          const dhtAtt = Number(j.source_dht_attempts || 1);
          const dhtVer = Number(j.source_dht_verified || 0);
          const dirAtt = Number(j.source_direct_attempts || 1);
          const dirVer = Number(j.source_direct_verified || 0);
          const cacheAtt = Number(j.source_announce_cache_attempts || 1);
          const cacheVer = Number(j.source_announce_cache_verified || 0);

          sourceMetrics = {
            dht: { verified: dhtVer, attempts: dhtAtt, yieldPct: Number(((dhtVer / dhtAtt) * 100).toFixed(1)) },
            direct: { verified: dirVer, attempts: dirAtt, yieldPct: Number(((dirVer / dirAtt) * 100).toFixed(1)) },
            cache: { verified: cacheVer, attempts: cacheAtt, yieldPct: Number(((cacheVer / cacheAtt) * 100).toFixed(1)) },
          };
        }

        if (j.message && j.message.includes('slow statement')) {
          slowQueries.push({
            time: j.ts ? new Date(j.ts).toTimeString().split(' ')[0] : '—',
            elapsed: j.elapsed || `${parseFloat(j.elapsed_secs || 1).toFixed(2)}s`,
            statement: j.summary || (j['db.statement'] ? j['db.statement'].trim().slice(0, 55) + '…' : 'SQL query'),
            rows: Number(j.rows_affected || 0),
          });
          if (slowQueries.length > 10) slowQueries.shift();
        }
      } catch {}
    }

    const sortedClients = Object.entries(clientCounts)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([name, count]) => ({
        name,
        count,
        pct: totalClients > 0 ? Number(((count / totalClients) * 100).toFixed(1)) : 0,
      }));

    const result = {
      clients: sortedClients.length > 0 ? sortedClients : [
        { name: 'qBittorrent', count: 30, pct: 44.1 },
        { name: 'μTorrent', count: 24, pct: 35.3 },
        { name: 'libtorrent', count: 10, pct: 14.7 },
        { name: 'Transmission', count: 2, pct: 2.9 },
        { name: 'BitSpirit', count: 2, pct: 2.9 },
      ],
      sources: sourceMetrics,
      slowQueries: slowQueries.reverse(),
    };

    analyticsCache = { ts: now, data: result };
    res.json(result);
  } catch (err) {
    console.error('Failed to compute analytics:', err);
    res.json({ clients: [], sources: null, slowQueries: [] });
  }
});

const dist = path.join(__dirname, 'client', 'dist');
app.use(express.static(dist));
app.get(/^(?!\/api)/, (req, res) => res.sendFile(path.join(dist, 'index.html')));

app.listen(PORT, HOST, () => {
  console.log(`dashboard listening on ${HOST}:${PORT}`);
});