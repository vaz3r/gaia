import type { Pool } from "pg";

import { ConfigRepository } from "../repositories/config.repo.js";
import { StatsRepository } from "../repositories/stats.repo.js";
import { SystemRepository } from "../repositories/system.repo.js";
import type { ConfigEntry, CrawlSnapshotRow } from "../types/api.js";

/** Valid monitoring time ranges, in seconds. */
export const RANGES: Record<string, number> = {
  "5m": 300,
  "30m": 1800,
  "1h": 3600,
  "6h": 21600,
  "24h": 86400,
  "7d": 604800,
};

export class InvalidRangeError extends Error {}

export class AdminService {
  private readonly statsRepo: StatsRepository;
  private readonly configRepo: ConfigRepository;
  private readonly systemRepo: SystemRepository;

  constructor(pool: Pool) {
    this.statsRepo = new StatsRepository(pool);
    this.configRepo = new ConfigRepository(pool);
    this.systemRepo = new SystemRepository(pool);
  }

  latest(): Promise<CrawlSnapshotRow | null> {
    return this.statsRepo.latest();
  }

  async history(metric: string, range: string): Promise<{ ts: string; value: number | null }[]> {
    const secs = resolveRange(range);
    return this.statsRepo.history(metric, secs);
  }

  async rateHistory(metric: string, range: string): Promise<{ ts: string; value: number | null }[]> {
    const secs = resolveRange(range);
    return this.statsRepo.rateHistory(metric, secs);
  }

  async failures(range: string): Promise<{ reason: string; count: string }[]> {
    const secs = resolveRange(range);
    return this.statsRepo.failures(secs);
  }

  async system(kind: string, range: string): Promise<unknown[]> {
    const secs = resolveRange(range);
    switch (kind) {
      case "network":
        return this.systemRepo.network(secs);
      case "memory":
        return this.systemRepo.memory(secs);
      case "cpu":
        return this.systemRepo.cpu(secs);
      case "disk":
        return this.systemRepo.disk(secs);
      case "loadavg":
        return this.systemRepo.loadavg(secs);
      default:
        throw new InvalidRangeError(`unknown system metric kind: ${kind}`);
    }
  }

  listConfig(): Promise<ConfigEntry[]> {
    return this.configRepo.list();
  }

  getConfig(key: string): Promise<ConfigEntry | null> {
    return this.configRepo.get(key);
  }

  setConfig(key: string, value: unknown): Promise<ConfigEntry> {
    return this.configRepo.upsert(key, value);
  }

  async deleteConfig(key: string): Promise<boolean> {
    return this.configRepo.remove(key);
  }
}

export function resolveRange(range: string): number {
  const secs = RANGES[range];
  if (secs === undefined) {
    throw new InvalidRangeError(
      `unsupported range '${range}'; expected one of ${Object.keys(RANGES).join(", ")}`,
    );
  }
  return secs;
}
