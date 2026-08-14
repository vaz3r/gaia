import { Router } from "express";

import type { SearchController } from "../controllers/search.controller.js";
import { asyncHandler } from "../middleware/asyncHandler.js";

export function searchRouter(search: SearchController): Router {
  const router = Router();
  router.get("/", asyncHandler(search.search));
  return router;
}
