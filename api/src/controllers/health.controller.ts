import type { Request, Response } from "express";

import type { HealthService } from "../services/health.service.js";

export class HealthController {
  constructor(private readonly health: HealthService) {}

  check = async (_req: Request, res: Response): Promise<void> => {
    const status = await this.health.check();
    res.json(status);
  };
}
