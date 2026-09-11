"""
Continuous Shadow Scoring Daemon (gaia-scoring-worker).
Features:
1. Continuous scoring of newly discovered torrents (scored_at IS NULL).
2. Continuous availability refresh for stale scores (scored_at < NOW() - 24h).
3. Automated periodic retraining (default every 7 days / 168 hours) with Gold Benchmark gating.
4. Heartbeat file for container healthcheck.
5. Graceful shutdown on SIGTERM / SIGINT.
"""
import os
import sys
import time
import signal
import uuid
import logging
from pathlib import Path
from datetime import datetime, timezone
import numpy as np
import psycopg2
from psycopg2.extras import execute_values

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.data.db import get_db_connection
from src.features.integrity import extract_integrity_features
from src.models.trust_classifier import TrustClassifier
from src.policy.engine import evaluate_policy, POLICY_VERSION
from src.pipeline.train import run_training_pipeline

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] [scoring-worker] %(message)s",
)
logger = logging.getLogger("scoring-worker")

MODEL_PATH = Path(__file__).resolve().parent.parent / "model_registry" / "trust_classifier_v1.0.0.joblib"
HEARTBEAT_PATH = Path("/tmp/scoring_worker_heartbeat")


class ScoringWorker:
    def __init__(
        self,
        batch_size: int = 500,
        poll_interval_sec: float = 5.0,
        retrain_interval_hours: float = 168.0,  # 7 days default
        availability_refresh_hours: float = 24.0,
    ):
        self.batch_size = int(os.getenv("BATCH_SIZE", batch_size))
        self.poll_interval_sec = float(os.getenv("POLL_INTERVAL", poll_interval_sec))
        self.retrain_interval_sec = float(os.getenv("RETRAIN_INTERVAL_HOURS", retrain_interval_hours)) * 3600.0
        self.availability_refresh_hours = float(os.getenv("AVAILABILITY_REFRESH_HOURS", availability_refresh_hours))

        self.running = True
        self.scoring_run_id = uuid.uuid4()
        self.last_retrain_time = time.time()

        logger.info(f"Loading TrustClassifier model from {MODEL_PATH}...")
        self.classifier = TrustClassifier.load(MODEL_PATH)
        logger.info("TrustClassifier loaded successfully.")

        signal.signal(signal.SIGTERM, self._handle_signal)
        signal.signal(signal.SIGINT, self._handle_signal)

    def _handle_signal(self, signum, frame):
        logger.info(f"Received shutdown signal ({signum}). Stopping worker gracefully...")
        self.running = False

    def touch_heartbeat(self):
        try:
            HEARTBEAT_PATH.touch()
        except Exception:
            pass

    def run_scoring_batch(self, conn, rows, is_refresh: bool = False) -> int:
        if not rows:
            return 0

        # 1. Vectorized feature extraction
        feat_matrix = []
        for r in rows:
            feats = extract_integrity_features(
                name=r[1],
                total_size=r[3],
                file_count=r[4],
                piece_length=r[2],
                files=r[5],
            )
            feat_matrix.append(list(feats.values()))

        # 2. Batch calibrated prediction
        X_np = np.array(feat_matrix, dtype=np.float32)
        p_safe_batch = self.classifier.predict_safe_probability(X_np)

        # 3. Policy evaluation
        scoring_results = []
        now_utc = datetime.now(timezone.utc)
        for p_safe, r in zip(p_safe_batch, rows):
            # Compute fetch stats from query results (indices 10-13)
            total_fetches = r[10] or 0
            successful_fetches = r[11] or 0
            timeout_fetches = r[12] or 0
            last_success = r[13]

            if total_fetches > 0 and successful_fetches > 0:
                fetch_success_rate = successful_fetches / total_fetches
                fetch_timeout_rate = timeout_fetches / total_fetches
                if last_success is not None:
                    if last_success.tzinfo is None:
                        last_success = last_success.replace(tzinfo=timezone.utc)
                    hours_since_success = (now_utc - last_success).total_seconds() / 3600.0
                else:
                    hours_since_success = None
            else:
                fetch_success_rate = 0.0
                fetch_timeout_rate = 0.0
                hours_since_success = None

            res = evaluate_policy(
                infohash=r[0],
                model_safe_probability=float(p_safe),
                name=r[1],
                total_size=r[3],
                file_count=r[4],
                files=r[5],
                category=r[6],
                seed_confirmed=bool(r[7]),
                swarm_peers=r[8],
                last_seen=r[9],
                score_model_version="trust_classifier_v1.0.0",
                fetch_success_rate=fetch_success_rate,
                fetch_timeout_rate=fetch_timeout_rate,
                hours_since_success=hours_since_success,
            )
            scoring_results.append(res)

        # 4. Atomic staging write and update
        with conn.cursor() as cur:
            cur.execute("""
                CREATE TEMP TABLE staging_worker_scores (
                    infohash BYTEA PRIMARY KEY,
                    integrity_score SMALLINT,
                    model_safe_probability REAL,
                    policy_integrity_score SMALLINT,
                    policy_action VARCHAR(16),
                    policy_version VARCHAR(64),
                    decision_source VARCHAR(16),
                    metadata_quality_score SMALLINT,
                    availability_score SMALLINT,
                    risk_tier VARCHAR(16),
                    availability_state VARCHAR(16),
                    score_model_version VARCHAR(64),
                    scored_at TIMESTAMPTZ
                ) ON COMMIT DROP;
            """)

            staging_values = [
                (
                    s.infohash,
                    s.integrity_score,
                    s.model_safe_probability,
                    s.policy_integrity_score,
                    s.policy_action.value,
                    s.policy_version,
                    s.decision_source.value,
                    s.metadata_quality_score,
                    s.availability_score,
                    s.risk_tier.value,
                    s.availability_state.value,
                    s.score_model_version,
                    s.scored_at,
                )
                for s in scoring_results
            ]

            execute_values(
                cur,
                "INSERT INTO staging_worker_scores VALUES %s;",
                staging_values,
            )

            # Update torrents summary columns
            cur.execute("""
                UPDATE torrents t
                SET integrity_score = CASE WHEN t.decision_source = 'MANUAL' THEN t.integrity_score ELSE s.integrity_score END,
                    model_safe_probability = s.model_safe_probability,
                    policy_integrity_score = CASE WHEN t.decision_source = 'MANUAL' THEN t.policy_integrity_score ELSE s.policy_integrity_score END,
                    policy_action = CASE WHEN t.decision_source = 'MANUAL' THEN t.policy_action ELSE s.policy_action END,
                    policy_version = s.policy_version,
                    decision_source = CASE WHEN t.decision_source = 'MANUAL' THEN t.decision_source ELSE s.decision_source END,
                    metadata_quality_score = s.metadata_quality_score,
                    availability_score = s.availability_score,
                    risk_tier = CASE WHEN t.decision_source = 'MANUAL' THEN t.risk_tier ELSE s.risk_tier END,
                    availability_state = s.availability_state,
                    score_model_version = s.score_model_version,
                    scored_at = s.scored_at
                FROM staging_worker_scores s
                WHERE t.infohash = s.infohash;
            """)

            # Record in history table
            history_values = [
                (
                    s.infohash,
                    str(self.scoring_run_id),
                    "trust_classifier",
                    s.score_model_version,
                    s.model_safe_probability,
                    s.policy_integrity_score,
                    s.integrity_score,
                    s.metadata_quality_score,
                    s.availability_score,
                    s.risk_tier.value,
                    s.policy_action.value,
                    s.decision_source.value,
                    psycopg2.extras.Json(s.reason_codes),
                    "VALID",
                    s.scored_at,
                )
                for s in scoring_results
            ]

            execute_values(
                cur,
                """
                INSERT INTO torrent_score_history (
                    infohash, scoring_run_id, model_name, model_version,
                    model_safe_probability, policy_integrity_score, integrity_score,
                    metadata_quality_score, availability_score, risk_tier,
                    policy_action, decision_source, reason_codes, score_status, scored_at
                ) VALUES %s
                ON CONFLICT (infohash, scoring_run_id, model_name) DO NOTHING;
                """,
                history_values,
            )

        conn.commit()

        suppressed = sum(1 for s in scoring_results if s.policy_action.value == "SUPPRESS")
        allowed = sum(1 for s in scoring_results if s.policy_action.value == "ALLOW")
        review = sum(1 for s in scoring_results if s.policy_action.value in ("REVIEW", "DOWNRANK"))
        tag = "Refreshed" if is_refresh else "Scored"
        logger.info(
            f"{tag} {len(rows)} torrents (ALLOW={allowed}, REVIEW/DOWN={review}, SUPPRESS={suppressed})"
        )
        return len(rows)

    def run_scoring_cycle(self, conn) -> int:
        """
        Two-tiered operational cycle:
        Priority 1: Unscored new arrivals (scored_at IS NULL).
        Priority 2: Stale availability refresh (scored_at < NOW() - 24h).
        """
        # 1. Fetch unscored new torrents with fetch outcome stats
        with conn.cursor() as cur:
            cur.execute("""
                SELECT t.infohash, t.name, t.piece_length, t.total_size, t.file_count, 
                       t.files, t.category, t.seed_confirmed, t.swarm_peers, t.last_seen,
                       COALESCE(f.total_fetches, 0) AS total_fetches,
                       COALESCE(f.successful, 0) AS successful_fetches,
                       COALESCE(f.timeouts, 0) AS timeout_fetches,
                       f.last_success
                FROM torrents t
                LEFT JOIN LATERAL (
                    SELECT count(*) AS total_fetches,
                           count(*) FILTER (WHERE result IN ('ok','metadata_ok')) AS successful,
                           count(*) FILTER (WHERE result IN ('timeout','metadata_timeout')) AS timeouts,
                           max(created_at) FILTER (WHERE result IN ('ok','metadata_ok')) AS last_success
                    FROM fetch_peer_outcomes
                    WHERE infohash = t.infohash
                      AND created_at > now() - interval '48 hours'
                ) f ON true
                WHERE t.scored_at IS NULL
                ORDER BY t.verified_at DESC NULLS LAST
                LIMIT %s;
            """, (self.batch_size,))
            unscored_rows = cur.fetchall()

        if unscored_rows:
            return self.run_scoring_batch(conn, unscored_rows, is_refresh=False)

        # 2. If no new unscored torrents, refresh stale availability scores
        with conn.cursor() as cur:
            cur.execute("""
                SELECT t.infohash, t.name, t.piece_length, t.total_size, t.file_count, 
                       t.files, t.category, t.seed_confirmed, t.swarm_peers, t.last_seen,
                       COALESCE(f.total_fetches, 0) AS total_fetches,
                       COALESCE(f.successful, 0) AS successful_fetches,
                       COALESCE(f.timeouts, 0) AS timeout_fetches,
                       f.last_success
                FROM torrents t
                LEFT JOIN LATERAL (
                    SELECT count(*) AS total_fetches,
                           count(*) FILTER (WHERE result IN ('ok','metadata_ok')) AS successful,
                           count(*) FILTER (WHERE result IN ('timeout','metadata_timeout')) AS timeouts,
                           max(created_at) FILTER (WHERE result IN ('ok','metadata_ok')) AS last_success
                    FROM fetch_peer_outcomes
                    WHERE infohash = t.infohash
                      AND created_at > now() - interval '48 hours'
                ) f ON true
                WHERE t.scored_at < now() - (%s || ' hours')::interval
                ORDER BY t.scored_at ASC
                LIMIT %s;
            """, (str(self.availability_refresh_hours), self.batch_size))
            stale_rows = cur.fetchall()

        if stale_rows:
            return self.run_scoring_batch(conn, stale_rows, is_refresh=True)

        return 0

    def maybe_retrain(self):
        """Checks if periodic retraining interval has elapsed."""
        now = time.time()
        if now - self.last_retrain_time >= self.retrain_interval_sec:
            logger.info("Periodic retraining interval reached. Triggering model retraining pipeline...")
            try:
                run_training_pipeline()
                logger.info("Retraining successful. Hot-reloading updated classifier...")
                self.classifier = TrustClassifier.load(MODEL_PATH)
                self.last_retrain_time = now
                logger.info("Classifier hot-reloaded successfully with zero downtime.")
            except Exception as e:
                logger.error(f"Retraining failed; falling back to existing champion model: {e}", exc_info=True)
                # Retry in 6 hours instead of waiting full interval
                self.last_retrain_time = now - self.retrain_interval_sec + 21600

    def start(self):
        logger.info(
            f"Starting ScoringWorker daemon (run_id={self.scoring_run_id}, "
            f"batch_size={self.batch_size}, retrain_interval={self.retrain_interval_sec / 3600:.1f}h)..."
        )
        while self.running:
            self.touch_heartbeat()
            try:
                conn = get_db_connection()
                conn.autocommit = False
                scored = self.run_scoring_cycle(conn)
                conn.close()

                # Check periodic retraining
                self.maybe_retrain()

                if scored == 0:
                    time.sleep(self.poll_interval_sec)
                else:
                    time.sleep(0.1)

            except Exception as e:
                logger.error(f"Error in scoring cycle: {e}", exc_info=True)
                time.sleep(self.poll_interval_sec)

        logger.info("ScoringWorker stopped cleanly.")


def main():
    worker = ScoringWorker()
    worker.start()


if __name__ == "__main__":
    main()
