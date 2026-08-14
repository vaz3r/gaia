import cors from "cors";
import express from "express";
import type { Pool } from "pg";

import type { AppConfig } from "./config/env.js";
import { AdminController } from "./controllers/admin.controller.js";
import { HealthController } from "./controllers/health.controller.js";
import { SearchController } from "./controllers/search.controller.js";
import { errorHandler } from "./middleware/error.js";
import { requestLogger } from "./middleware/logger.js";
import { adminRouter } from "./routes/admin.routes.js";
import { healthRouter } from "./routes/health.routes.js";
import { searchRouter } from "./routes/search.routes.js";
import { AdminService } from "./services/admin.service.js";
import { HealthService } from "./services/health.service.js";
import { SearchService } from "./services/search.service.js";

/** Assemble the Express app from config + shared pool. */
export function createApp(config: AppConfig, pool: Pool): express.Express {
  const app = express();

  app.use(cors());
  app.use(express.json({ limit: "1mb" }));
  app.use(requestLogger);

  const adminController = new AdminController(new AdminService(pool));
  const healthController = new HealthController(
    new HealthService(pool, config.REDIS_URL, config.CRAWLER_URL),
  );
  const searchController = new SearchController(new SearchService(pool));

  app.use("/api/admin", adminRouter(adminController));
  app.use("/api/search", searchRouter(searchController));
  app.use("/health", healthRouter(healthController));

  app.use(errorHandler);
  return app;
}
