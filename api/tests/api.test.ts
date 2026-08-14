import { afterAll, beforeAll, describe, expect, it } from "vitest";
import request from "supertest";
import { createPool } from "../src/config/db.js";
import { loadConfig } from "../src/config/env.js";
import { createApp } from "../src/app.js";
import type { Express } from "express";

let app: Express;
let pool: ReturnType<typeof createPool>;

const testConfig = loadConfig({
  DATABASE_URL:
    process.env.TEST_DATABASE_URL ??
    "postgres://crawler:crawler@localhost:5432/crawler_test",
  REDIS_URL: process.env.TEST_REDIS_URL ?? "redis://localhost:6379",
});

beforeAll(async () => {
  pool = createPool(testConfig);
  // Ensure the test DB has the schema + a seed torrent for search.
  const seedHash = "aa".repeat(20);
  await pool.query(
    `INSERT INTO torrents (info_hash, name, size_bytes, file_count, first_seen, last_seen)
     VALUES (decode($1, 'hex'), 'The Matrix 1999', 1073741824, 2, 1700000000, 1700000001)
     ON CONFLICT (info_hash) DO NOTHING`,
    [seedHash],
  );
  app = createApp(testConfig, pool);
});

afterAll(async () => {
  await pool.query(
    `DELETE FROM torrents WHERE encode(info_hash, 'hex') = $1`,
    ["aa".repeat(20)],
  );
  await pool.query(`DELETE FROM app_config WHERE key LIKE 'test:%'`);
  await pool.end();
});

describe("health", () => {
  it("returns healthy status shape", async () => {
    const res = await request(app).get("/health");
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      postgres: expect.stringMatching(/healthy|unhealthy/),
      redis: expect.stringMatching(/healthy|unhealthy|unknown/),
      api: "healthy",
    });
  });
});

describe("admin monitor", () => {
  it("latest returns a snapshot or null", async () => {
    const res = await request(app).get("/api/admin/monitor/latest");
    expect(res.status).toBe(200);
    expect(res.body).toBeDefined();
  });

  it("rejects invalid range with 400", async () => {
    const res = await request(app).get("/api/admin/monitor/history?metric=ts&range=99m");
    expect(res.status).toBe(400);
    expect(res.body.error).toBeDefined();
  });

  it("history returns rows for a valid metric", async () => {
    const res = await request(app).get(
      "/api/admin/monitor/history?metric=hashes_sampled&range=24h",
    );
    expect(res.status).toBe(200);
    expect(res.body.metric).toBe("hashes_sampled");
    expect(Array.isArray(res.body.data)).toBe(true);
  });

  it("failures returns breakdown", async () => {
    const res = await request(app).get("/api/admin/monitor/failures?range=24h");
    expect(res.status).toBe(200);
    expect(Array.isArray(res.body.data)).toBe(true);
  });

  it("system rejects unknown kind with 400", async () => {
    const res = await request(app).get("/api/admin/monitor/system?kind=bogus&range=1h");
    expect(res.status).toBe(400);
  });
});

describe("admin config", () => {
  it("upserts and reads a config key", async () => {
    const put = await request(app)
      .put("/api/admin/config/test:greeting")
      .send({ hello: "world" });
    expect(put.status).toBe(200);
    expect(put.body.value).toEqual({ hello: "world" });

    const get = await request(app).get("/api/admin/config/test:greeting");
    expect(get.status).toBe(200);
    expect(get.body.value).toEqual({ hello: "world" });
  });

  it("lists config keys", async () => {
    const res = await request(app).get("/api/admin/config");
    expect(res.status).toBe(200);
    expect(Array.isArray(res.body.data)).toBe(true);
  });

  it("deletes a config key", async () => {
    const del = await request(app).delete("/api/admin/config/test:greeting");
    expect(del.status).toBe(204);
    const get = await request(app).get("/api/admin/config/test:greeting");
    expect(get.status).toBe(404);
  });
});

describe("search", () => {
  it("finds a fuzzy match by relevance", async () => {
    const res = await request(app).get("/api/search?q=matris");
    expect(res.status).toBe(200);
    expect(res.body.total).toBeGreaterThan(0);
    expect(res.body.data[0].name).toContain("Matrix");
    expect(typeof res.body.data[0].similarity).toBe("number");
  });

  it("applies size filter", async () => {
    const res = await request(app).get(
      "/api/search?q=matrix&size_min=1000000000&size_max=2000000000",
    );
    expect(res.status).toBe(200);
    expect(res.body.total).toBeGreaterThan(0);
  });

  it("rejects missing q with 400", async () => {
    const res = await request(app).get("/api/search");
    expect(res.status).toBe(400);
  });

  it("rejects invalid sort with 400", async () => {
    const res = await request(app).get("/api/search?q=matrix&sort=bogus");
    expect(res.status).toBe(400);
  });

  it("sorts by newest", async () => {
    const res = await request(app).get("/api/search?q=matrix&sort=newest&order=desc");
    expect(res.status).toBe(200);
    expect(res.body.data.length).toBeGreaterThan(0);
  });
});
