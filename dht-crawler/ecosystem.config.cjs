/**
 * PM2 configuration for dht-crawler.
 *
 * Start:     pm2 start ecosystem.config.cjs
 * Restart:   pm2 restart dht-crawler
 * Logs:      pm2 logs dht-crawler
 * Stop:      pm2 stop dht-crawler
 * Auto-start on boot: pm2 save && pm2 startup
 *
 * The binary handles SIGTERM gracefully (drains in-flight fetches, flushes
 * the DB, persists routing tables), so PM2's default SIGTERM shutdown works;
 * kill_timeout is raised to cover the ~15s drain.
 *
 * To run multiple instances sharing one DB (multiplies discovery breadth),
 * uncomment the instances/port line or pass `--instances N` to args.
 */
module.exports = {
  apps: [
    {
      name: "dht-crawler",
      cwd: __dirname,
      script: "../target/release/dht-crawler",
      args: "run --db crawler.sqlite --state-dir state --port 6881 --instances 4 --log dht_crawler=info",
      // The crawler is a single long-running process; do not fork-mode cluster.
      instances: 1,
      exec_mode: "fork",
      autorestart: true,
      max_restarts: 10,
      min_uptime: "10s",
      // SIGTERM triggers graceful shutdown; allow up to 30s for the drain.
      kill_timeout: 30000,
      stop_exit_codes: [0],
      exp_backoff_restart_delay: 100,
      out_file: "./pm2-out.log",
      error_file: "./pm2-error.log",
      merge_logs: true,
      time: true,
      env: {
        NODE_ENV: "production",
      },
    },
  ],
};
