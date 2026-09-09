import os
import sys
import time
import signal
import logging
import argparse
from pathlib import Path
from datetime import datetime, timedelta, timezone

# Add project root to path
BASE_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(BASE_DIR))

from config import MODELS_DIR, DATA_DIR
from src.data.postgres_extractor import PostgresExtractor
from src.data.feature_pipeline import FeaturePipeline
from src.data.alert_recorder import AlertRecorder
from src.pipeline.detector import AnomalyDetector
from src.pipeline.train import train_all_models

logging.basicConfig(
    level=os.getenv("LOG_LEVEL", "INFO").upper(),
    format="%(asctime)s [%(levelname)s] [worker] %(message)s",
)
logger = logging.getLogger("anomaly-worker")

class AnomalyWorker:
    def __init__(
        self,
        detection_interval_mins: int = int(os.getenv("DETECTION_INTERVAL_MINUTES", "15")),
        retrain_interval_hours: int = int(os.getenv("RETRAIN_INTERVAL_HOURS", "24")),
        alert_min_score: float = float(os.getenv("ALERT_MIN_SCORE", "0.60")),
        webhook_url: str = os.getenv("ALERT_WEBHOOK_URL", ""),
        heartbeat_file: str = os.getenv("HEARTBEAT_FILE", "/tmp/worker_heartbeat"),
    ):
        self.detection_interval_secs = detection_interval_mins * 60
        self.retrain_interval_secs = retrain_interval_hours * 3600
        self.alert_min_score = alert_min_score
        self.heartbeat_file = Path(heartbeat_file)
        self.running = True

        self.pg_extractor = PostgresExtractor()
        self.pipeline = FeaturePipeline()
        self.detector = AnomalyDetector(models_dir=MODELS_DIR)
        self.recorder = AlertRecorder(webhook_url=webhook_url if webhook_url else None)

        self.last_detection_time = 0.0
        self.last_retrain_time = time.time()  # Initialized so retraining runs on schedule

        # Signal Handlers
        signal.signal(signal.SIGINT, self._handle_signal)
        signal.signal(signal.SIGTERM, self._handle_signal)

    def _handle_signal(self, signum, frame):
        logger.info(f"Received signal {signum}. Initiating graceful shutdown...")
        self.running = False

    def update_heartbeat(self):
        try:
            self.heartbeat_file.write_text(str(datetime.now(timezone.utc).timestamp()))
        except Exception as e:
            logger.debug(f"Failed to touch heartbeat: {e}")

    def run_detection_cycle(self) -> int:
        """
        Fetch recent metrics from Postgres, evaluate anomaly scores, and record any alerts.
        Returns the number of alerts recorded.
        """
        logger.info("Executing periodic anomaly detection cycle...")
        end_time = datetime.now(timezone.utc)
        # Pull last 3 hours to allow stable delta computation
        start_time = end_time - timedelta(hours=3)

        try:
            raw_df = self.pg_extractor.extract_raw_metrics(start_time=start_time, end_time=end_time)
            if raw_df.empty:
                logger.warning("No metrics returned for recent window.")
                return 0

            features_df = self.pipeline.transform_raw_to_features(raw_df)
            if features_df.empty:
                logger.warning("No feature rows generated.")
                return 0

            # Evaluate the latest 1-2 intervals
            recent_features = features_df.tail(2)
            reports = self.detector.detect(recent_features)

            recorded_count = 0
            for r in reports:
                score = r["anomaly_score"]
                sev = r["severity"]
                inc = r["predicted_incident"]
                logger.info(f"Window: {r['timestamp']} | Score: {score:.3f} | Severity: {sev} | Incident: {inc}")

                if score >= self.alert_min_score:
                    alert_id = self.recorder.record_alert(r, deduplicate=True)
                    if alert_id:
                        recorded_count += 1
                        logger.warning(
                            f"ANOMALY ALERT [{sev}] #{alert_id}: {inc} (Score: {score:.3f})\n"
                            f"  Guidance: {r['actionable_guidance']}"
                        )

            logger.info(f"Detection cycle finished. Flagged alerts: {recorded_count}")
            return recorded_count
        except Exception as e:
            logger.error(f"Error during detection cycle: {e}", exc_info=True)
            return 0

    def run_retrain_cycle(self):
        """
        Extract extended history, retrain all models, and hot-reload weights into detector.
        """
        logger.info("Executing scheduled model retraining cycle...")
        try:
            results = train_all_models()
            logger.info(f"Retraining successful. Dataset size: {results['dataset_size']} hours.")
            self.detector.reload_models()
            self.last_retrain_time = time.time()
        except Exception as e:
            logger.error(f"Error during model retraining cycle: {e}", exc_info=True)

    def start(self):
        logger.info("Starting Gaia Anomaly Worker daemon...")
        logger.info(
            f"Config: Detection every {self.detection_interval_secs // 60}m, "
            f"Retraining every {self.retrain_interval_secs // 3600}h, "
            f"Alert Threshold >= {self.alert_min_score}"
        )

        # Ensure models exist on startup, train if missing
        if self.detector.iso_forest is None or self.detector.autoencoder is None:
            logger.info("Models missing on startup. Running initial training cycle...")
            self.run_retrain_cycle()

        # Run immediate detection on startup
        self.run_detection_cycle()
        self.last_detection_time = time.time()

        while self.running:
            self.update_heartbeat()
            now = time.time()

            # Check if detection is due
            if now - self.last_detection_time >= self.detection_interval_secs:
                self.run_detection_cycle()
                self.last_detection_time = time.time()

            # Check if retraining is due
            if now - self.last_retrain_time >= self.retrain_interval_secs:
                self.run_retrain_cycle()
                self.last_retrain_time = time.time()

            # Sleep in short increments to allow rapid response to shutdown signals
            for _ in range(10):
                if not self.running:
                    break
                time.sleep(1)

        logger.info("Anomaly Worker shutdown complete.")

def main():
    parser = argparse.ArgumentParser(description="Gaia Crawler Anomaly Worker")
    parser.add_argument("--run-once", action="store_true", help="Execute single detection cycle and exit")
    parser.add_argument("--retrain-now", action="store_true", help="Execute single retraining cycle and exit")
    args = parser.parse_args()

    worker = AnomalyWorker()

    if args.retrain_now:
        worker.run_retrain_cycle()
        sys.exit(0)

    if args.run_once:
        worker.run_detection_cycle()
        sys.exit(0)

    worker.start()

if __name__ == "__main__":
    main()
