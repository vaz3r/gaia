import type { Server } from 'http';
import pino from 'pino';
import { loadConfig } from './config.js';
import { startShipper } from './modules/shipper.js';
import { createReceiverApp } from './modules/receiver.js';
import { createAnalyzerApp, AnalyzerInstance } from './modules/analyzer.js';

const config = loadConfig();
const logger = pino({ level: config.logLevel });

logger.info({ profile: config.profile }, 'Starting Gaia Logger Worker...');

let receiverServer: Server | null = null;
let analyzerServer: Server | null = null;
let analyzerInstance: AnalyzerInstance | null = null;
let shipperHandle: { stop: () => void } | null = null;

async function bootstrap() {
  const shouldRunReceiver = config.profile === 'receiver' || config.profile === 'server' || config.profile === 'all';
  const shouldRunAnalyzer = config.profile === 'analyzer' || config.profile === 'server' || config.profile === 'all';
  const shouldRunShipper = config.profile === 'shipper' || config.profile === 'all';

  if (shouldRunReceiver) {
    logger.info({ port: config.receiverPort }, 'Launching log receiver...');
    const app = createReceiverApp(config, logger);
    receiverServer = app.listen(config.receiverPort, '0.0.0.0', () => {
      logger.info(`Log Receiver listening on 0.0.0.0:${config.receiverPort}`);
    });
  }

  if (shouldRunAnalyzer) {
    logger.info({ port: config.analyzerPort }, 'Launching DuckDB log analyzer...');
    analyzerInstance = await createAnalyzerApp(config, logger);
    analyzerServer = analyzerInstance.app.listen(config.analyzerPort, '0.0.0.0', () => {
      logger.info(`Log Analyzer listening on 0.0.0.0:${config.analyzerPort}`);
    });
  }

  if (shouldRunShipper) {
    logger.info({ receiverUrl: config.receiverUrl, sourceDir: config.sourceDir }, 'Launching log shipper...');
    shipperHandle = await startShipper(config, logger);
  }
}

async function shutdown(signal: string) {
  logger.info({ signal }, 'Shutting down Gaia Logger Worker...');

  if (shipperHandle) {
    shipperHandle.stop();
  }

  const closePromises: Promise<void>[] = [];

  if (receiverServer) {
    closePromises.push(new Promise((resolve) => receiverServer!.close(() => resolve())));
  }

  if (analyzerServer) {
    closePromises.push(new Promise((resolve) => analyzerServer!.close(() => resolve())));
  }

  if (analyzerInstance) {
    closePromises.push(analyzerInstance.close());
  }

  await Promise.all(closePromises);
  logger.info('Graceful shutdown completed.');
  process.exit(0);
}

process.on('SIGINT', () => shutdown('SIGINT'));
process.on('SIGTERM', () => shutdown('SIGTERM'));

bootstrap().catch((error) => {
  logger.fatal({ error: error.message, stack: error.stack }, 'Fatal error during startup');
  process.exit(1);
});
