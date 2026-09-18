import dotenv from 'dotenv';
import path from 'path';

dotenv.config();

export type LogProfile = 'shipper' | 'receiver' | 'analyzer' | 'server' | 'all';

export interface LoggerConfig {
  profile: LogProfile;
  logLevel: string;
  apiKey: string;
  // Shipper
  sourceDir: string;
  receiverUrl: string;
  scanIntervalMs: number;
  fileMinAgeMs: number;
  hostName: string;
  // Receiver
  storageDir: string;
  receiverPort: number;
  maxFileSize: number;
  // Analyzer
  logsDir: string;
  analyzerPort: number;
}

export function loadConfig(): LoggerConfig {
  const rawProfile = (process.env.LOG_PROFILE || process.env.PROFILE || 'server').toLowerCase() as LogProfile;
  const validProfiles: LogProfile[] = ['shipper', 'receiver', 'analyzer', 'server', 'all'];
  const profile: LogProfile = validProfiles.includes(rawProfile) ? rawProfile : 'server';

  const receiverPort = process.env.RECEIVER_PORT 
    ? parseInt(process.env.RECEIVER_PORT, 10) 
    : (profile === 'receiver' && process.env.PORT ? parseInt(process.env.PORT, 10) : 3000);

  const analyzerPort = process.env.ANALYZER_PORT 
    ? parseInt(process.env.ANALYZER_PORT, 10) 
    : (profile === 'analyzer' && process.env.PORT ? parseInt(process.env.PORT, 10) : 3001);

  return {
    profile,
    logLevel: process.env.LOG_LEVEL || 'info',
    apiKey: process.env.API_KEY || process.env.LOG_SHIPPER_API_KEY || '',
    // Shipper
    sourceDir: process.env.SOURCE_DIR || '/logs',
    receiverUrl: process.env.RECEIVER_URL || 'http://100.87.194.112:3100',
    scanIntervalMs: process.env.SCAN_INTERVAL_MS ? parseInt(process.env.SCAN_INTERVAL_MS, 10) : 300000,
    fileMinAgeMs: process.env.FILE_MIN_AGE_MS ? parseInt(process.env.FILE_MIN_AGE_MS, 10) : 120000,
    hostName: process.env.HOST_NAME || 'gaia',
    // Receiver
    storageDir: process.env.STORAGE_DIR || '/logs',
    receiverPort,
    maxFileSize: process.env.MAX_FILE_SIZE ? parseInt(process.env.MAX_FILE_SIZE, 10) : 104857600,
    // Analyzer
    logsDir: process.env.LOGS_DIR || process.env.STORAGE_DIR || '/logs',
    analyzerPort,
  };
}
