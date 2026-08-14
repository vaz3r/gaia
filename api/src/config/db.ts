import pg from "pg";

import type { AppConfig } from "./env.js";

const { Pool } = pg;

/**
 * Singleton Postgres pool. Pure SQL throughout — no ORM. Query timeouts are
 * enforced at the DB level (statement_timeout) and via AbortSignal here for
 * extra safety on long admin queries.
 */
export function createPool(config: AppConfig): pg.Pool {
  return new Pool({
    connectionString: config.DATABASE_URL,
    max: 10,
    idleTimeoutMillis: 30_000,
    connectionTimeoutMillis: 5_000,
  });
}

export type { pg };
