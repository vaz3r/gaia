"""
Health scorer orchestrator.

Reads evidence (from observations or legacy adapter), computes canonical
health scores, and logs results. In shadow mode, no production writes occur.
"""

from __future__ import annotations

import json
import logging
import time
from dataclasses import asdict
from datetime import datetime, timezone
from typing import Dict, List, Optional

try:
    from .config import ALGORITHM_VERSION, SHADOW_MODE
    from .cursor import ScorerCursor
    from .evidence_aggregator import Observation, aggregate_observations
    from .formula import HealthScoreResult, HealthState, compute_health_score
    from .legacy_adapter import (
        derive_evidence_from_legacy,
        derive_observations_from_legacy,
        fetch_legacy_records,
    )
    from .observation_reader import (
        fetch_observations_batch,
        group_observations_by_infohash,
    )
except ImportError:
    from config import ALGORITHM_VERSION, SHADOW_MODE
    from cursor import ScorerCursor
    from evidence_aggregator import Observation, aggregate_observations
    from formula import HealthScoreResult, HealthState, compute_health_score
    from legacy_adapter import (
        derive_evidence_from_legacy,
        derive_observations_from_legacy,
        fetch_legacy_records,
    )
    from observation_reader import (
        fetch_observations_batch,
        group_observations_by_infohash,
    )

logger = logging.getLogger(__name__)


def _fetch_current_scores(infohashes: List[bytes]) -> Dict[bytes, Optional[int]]:
    """Fetch current health_score for comparison (read-only)."""
    try:
        from . import db
    except ImportError:
        import db

    if not infohashes:
        return {}

    pool = db.get_pool()
    conn = pool.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            cur.execute(
                """
                SELECT infohash, health_score
                FROM torrents
                WHERE infohash = ANY(%s)
                """,
                (infohashes,),
            )
            return {r[0]: r[1] for r in cur.fetchall()}
    finally:
        db.release_conn(pool, conn)


def _log_shadow_result(
    infohash: bytes,
    result: HealthScoreResult,
    current_score: Optional[int],
    evidence_source: str,
    evidence: object,
) -> None:
    """Log a shadow scoring result as structured JSON.

    This is the primary output of the shadow scorer. Each log line contains:
    - infohash_hex
    - proposed score, confidence, state
    - current score (if any)
    - score delta
    - evidence family breakdown
    - algorithm version
    - evidence source (observations, legacy_adapter, or mixed)
    - calculation timestamp
    """
    try:
        from .db import bytea_to_hex
    except ImportError:
        from db import bytea_to_hex

    # Build evidence breakdown
    evidence_breakdown = {}
    if hasattr(evidence, "direct_success_age_hours"):
        evidence_breakdown["direct"] = {
            "value": result.direct_component,
            "age_hours": evidence.direct_success_age_hours,
        }
        evidence_breakdown["seed"] = {
            "value": result.seed_component,
            "age_hours": evidence.seed_age_hours,
        }
        evidence_breakdown["peer"] = {
            "value": result.peer_component,
            "age_hours": evidence.peer_evidence_age_hours,
            "peer_count": evidence.max_recent_peer_count,
        }
        evidence_breakdown["dht"] = {
            "value": result.dht_component,
            "age_hours": evidence.latest_dht_sighting_age_hours,
            "count_12h": evidence.dht_sighting_count_12h,
        }
        evidence_breakdown["failure_penalty"] = {
            "value": result.failure_penalty,
            "age_hours": evidence.latest_failure_age_hours,
            "failures": evidence.recent_failures_after_latest_success,
        }

    delta = None
    if current_score is not None and result.health_score is not None:
        delta = result.health_score - current_score

    log_entry = {
        "infohash_hex": bytea_to_hex(infohash),
        "proposed_score": result.health_score,
        "proposed_confidence": result.health_confidence,
        "proposed_state": result.health_state.value,
        "current_score": current_score,
        "score_delta": delta,
        "evidence_families": evidence_breakdown,
        "evidence_source": evidence_source,
        "algorithm_version": result.algorithm_version,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    # Use INFO level so shadow results appear in production logs
    logger.info("SHADOW_SCORE %s", json.dumps(log_entry, default=str))


class HealthScorer:
    """Orchestrates evidence reading, aggregation, scoring, and logging.

    In shadow mode (default), computes scores and logs them without writing
    to the torrents table or Redis. In production mode (M2+), writes canonical
    scores atomically.
    """

    def __init__(self, shadow_mode: bool = SHADOW_MODE):
        self.shadow_mode = shadow_mode
        self._stats = {
            "batches_processed": 0,
            "observations_processed": 0,
            "infohashes_scored": 0,
            "legacy_records_scored": 0,
        }

    def process_batch(self, batch_size: int = 500, cursor: Optional[ScorerCursor] = None) -> int:
        """Process a batch of observations through the scoring pipeline.

        Args:
            batch_size: Maximum observations to process per batch.
            cursor: Pre-acquired ScorerCursor. If None, creates and acquires one.

        Returns:
            Number of observations processed.
        """
        own_cursor = cursor is None
        if own_cursor:
            cursor = ScorerCursor()
            if not cursor.acquire():
                logger.debug("Scorer is not the active singleton, skipping batch")
                return 0

        t0 = time.time()
        try:
            # 1. Read cursor position and get batch upper bound
            after_id, _ = cursor.get_cursor_position()
            upper_bound = cursor.get_batch_upper_bound()

            if after_id >= upper_bound:
                logger.debug(
                    "No new observations (cursor=%d, upper=%d)", after_id, upper_bound
                )
                return 0

            # 2. Fetch new observations
            observations = fetch_observations_batch(after_id, upper_bound, batch_size)
            if not observations:
                cursor.advance(upper_bound)
                return 0

            # 3. Group by infohash
            groups = group_observations_by_infohash(observations)

            # 4. Fetch current scores for comparison
            current_scores = _fetch_current_scores(list(groups.keys()))

            # 5. Score each infohash
            max_obs_id = 0
            for infohash, obs_list in groups.items():
                evidence = aggregate_observations(obs_list)
                result = compute_health_score(
                    evidence, algorithm_version=ALGORITHM_VERSION
                )
                current = current_scores.get(infohash)

                _log_shadow_result(
                    infohash, result, current, "observations", evidence
                )

                max_obs_id = max(max_obs_id, max(o.id for o in obs_list))
                self._stats["infohashes_scored"] += 1

            # 6. Advance cursor
            if max_obs_id > 0:
                cursor.advance(max_obs_id)

            self._stats["batches_processed"] += 1
            self._stats["observations_processed"] += len(observations)

            elapsed = time.time() - t0
            logger.info(
                "Batch processed: %d observations, %d infohashes, %.3fs",
                len(observations),
                len(groups),
                elapsed,
            )

            return len(observations)
        finally:
            if own_cursor:
                cursor.release()

    def process_legacy_batch(self, batch_size: int = 1000, offset: int = 0) -> int:
        """Process a batch of legacy torrents through the scoring pipeline.

        This reads existing torrent records and derives provisional evidence.
        Used for shadow validation when the observation table is empty.

        Returns:
            Number of legacy records processed.
        """
        t0 = time.time()

        records = fetch_legacy_records(limit=batch_size, offset=offset)
        if not records:
            return 0

        # Fetch current scores for comparison
        current_scores = _fetch_current_scores([r.infohash for r in records])

        for record in records:
            evidence = derive_evidence_from_legacy(record)
            result = compute_health_score(
                evidence, algorithm_version=ALGORITHM_VERSION
            )
            current = current_scores.get(record.infohash)

            _log_shadow_result(
                record.infohash, result, current, "legacy_adapter", evidence
            )

            self._stats["legacy_records_scored"] += 1

        elapsed = time.time() - t0
        logger.info(
            "Legacy batch processed: %d records, %.3fs", len(records), elapsed
        )

        return len(records)

    @property
    def stats(self) -> dict:
        return dict(self._stats)
