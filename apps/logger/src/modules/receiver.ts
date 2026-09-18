import express, { Request, Response, NextFunction } from 'express';
import multer from 'multer';
import fs from 'fs-extra';
import path from 'path';
import crypto from 'crypto';
import type { Logger } from 'pino';
import type { LoggerConfig } from '../config.js';

function calculateChecksum(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('error', err => reject(err));
    stream.on('data', chunk => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

export function createReceiverApp(config: LoggerConfig, logger: Logger): express.Express {
  if (!config.apiKey) {
    logger.fatal('API_KEY or LOG_SHIPPER_API_KEY environment variable is required for receiver');
    process.exit(1);
  }

  const app = express();

  const authMiddleware = (req: Request, res: Response, next: NextFunction): void => {
    const authHeader = req.headers.authorization;
    if (!authHeader || authHeader !== `Bearer ${config.apiKey}`) {
      res.status(401).json({ error: 'Unauthorized' });
      return;
    }
    next();
  };

  const uploadDir = '/tmp/uploads';
  fs.ensureDirSync(uploadDir);
  const upload = multer({ dest: uploadDir, limits: { fileSize: config.maxFileSize } });

  app.get('/health', (_req: Request, res: Response) => {
    res.json({ status: 'ok', timestamp: new Date().toISOString() });
  });

  app.post('/logs', authMiddleware, upload.single('file'), async (req: Request, res: Response): Promise<void> => {
    const file = req.file;
    const { filename, host, checksum } = req.body as { filename?: string; host?: string; checksum?: string };

    if (!file || !filename || !host || !checksum) {
      if (file) await fs.remove(file.path);
      res.status(400).json({ error: 'Missing required fields' });
      return;
    }

    const hostDir = path.join(config.storageDir, host);
    const targetPath = path.join(hostDir, filename);

    try {
      await fs.ensureDir(hostDir);

      if (await fs.pathExists(targetPath)) {
        const existingChecksum = await calculateChecksum(targetPath);
        if (existingChecksum === checksum) {
          logger.info({ event: 'receiver_duplicate', filename, host }, 'Duplicate file detected, ignoring');
          await fs.remove(file.path);
          res.json({ status: 'duplicate', filename, checksum });
          return;
        }
      }

      const uploadedChecksum = await calculateChecksum(file.path);
      if (uploadedChecksum !== checksum) {
        await fs.remove(file.path);
        res.status(400).json({ error: 'Checksum mismatch' });
        return;
      }

      const tempTarget = targetPath + '.tmp';
      await fs.move(file.path, tempTarget, { overwrite: true });
      await fs.rename(tempTarget, targetPath);

      logger.info({ event: 'receiver_stored', filename, host, size: file.size }, 'File stored successfully');
      res.json({ status: 'stored', filename, checksum, size: file.size });
    } catch (error: any) {
      logger.error({ error: error.message }, 'Failed to store file');
      if (file && (await fs.pathExists(file.path))) {
        await fs.remove(file.path);
      }
      res.status(500).json({ error: 'Internal server error' });
    }
  });

  return app;
}
