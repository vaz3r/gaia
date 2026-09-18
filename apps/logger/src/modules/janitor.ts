import fs from 'fs-extra';
import path from 'path';
import type { Logger } from 'pino';
import type { LoggerConfig } from '../config.js';

export interface JanitorHandle {
  stop: () => void;
  sweep: () => Promise<void>;
}

export function parseLogFileTime(filename: string, stats: fs.Stats): number {
  // Filename pattern: crawler-2026-08-30T12-03-07Z.jsonl
  const match = filename.match(/crawler-(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})Z\.jsonl/);
  if (match) {
    const isoString = `${match[1]}T${match[2]}:${match[3]}:${match[4]}Z`;
    const parsed = Date.parse(isoString);
    if (!isNaN(parsed)) {
      return parsed;
    }
  }
  return stats.mtimeMs;
}

export async function runRetentionSweep(config: LoggerConfig, logger: Logger): Promise<{ deletedFiles: number; freedBytes: number; retainedFiles: number }> {
  const cutoffMs = Date.now() - config.logRetentionDays * 86400 * 1000;
  let deletedFiles = 0;
  let freedBytes = 0;
  let retainedFiles = 0;

  logger.info(
    { storageDir: config.storageDir, retentionDays: config.logRetentionDays, cutoff: new Date(cutoffMs).toISOString() },
    'Starting log retention sweep...'
  );

  async function walkAndPrune(dir: string): Promise<void> {
    if (!(await fs.pathExists(dir))) return;

    const entries = await fs.readdir(dir, { withFileTypes: true });

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);

      if (entry.isDirectory()) {
        await walkAndPrune(fullPath);
      } else if (entry.isFile() && entry.name.endsWith('.jsonl')) {
        try {
          const stats = await fs.stat(fullPath);
          const fileTime = parseLogFileTime(entry.name, stats);

          if (fileTime < cutoffMs) {
            await fs.remove(fullPath);
            deletedFiles++;
            freedBytes += stats.size;
            logger.debug({ file: entry.name, fileTime: new Date(fileTime).toISOString() }, 'Pruned expired log file');
          } else {
            retainedFiles++;
          }
        } catch (err: any) {
          logger.warn({ file: entry.name, error: err.message }, 'Failed to check or prune log file');
        }
      }
    }
  }

  try {
    await walkAndPrune(config.storageDir);

    if (deletedFiles > 0) {
      logger.info(
        {
          event: 'log_retention_sweep_completed',
          deletedFiles,
          freedMb: (freedBytes / (1024 * 1024)).toFixed(2),
          retainedFiles,
          retentionDays: config.logRetentionDays
        },
        'Log retention sweep completed: pruned expired logs'
      );
    } else {
      logger.info(
        { event: 'log_retention_sweep_completed', deletedFiles: 0, retainedFiles, retentionDays: config.logRetentionDays },
        'Log retention sweep completed: no expired files'
      );
    }
  } catch (error: any) {
    logger.error({ error: error.message }, 'Error during log retention sweep');
  }

  return { deletedFiles, freedBytes, retainedFiles };
}

export async function startJanitor(config: LoggerConfig, logger: Logger): Promise<JanitorHandle> {
  if (config.logRetentionDays <= 0) {
    logger.info('Log retention pruning is disabled (LOG_RETENTION_DAYS <= 0)');
    return {
      stop: () => {},
      sweep: async () => {}
    };
  }

  // Initial sweep on startup
  await runRetentionSweep(config, logger);

  // Periodic recurring sweep
  const timer = setInterval(() => {
    runRetentionSweep(config, logger).catch((err) => {
      logger.error({ error: err.message }, 'Unhandled error in scheduled retention sweep');
    });
  }, config.janitorIntervalMs);

  return {
    stop: () => {
      clearInterval(timer);
      logger.info('Log retention janitor stopped');
    },
    sweep: () => runRetentionSweep(config, logger).then(() => {})
  };
}
