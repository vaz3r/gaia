import express, { Request, Response, NextFunction } from 'express';
import multer from 'multer';
import fs from 'fs-extra';
import path from 'path';
import crypto from 'crypto';
import pino from 'pino';

const logger = pino({ level: process.env.LOG_LEVEL || 'info' });

const API_KEY = process.env.API_KEY;
const STORAGE_DIR = process.env.STORAGE_DIR || '/logs';
const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;
const MAX_FILE_SIZE = process.env.MAX_FILE_SIZE ? parseInt(process.env.MAX_FILE_SIZE, 10) : 104857600;

if (!API_KEY) {
  logger.fatal('API_KEY environment variable is required');
  process.exit(1);
}

const app = express();

const authMiddleware = (req: Request, res: Response, next: NextFunction) => {
  const authHeader = req.headers.authorization;
  if (!authHeader || authHeader !== `Bearer ${API_KEY}`) {
    res.status(401).json({ error: 'Unauthorized' });
    return;
  }
  next();
};

const upload = multer({ dest: '/tmp/uploads', limits: { fileSize: MAX_FILE_SIZE } });

app.get('/health', (req: Request, res: Response) => {
  res.json({ status: 'ok', timestamp: new Date().toISOString() });
});

app.post('/logs', authMiddleware, upload.single('file'), async (req: Request, res: Response): Promise<void> => {
  const file = req.file;
  const { filename, host, checksum } = req.body as { filename?: string, host?: string, checksum?: string };

  if (!file || !filename || !host || !checksum) {
    if (file) await fs.remove(file.path);
    res.status(400).json({ error: 'Missing required fields' });
    return;
  }

  const hostDir = path.join(STORAGE_DIR, host);
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
    if (file && await fs.pathExists(file.path)) {
      await fs.remove(file.path);
    }
    res.status(500).json({ error: 'Internal server error' });
  }
});

function calculateChecksum(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('error', err => reject(err));
    stream.on('data', chunk => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

app.listen(PORT, '0.0.0.0', () => {
  logger.info(`Log Receiver listening on port ${PORT}`);
});
