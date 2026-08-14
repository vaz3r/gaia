import { Router } from "express";

import type { HealthController } from "../controllers/health.controller.js";
import { asyncHandler } from "../middleware/asyncHandler.js";

export function healthRouter(health: HealthController): Router {
  const router = Router();
  router.get("/", asyncHandler(health.check));
  return router;
}
