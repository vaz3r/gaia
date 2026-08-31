const express = require('express');
const multer = require('multer');
const fs = require('fs-extra');
const path = require('path');
const crypto = require('crypto');
const pino = require('pino');

const logger = pino({ level: process.env.LOG_LEVEL || 'info' });

const API_KEY = process.env.API_KEY;
const STORAGE_DIR = process.env.STORAGE_DIR || '/logs';
const PORT = process.env.PORT || 3000;
const MAX_FILE_SIZE = process.env.MAX_FILE_SIZE || 104857600; // 100MB default

if (!API_KEY) {
  logger.fatal('API_KEY environment variable is required');
  process.exit(1);
}

const app = express();

const authMiddleware = (req, res, next) => {
  const authHeader = req.headers.authorization;
  if (!authHeader || authHeader !== `Bearer ${API_KEY}`) {
    return res.status(401).json({ error: 'Unauthorized' });
  }
  next();
};

const upload = multer({ dest: '/tmp/uploads', limits: { fileSize: parseInt(MAX_FILE_SIZE) } });

app.get('/health', (req, res) => {
  res.json({ status: 'ok', timestamp: new Date().toISOString() });
});

app.post('/logs', authMiddleware, upload.single('file'), async (req, res) => {
  const { file } = req;
  const { filename, host, checksum } = req.body;

  if (!file || !filename || !host || !checksum) {
    if (file) await fs.remove(file.path);
    return res.status(400).json({ error: 'Missing required fields' });
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
        return res.json({ status: 'duplicate', filename, checksum });
      }
    }

    const uploadedChecksum = await calculateChecksum(file.path);
    if (uploadedChecksum !== checksum) {
      await fs.remove(file.path);
      return res.status(400).json({ error: 'Checksum mismatch' });
    }

    const tempTarget = targetPath + '.tmp';
    await fs.move(file.path, tempTarget, { overwrite: true });
    await fs.rename(tempTarget, targetPath);

    logger.info({ event: 'receiver_stored', filename, host, size: file.size }, 'File stored successfully');
    res.json({ status: 'stored', filename, checksum, size: file.size });

  } catch (error) {
    logger.error({ error: error.message }, 'Failed to store file');
    if (file && await fs.pathExists(file.path)) {
      await fs.remove(file.path);
    }
    res.status(500).json({ error: 'Internal server error' });
  }
});

function calculateChecksum(filePath) {
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
