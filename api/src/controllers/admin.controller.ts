import type { Request, Response } from "express";
import { z } from "zod";

import type { AdminService } from "../services/admin.service.js";
import { InvalidRangeError } from "../services/admin.service.js";

const rangeSchema = z.enum(["5m", "30m", "1h", "6h", "24h", "7d"]);
const metricSchema = z
  .string()
  .regex(/^[a-z_]+$/, "metric must be a crawl_stats_history column name");
const systemKindSchema = z.enum(["network", "memory", "cpu", "disk", "loadavg"]);
const configValueSchema = z.unknown();

export class AdminController {
  constructor(private readonly admin: AdminService) {}

  latest = async (_req: Request, res: Response): Promise<void> => {
    const row = await this.admin.latest();
    res.json(row ?? { ts: null });
  };

  history = async (req: Request, res: Response): Promise<void> => {
    const metric = metricSchema.safeParse(req.query.metric);
    const range = rangeSchema.safeParse(req.query.range);
    if (!metric.success) {
      res.status(400).json({ error: "invalid metric", details: metric.error.message });
      return;
    }
    if (!range.success) {
      res.status(400).json({ error: "invalid range", details: range.error.message });
      return;
    }
    const rows = await this.admin.history(metric.data, range.data);
    res.json({ metric: metric.data, range: range.data, data: rows });
  };

  rateHistory = async (req: Request, res: Response): Promise<void> => {
    const metric = metricSchema.safeParse(req.query.metric);
    const range = rangeSchema.safeParse(req.query.range);
    if (!metric.success) {
      res.status(400).json({ error: "invalid metric", details: metric.error.message });
      return;
    }
    if (!range.success) {
      res.status(400).json({ error: "invalid range", details: range.error.message });
      return;
    }
    const rows = await this.admin.rateHistory(metric.data, range.data);
    res.json({ metric: metric.data, range: range.data, data: rows });
  };

  failures = async (req: Request, res: Response): Promise<void> => {
    const range = rangeSchema.safeParse(req.query.range);
    if (!range.success) {
      res.status(400).json({ error: "invalid range", details: range.error.message });
      return;
    }
    const rows = await this.admin.failures(range.data);
    res.json({ range: range.data, data: rows });
  };

  system = async (req: Request, res: Response): Promise<void> => {
    const kind = systemKindSchema.safeParse(req.query.kind);
    const range = rangeSchema.safeParse(req.query.range);
    if (!kind.success) {
      res.status(400).json({ error: "invalid system kind", details: kind.error.message });
      return;
    }
    if (!range.success) {
      res.status(400).json({ error: "invalid range", details: range.error.message });
      return;
    }
    try {
      const rows = await this.admin.system(kind.data, range.data);
      res.json({ kind: kind.data, range: range.data, data: rows });
    } catch (err) {
      if (err instanceof InvalidRangeError) {
        res.status(400).json({ error: err.message });
        return;
      }
      throw err;
    }
  };

  listConfig = async (_req: Request, res: Response): Promise<void> => {
    const rows = await this.admin.listConfig();
    res.json({ data: rows });
  };

  getConfig = async (req: Request, res: Response): Promise<void> => {
    const key = req.params.key;
    if (!key) {
      res.status(400).json({ error: "missing config key" });
      return;
    }
    const row = await this.admin.getConfig(key);
    if (!row) {
      res.status(404).json({ error: `config key '${key}' not found` });
      return;
    }
    res.json(row);
  };

  setConfig = async (req: Request, res: Response): Promise<void> => {
    const key = req.params.key;
    if (!key) {
      res.status(400).json({ error: "missing config key" });
      return;
    }
    const value = configValueSchema.safeParse(req.body);
    if (!value.success) {
      res.status(400).json({ error: "invalid config value", details: value.error.message });
      return;
    }
    const row = await this.admin.setConfig(key, value.data);
    res.json(row);
  };

  deleteConfig = async (req: Request, res: Response): Promise<void> => {
    const key = req.params.key;
    if (!key) {
      res.status(400).json({ error: "missing config key" });
      return;
    }
    const removed = await this.admin.deleteConfig(key);
    if (!removed) {
      res.status(404).json({ error: `config key '${key}' not found` });
      return;
    }
    res.status(204).end();
  };
}
