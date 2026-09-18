import fs from 'fs-extra';
import path from 'path';
import axios from 'axios';
import FormData from 'form-data';
import crypto from 'crypto';
import type { Logger } from 'pino';
import type { LoggerConfig } from '../config.js';

interface ProcessedFile {
  checksum: string;
  shippedAt: string;
}

function calculateChecksum(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('error', err => reject(err));
    stream.on('data', chunk => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

export async function startShipper(config: LoggerConfig, logger: Logger): Promise<{ stop: () => void }> {
  if (!config.apiKey) {
    logger.fatal('API_KEY or LOG_SHIPPER_API_KEY environment variable is required for shipper profile');
    process.exit(1);
  }

  const manifestPath = path.join(config.sourceDir, 'processed.json');
  let processedManifest: Record<string, ProcessedFile> = {};

  async function loadManifest(): Promise<void> {
    try {
      if (await fs.pathExists(manifestPath)) {
        processedManifest = await fs.readJson(manifestPath);
      }
    } catch (error: any) {
      logger.warn({ error: error.message }, 'Could not load manifest, starting fresh');
    }
  }

  async function saveManifest(): Promise<void> {
    try {
      await fs.writeJson(manifestPath, processedManifest);
    } catch (error: any) {
      logger.error({ error: error.message }, 'Failed to save manifest');
    }
  }

  async function scanAndShip(): Promise<void> {
    logger.info({ sourceDir: config.sourceDir }, 'Starting scan for crawler logs...');
    try {
      await fs.ensureDir(config.sourceDir);

      const files = await fs.readdir(config.sourceDir);
      const now = Date.now();

      for (const file of files) {
        if (!file.startsWith('crawler-') || !file.endsWith('.jsonl')) continue;

        const filePath = path.join(config.sourceDir, file);
        const stats = await fs.stat(filePath);

        if (now - stats.mtimeMs < config.fileMinAgeMs) {
          continue;
        }

        if (processedManifest[file]) {
          logger.debug({ file }, 'File already processed, skipping');
          try {
            await fs.remove(filePath);
            logger.info({ file }, 'Cleaned up leftover processed file');
          } catch (e) {}
          continue;
        }

        const checksum = await calculateChecksum(filePath);
        logger.info({ file, size: stats.size }, 'Shipping file');

        const formData = new FormData();
        formData.append('filename', file);
        formData.append('host', config.hostName);
        formData.append('checksum', checksum);
        formData.append('file', fs.createReadStream(filePath));

        try {
          const response = await axios.post(`${config.receiverUrl}/logs`, formData, {
            headers: {
              ...formData.getHeaders(),
              Authorization: `Bearer ${config.apiKey}`
            },
            maxContentLength: Infinity,
            maxBodyLength: Infinity
          });

          if (response.status === 200) {
            logger.info({ event: 'file_shipped', file }, 'File successfully shipped');
            processedManifest[file] = { checksum, shippedAt: new Date().toISOString() };
            await saveManifest();

            await fs.remove(filePath);
            logger.info({ file }, 'Deleted file from source directory');
          }
        } catch (error: any) {
          if (
            error.response &&
            error.response.status === 200 &&
            error.response.data &&
            error.response.data.status === 'duplicate'
          ) {
            logger.info({ event: 'file_duplicate', file }, 'File was a duplicate on receiver');
            processedManifest[file] = { checksum, shippedAt: new Date().toISOString() };
            await saveManifest();
            await fs.remove(filePath);
          } else {
            logger.error({ event: 'file_failed', file, error: error.message }, 'Failed to ship file');
          }
        }
      }
    } catch (error: any) {
      logger.error({ error: error.message }, 'Error during scan loop');
    }
  }

  await loadManifest();
  await scanAndShip();
  const timer = setInterval(scanAndShip, config.scanIntervalMs);

  return {
    stop: () => {
      clearInterval(timer);
      logger.info('Shipper stopped');
    }
  };
}
