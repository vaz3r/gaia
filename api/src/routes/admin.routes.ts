import { Router } from "express";

import type { AdminController } from "../controllers/admin.controller.js";
import { asyncHandler } from "../middleware/asyncHandler.js";

export function adminRouter(admin: AdminController): Router {
  const router = Router();

  router.get("/monitor/latest", asyncHandler(admin.latest));
  router.get("/monitor/history", asyncHandler(admin.history));
  router.get("/monitor/rates", asyncHandler(admin.rateHistory));
  router.get("/monitor/failures", asyncHandler(admin.failures));
  router.get("/monitor/system", asyncHandler(admin.system));

  router.get("/config", asyncHandler(admin.listConfig));
  router.get("/config/:key", asyncHandler(admin.getConfig));
  router.put("/config/:key", asyncHandler(admin.setConfig));
  router.delete("/config/:key", asyncHandler(admin.deleteConfig));

  return router;
}
