import type { Request, Response } from "express";
import { z } from "zod";

import type { SearchService } from "../services/search.service.js";

const sortSchema = z.enum(["relevance", "newest", "largest", "name"]);
const orderSchema = z.enum(["asc", "desc"]).default("desc");

export class SearchController {
  constructor(private readonly svc: SearchService) {}

  search = async (req: Request, res: Response): Promise<void> => {
    const q = z.string().min(1).max(200).safeParse(req.query.q);
    if (!q.success) {
      res.status(400).json({ error: "missing or invalid 'q' query parameter" });
      return;
    }
    const sizeMin = parseOptionalInt(req.query.size_min);
    const sizeMax = parseOptionalInt(req.query.size_max);
    const fileMin = parseOptionalInt(req.query.file_min);
    const ageMin = parseOptionalInt(req.query.age_min);
    const sort = sortSchema.safeParse(req.query.sort ?? "relevance");
    const order = orderSchema.safeParse(req.query.order);
    const from = parseOptionalInt(req.query.from) ?? 0;
    const limit = parseOptionalInt(req.query.limit) ?? 50;

    if (!sort.success) {
      res.status(400).json({ error: "invalid sort", details: sort.error.message });
      return;
    }
    if (!order.success) {
      res.status(400).json({ error: "invalid order", details: order.error.message });
      return;
    }

    const result = await this.svc.search({
      q: q.data,
      ...(sizeMin !== undefined ? { sizeMin } : {}),
      ...(sizeMax !== undefined ? { sizeMax } : {}),
      ...(fileMin !== undefined ? { fileMin } : {}),
      ...(ageMin !== undefined ? { ageMin } : {}),
      sort: sort.data,
      order: order.data,
      from,
      limit: Math.min(limit, 200),
    });
    res.json(result);
  };
}

function parseOptionalInt(v: unknown): number | undefined {
  if (v === undefined || v === null || v === "") {
    return undefined;
  }
  const n = Number(v);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : undefined;
}
