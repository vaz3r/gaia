const fs = require('fs-extra');
const path = require('path');
const axios = require('axios');
const FormData = require('form-data');
const crypto = require('crypto');
const pino = require('pino');

const logger = pino({ level: process.env.LOG_LEVEL || 'info' });

const SOURCE_DIR = process.env.SOURCE_DIR || '/logs';
const RECEIVER_URL = process.env.RECEIVER_URL || 'http://log-receiver:3000';
const API_KEY = process.env.API_KEY;
const SCAN_INTERVAL_MS = parseInt(process.env.SCAN_INTERVAL_MS) || 300000;
const FILE_MIN_AGE_MS = parseInt(process.env.FILE_MIN_AGE_MS) || 120000;
const HOST_NAME = process.env.HOST_NAME || 'gaia';

if (!API_KEY) {
  logger.fatal('API_KEY environment variable is required');
  process.exit(1);
}

const MANIFEST_PATH = path.join(SOURCE_DIR, 'processed.json');
let processedManifest = {};

async function loadManifest() {
  try {
    if (await fs.pathExists(MANIFEST_PATH)) {
      processedManifest = await fs.readJson(MANIFEST_PATH);
    }
  } catch (error) {
    logger.warn({ error: error.message }, 'Could not load manifest, starting fresh');
  }
}

async function saveManifest() {
  try {
    await fs.writeJson(MANIFEST_PATH, processedManifest);
  } catch (error) {
    logger.error({ error: error.message }, 'Failed to save manifest');
  }
}

function calculateChecksum(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('error', err => reject(err));
    stream.on('data', chunk => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

async function scanAndShip() {
  logger.info('Starting scan...');
  try {
    await fs.ensureDir(SOURCE_DIR);
    
    const files = await fs.readdir(SOURCE_DIR);
    const now = Date.now();

    for (const file of files) {
      if (!file.startsWith('crawler-') || !file.endsWith('.jsonl')) continue;

      const filePath = path.join(SOURCE_DIR, file);
      const stats = await fs.stat(filePath);
      
      // Check if file is old enough to ensure crawler is done writing to it
      if (now - stats.mtimeMs < FILE_MIN_AGE_MS) {
        continue;
      }

      if (processedManifest[file]) {
        logger.debug({ file }, 'File already processed, skipping');
        // Delete it if it still exists (maybe we failed to delete it last time)
        try {
          await fs.remove(filePath);
          logger.info({ file }, 'Cleaned up leftover processed file');
        } catch(e) {}
        continue;
      }

      const checksum = await calculateChecksum(filePath);
      logger.info({ file, size: stats.size }, 'Shipping file');

      const formData = new FormData();
      formData.append('filename', file);
      formData.append('host', HOST_NAME);
      formData.append('checksum', checksum);
      formData.append('file', fs.createReadStream(filePath));

      try {
        const response = await axios.post(`${RECEIVER_URL}/logs`, formData, {
          headers: {
            ...formData.getHeaders(),
            'Authorization': `Bearer ${API_KEY}`
          },
          maxContentLength: Infinity,
          maxBodyLength: Infinity
        });

        if (response.status === 200) {
          logger.info({ event: 'file_shipped', file }, 'File successfully shipped');
          processedManifest[file] = { checksum, shippedAt: new Date().toISOString() };
          await saveManifest();
          
          // Delete file from source!
          await fs.remove(filePath);
          logger.info({ file }, 'Deleted file from source directory');
        }
      } catch (error) {
        if (error.response && error.response.status === 200 && error.response.data && error.response.data.status === 'duplicate') {
           logger.info({ event: 'file_duplicate', file }, 'File was a duplicate on receiver');
           processedManifest[file] = { checksum, shippedAt: new Date().toISOString() };
           await saveManifest();
           await fs.remove(filePath);
        } else {
           logger.error({ event: 'file_failed', file, error: error.message }, 'Failed to ship file');
        }
      }
    }
  } catch (error) {
    logger.error({ error: error.message }, 'Error during scan loop');
  }
}

async function start() {
  await loadManifest();
  scanAndShip();
  setInterval(scanAndShip, SCAN_INTERVAL_MS);
}

start();
