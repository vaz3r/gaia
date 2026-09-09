"""
Sprint 6: Resumable Keyset Backfill Engine.
High-throughput scoring backfill using keyset scanning and temporary staging set-updates.
"""
import os
import sys
import uuid
import time
import logging
from pathlib import Path
from datetime import datetime, timezone
import psycopg2
from psycopg2.extras import execute_values
import numpy as np

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.data.db import get_db_connection
from src.features.integrity import extract_integrity_features
from src.models.trust_classifier import TrustClassifier
from src.policy.engine import evaluate_policy, POLICY_VERSION
from src.common.types import ScoringResult

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("backfill")

MODEL_REGISTRY_DIR = Path(__file__).resolve().parent.parent / "model_registry"
DEFAULT_MODEL_PATH = MODEL_REGISTRY_DIR / "trust_classifier_v1.0.0.joblib"


class KeysetBackfillEngine:
    def __init__(
        self,
        batch_size: int = 2000,
        pacing_sleep_sec: float = 0.05,
        model_path: Path = DEFAULT_MODEL_PATH,
    ):
        self.batch_size = batch_size
        self.pacing_sleep_sec = pacing_sleep_sec
        self.backfill_id = uuid.uuid4()
        self.scoring_run_id = self.backfill_id

        logger.info(f"Loading TrustClassifier model from {model_path}...")
        self.classifier = TrustClassifier.load(model_path)
        logger.info("TrustClassifier loaded successfully.")

    def get_or_create_checkpoint(self, conn) -> bytes:
        with conn.cursor() as cur:
            cur.execute("SELECT last_infohash FROM scoring_checkpoints ORDER BY id DESC LIMIT 1;")
            row = cur.fetchone()
            if row and row[0]:
                logger.info(f"Resuming backfill from checkpoint infohash: {row[0].hex()[:16]}...")
                return row[0]
        return b""

    def save_checkpoint(self, conn, last_infohash: bytes, count: int):
        with conn.cursor() as cur:
            cur.execute("""
                INSERT INTO scoring_checkpoints (backfill_id, last_infohash, processed_count, updated_at)
                VALUES (%s, %s, %s, now());
            """, (str(self.backfill_id), last_infohash, count))

    def run_backfill(self, max_batches: Optional[int] = None):
        conn = get_db_connection()
        conn.autocommit = False
        last_infohash = self.get_or_create_checkpoint(conn)

        total_processed = 0
        batch_idx = 0
        start_time = time.time()

        try:
            while True:
                if max_batches is not None and batch_idx >= max_batches:
                    logger.info(f"Reached max batches limit ({max_batches}). Stopping.")
                    break

                batch_start = time.time()
                with conn.cursor() as cur:
                    # Keyset query avoiding expensive OFFSET
                    if last_infohash:
                        cur.execute("""
                            SELECT infohash, name, piece_length, total_size, file_count, 
                                   files, category, seed_confirmed, swarm_peers, last_seen
                            FROM torrents
                            WHERE infohash > %s
                            ORDER BY infohash ASC
                            LIMIT %s;
                        """, (last_infohash, self.batch_size))
                    else:
                        cur.execute("""
                            SELECT infohash, name, piece_length, total_size, file_count, 
                                   files, category, seed_confirmed, swarm_peers, last_seen
                            FROM torrents
                            ORDER BY infohash ASC
                            LIMIT %s;
                        """, (self.batch_size,))
                    rows = cur.fetchall()

                if not rows:
                    logger.info("No more torrents found to score. Backfill complete!")
                    break

                # 1. Feature extraction in memory
                feat_matrix = []
                metadata_list = []
                for r in rows:
                    feats = extract_integrity_features(
                        name=r[1],
                        total_size=r[3],
                        file_count=r[4],
                        piece_length=r[2],
                        files=r[5],
                    )
                    feat_matrix.append(list(feats.values()))
                    metadata_list.append(r)

                # 2. Batch model inference
                X_np = np.array(feat_matrix, dtype=np.float32)
                p_safe_batch = self.classifier.predict_safe_probability(X_np)

                # 3. Policy evaluation
                scoring_results = []
                for p_safe, r in zip(p_safe_batch, metadata_list):
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
                    )
                    scoring_results.append(res)

                # 4. Atomic staging write and update
                with conn.cursor() as cur:
                    cur.execute("""
                        CREATE TEMP TABLE staging_torrent_scores (
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
                        """
                        INSERT INTO staging_torrent_scores VALUES %s;
                        """,
                        staging_values,
                    )

                    # Set update on torrents
                    cur.execute("""
                        UPDATE torrents t
                        SET integrity_score = s.integrity_score,
                            model_safe_probability = s.model_safe_probability,
                            policy_integrity_score = s.policy_integrity_score,
                            policy_action = s.policy_action,
                            policy_version = s.policy_version,
                            decision_source = s.decision_source,
                            metadata_quality_score = s.metadata_quality_score,
                            availability_score = s.availability_score,
                            risk_tier = s.risk_tier,
                            availability_state = s.availability_state,
                            score_model_version = s.score_model_version,
                            scored_at = s.scored_at
                        FROM staging_torrent_scores s
                        WHERE t.infohash = s.infohash;
                    """)

                    # Insert into history table
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

                last_infohash = rows[-1][0]
                total_processed += len(rows)
                batch_idx += 1

                self.save_checkpoint(conn, last_infohash, total_processed)
                conn.commit()

                batch_dur = time.time() - batch_start
                rate = len(rows) / max(0.001, batch_dur)
                logger.info(
                    f"Batch {batch_idx}: Processed {len(rows)} torrents in {batch_dur:.2f}s "
                    f"({rate:.0f} rows/s) | Total: {total_processed:,}"
                )

                if self.pacing_sleep_sec > 0:
                    time.sleep(self.pacing_sleep_sec)

        except Exception as e:
            conn.rollback()
            logger.error(f"Backfill encountered error: {e}", exc_info=True)
            raise e
        finally:
            conn.close()

        total_elapsed = time.time() - start_time
        logger.info(
            f"Backfill completed: {total_processed:,} torrents scored in {total_elapsed:.1f}s "
            f"({total_processed / max(0.001, total_elapsed):.0f} rows/s avg)."
        )


if __name__ == "__main__":
    max_b = int(sys.argv[1]) if len(sys.argv) > 1 else None
    engine = KeysetBackfillEngine(batch_size=2000, pacing_sleep_sec=0.05)
    engine.run_backfill(max_batches=max_b)
