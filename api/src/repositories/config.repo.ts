import type pg from "pg";

import type { ConfigEntry } from "../types/api.js";

/** Pure SQL access to the `app_config` key/value store (JSONB values). */
export class ConfigRepository {
  constructor(private readonly pool: pg.Pool) {}

  async list(): Promise<ConfigEntry[]> {
    const { rows } = await this.pool.query<ConfigEntry>(
      "SELECT key, value, updated_at FROM app_config ORDER BY key",
    );
    return rows;
  }

  async get(key: string): Promise<ConfigEntry | null> {
    const { rows } = await this.pool.query<ConfigEntry>(
      "SELECT key, value, updated_at FROM app_config WHERE key = $1",
      [key],
    );
    return rows[0] ?? null;
  }

  async upsert(key: string, value: unknown): Promise<ConfigEntry> {
    const { rows } = await this.pool.query<ConfigEntry>(
      `INSERT INTO app_config (key, value, updated_at)
       VALUES ($1, $2::jsonb, now())
       ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()
       RETURNING key, value, updated_at`,
      [key, JSON.stringify(value)],
    );
    const row = rows[0];
    if (!row) {
      throw new Error("upsert returned no row");
    }
    return row;
  }

  async remove(key: string): Promise<boolean> {
    const result = await this.pool.query("DELETE FROM app_config WHERE key = $1", [key]);
    return (result.rowCount ?? 0) > 0;
  }
}
