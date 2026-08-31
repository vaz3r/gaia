import express from 'express';
import cors from 'cors';
import { Database } from 'duckdb-async';
import dotenv from 'dotenv';
import path from 'path';

dotenv.config();

const app = express();
app.use(cors());
app.use(express.json());
app.set('json replacer', (key: string, value: any) => typeof value === 'bigint' ? Number(value) : value);

const PORT = process.env.PORT || 3001;
const LOGS_DIR = process.env.LOGS_DIR || '/logs';
const TARGET_GLOB = path.join(LOGS_DIR, '**/*.jsonl');

let db: Database;

async function initDb() {
  db = await Database.create(':memory:');
  console.log('DuckDB initialized in memory.');
}

// Endpoint 1: Routing Table Health Over Time
app.get('/api/metrics/routing-table', async (req, res) => {
  try {
    const query = `
      SELECT 
        (json->>'ts')::TIMESTAMP AS time, 
        (json->>'routing_table')::BIGINT AS routing_table 
      FROM read_json_objects('${TARGET_GLOB}')
      WHERE json->>'routing_table' IS NOT NULL
      ORDER BY time DESC
      LIMIT 100
    `;
    const rows = await db.all(query);
    res.json(rows);
  } catch (error: any) {
    console.error(error);
    res.status(500).json({ error: error.message });
  }
});

// Endpoint 2: Discovery Source Verification Success
app.get('/api/metrics/discovery-sources', async (req, res) => {
  try {
    const query = `
      SELECT 
        (json->>'ts')::TIMESTAMP AS time,
        (json->>'source_dht_verified')::BIGINT AS source_dht_verified,
        (json->>'source_direct_verified')::BIGINT AS source_direct_verified,
        (json->>'source_announce_cache_verified')::BIGINT AS source_announce_cache_verified
      FROM read_json_objects('${TARGET_GLOB}')
      WHERE json->>'message' = 'candidate source metrics'
      ORDER BY time DESC
      LIMIT 100
    `;
    const rows = await db.all(query);
    res.json(rows);
  } catch (error: any) {
    console.error(error);
    res.status(500).json({ error: error.message });
  }
});

// Endpoint 3: Slow SQL Queries Analysis
app.get('/api/metrics/slow-queries', async (req, res) => {
  try {
    const query = `
      SELECT 
        (json->>'ts')::TIMESTAMP AS time,
        (json->>'elapsed_secs')::DOUBLE AS duration_secs,
        (json->>'db.statement') AS query_statement,
        (json->>'rows_affected')::BIGINT AS rows_affected
      FROM read_json_objects('${TARGET_GLOB}')
      WHERE json->>'message' = 'slow statement: execution time exceeded alert threshold'
      ORDER BY duration_secs DESC
      LIMIT 50
    `;
    const rows = await db.all(query);
    res.json(rows);
  } catch (error: any) {
    console.error(error);
    res.status(500).json({ error: error.message });
  }
});

// Endpoint 4: Custom SQL Query (For direct dashboard analytics)
app.post('/api/query', async (req, res) => {
  try {
    const { sql } = req.body;
    if (!sql) {
      return res.status(400).json({ error: 'Missing sql parameter' });
    }
    
    // Inject the glob path dynamically for safety so the frontend just says FROM logs
    const safeSql = sql.replace(/FROM logs/gi, `FROM read_json_objects('${TARGET_GLOB}')`);
    
    const rows = await db.all(safeSql);
    res.json(rows);
  } catch (error: any) {
    console.error(error);
    res.status(500).json({ error: error.message });
  }
});

initDb().then(() => {
  app.listen(PORT, () => {
    console.log(`Log Analyzer API running on port ${PORT}`);
    console.log(`Targeting logs at: ${TARGET_GLOB}`);
  });
}).catch(console.error);
