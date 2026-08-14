import { createPool } from "./config/db.js";
import { loadConfig } from "./config/env.js";
import { createApp } from "./app.js";

const config = loadConfig();
const pool = createPool(config);

const app = createApp(config, pool);
const port = config.PORT;

app.listen(port, () => {
  // eslint-disable-next-line no-console
  console.log(`gaia-api listening on :${port}`);
});

const shutdown = (signal: string): void => {
  // eslint-disable-next-line no-console
  console.log(`${signal} received, shutting down`);
  pool.end().finally(() => process.exit(0));
};

process.on("SIGINT", () => shutdown("SIGINT"));
process.on("SIGTERM", () => shutdown("SIGTERM"));
