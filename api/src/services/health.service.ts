import { createClient } from "redis";
import { createConnection } from "node:net";
import type { Pool } from "pg";

import type { HealthStatus } from "../types/api.js";

/**
 * Health checks for the platform services. The API itself is healthy by
 * definition when this responds. Crawler health is best-effort via a TCP
 * probe when CRAWLER_URL is configured; otherwise "unknown".
 */
export class HealthService {
  constructor(
    private readonly pool: Pool,
    private readonly redisUrl: string,
    private readonly crawlerUrl?: string,
  ) {}

  async check(): Promise<HealthStatus> {
    const [postgres, redis, crawler] = await Promise.all([
      this.checkPostgres(),
      this.checkRedis(),
      this.checkCrawler(),
    ]);
    return {
      postgres,
      redis,
      crawler,
      api: "healthy",
    };
  }

  private async checkPostgres(): Promise<"healthy" | "unhealthy"> {
    try {
      await this.pool.query("SELECT 1");
      return "healthy";
    } catch {
      return "unhealthy";
    }
  }

  private async checkRedis(): Promise<"healthy" | "unhealthy"> {
    const client = createClient({
      url: this.redisUrl,
      socket: { connectTimeout: 2000, reconnectStrategy: () => 100 },
    });
    try {
      await Promise.race([
        client.connect(),
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error("redis connect timeout")), 2500),
        ),
      ]);
      await client.ping();
      return "healthy";
    } catch {
      return "unhealthy";
    } finally {
      await client.quit().catch(() => {});
    }
  }

  private async checkCrawler(): Promise<"healthy" | "unhealthy" | "unknown"> {
    if (!this.crawlerUrl) {
      return "unknown";
    }
    try {
      const url = new URL(this.crawlerUrl);
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 2000);
      const conn = await connectTcp(url.hostname, Number(url.port || 6881), controller.signal);
      clearTimeout(timer);
      conn.destroy();
      return "healthy";
    } catch {
      return "unhealthy";
    }
  }
}

function connectTcp(
  host: string,
  port: number,
  signal: AbortSignal,
): Promise<ReturnType<typeof createConnection>> {
  return new Promise((resolve, reject) => {
    const socket = createConnection({ host, port });
    const onAbort = () => {
      socket.destroy();
      reject(new Error("aborted"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
    socket.once("connect", () => {
      signal.removeEventListener("abort", onAbort);
      resolve(socket);
    });
    socket.once("error", (err) => {
      signal.removeEventListener("abort", onAbort);
      reject(err);
    });
  });
}
