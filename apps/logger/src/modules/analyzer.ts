import express, { Request, Response } from 'express';
import cors from 'cors';
import { Database } from 'duckdb-async';
import path from 'path';
import type { Logger } from 'pino';
import type { LoggerConfig } from '../config.js';

export interface AnalyzerInstance {
  app: express.Express;
  close: () => Promise<void>;
}

export async function createAnalyzerApp(config: LoggerConfig, logger: Logger): Promise<AnalyzerInstance> {
  const app = express();
  app.use(cors());
  app.use(express.json());
  app.set('json replacer', (_key: string, value: any) => (typeof value === 'bigint' ? Number(value) : value));

  const targetGlob = path.join(config.logsDir, '**/*.jsonl');

  logger.info({ logsDir: config.logsDir, targetGlob }, 'Initializing DuckDB in memory...');
  const db = await Database.create(':memory:');
  logger.info('DuckDB initialized in memory.');

  app.get('/health', (_req: Request, res: Response) => {
    res.json({ status: 'ok', engine: 'duckdb', timestamp: new Date().toISOString() });
  });

  // Endpoint 1: Routing Table Health Over Time
  app.get('/api/metrics/routing-table', async (_req: Request, res: Response): Promise<void> => {
    try {
      const query = `
        SELECT 
          (json->>'ts')::TIMESTAMP AS time, 
          (json->>'routing_table')::BIGINT AS routing_table 
        FROM read_json_objects('${targetGlob}')
        WHERE json->>'routing_table' IS NOT NULL
        ORDER BY time DESC
        LIMIT 100
      `;
      const rows = await db.all(query);
      res.json(rows);
    } catch (error: any) {
      logger.error({ error: error.message }, 'Failed routing table query');
      res.status(500).json({ error: error.message });
    }
  });

  // Endpoint 2: Discovery Source Verification Success
  app.get('/api/metrics/discovery-sources', async (_req: Request, res: Response): Promise<void> => {
    try {
      const query = `
        SELECT 
          (json->>'ts')::TIMESTAMP AS time,
          (json->>'source_dht_verified')::BIGINT AS source_dht_verified,
          (json->>'source_direct_verified')::BIGINT AS source_direct_verified,
          (json->>'source_announce_cache_verified')::BIGINT AS source_announce_cache_verified
        FROM read_json_objects('${targetGlob}')
        WHERE json->>'message' = 'candidate source metrics'
        ORDER BY time DESC
        LIMIT 100
      `;
      const rows = await db.all(query);
      res.json(rows);
    } catch (error: any) {
      logger.error({ error: error.message }, 'Failed discovery sources query');
      res.status(500).json({ error: error.message });
    }
  });

  // Endpoint 3: Slow SQL Queries Analysis
  app.get('/api/metrics/slow-queries', async (_req: Request, res: Response): Promise<void> => {
    try {
      const query = `
        SELECT 
          (json->>'ts')::TIMESTAMP AS time,
          (json->>'elapsed_secs')::DOUBLE AS duration_secs,
          (json->>'db.statement') AS query_statement,
          (json->>'rows_affected')::BIGINT AS rows_affected
        FROM read_json_objects('${targetGlob}')
        WHERE json->>'message' = 'slow statement: execution time exceeded alert threshold'
        ORDER BY duration_secs DESC
        LIMIT 50
      `;
      const rows = await db.all(query);
      res.json(rows);
    } catch (error: any) {
      logger.error({ error: error.message }, 'Failed slow queries query');
      res.status(500).json({ error: error.message });
    }
  });

  // Endpoint 4: Custom SQL Query (For direct dashboard / anomaly analytics)
  app.post('/api/query', async (req: Request, res: Response): Promise<void> => {
    try {
      const { sql } = req.body;
      if (!sql) {
        res.status(400).json({ error: 'Missing sql parameter' });
        return;
      }

      // Inject the glob path dynamically for safety so the frontend just says FROM logs
      const safeSql = sql.replace(/FROM logs/gi, `FROM read_json_objects('${targetGlob}')`);

      const rows = await db.all(safeSql);
      res.json(rows);
    } catch (error: any) {
      logger.error({ error: error.message, sql: req.body?.sql }, 'Failed custom query');
      res.status(500).json({ error: error.message });
    }
  });

  return {
    app,
    close: async () => {
      try {
        await db.close();
        logger.info('DuckDB closed');
      } catch (err: any) {
        logger.warn({ error: err.message }, 'Error closing DuckDB');
      }
    }
  };
}
