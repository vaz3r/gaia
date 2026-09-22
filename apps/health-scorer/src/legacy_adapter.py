"""
Legacy evidence adapter: derives provisional evidence from existing facts.

When torrent_availability_observations is empty (before Milestone 2 activates
crawler observation writes), this adapter reads existing columns to provide
the shadow scorer with something to process. This enables formula validation
without changing any production writer.

All reads are strictly read-only. No writes are performed.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Dict, List, Optional

try:
    from . import db
    from .evidence_aggregator import EvidenceFamilies, Observation, _hours_since
except ImportError:
    import db
    from evidence_aggregator import EvidenceFamilies, Observation, _hours_since

logger = logging.getLogger(__name__)


@dataclass
class LegacyTorrentRecord:
    """Minimal torrent record for legacy evidence derivation."""

    infohash: bytes
    last_seen: Optional[datetime]
    swarm_peers: int
    seed_confirmed: bool
    total_seen: int
    last_health_check: Optional[datetime]
    health_score: Optional[int]

    # From fetch_peer_outcomes (aggregated)
    fetch_total: int = 0
    fetch_successful: int = 0
    fetch_timeouts: int = 0
    fetch_last_success: Optional[datetime] = None


def fetch_legacy_records(
    limit: int = 1000,
    offset: int = 0,
) -> List[LegacyTorrentRecord]:
    """Fetch torrents with existing health-related columns.

    Reads directly from the torrents table and fetch_peer_outcomes.
    No writes are performed.

    Args:
        limit: Maximum records to fetch.
        offset: Pagination offset.

    Returns:
        List of LegacyTorrentRecord objects.
    """
    pool = db.get_pool()
    conn = pool.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            # Fetch torrents with health columns
            cur.execute(
                """
                SELECT t.infohash, t.last_seen, t.swarm_peers,
                       t.seed_confirmed, t.total_seen,
                       t.last_health_check, t.health_score
                FROM torrents t
                WHERE t.policy_action IS DISTINCT FROM 'SUPPRESS'
                ORDER BY t.verified_at DESC NULLS LAST
                LIMIT %s OFFSET %s
                """,
                (limit, offset),
            )
            torrent_rows = cur.fetchall()

            if not torrent_rows:
                return []

            # Fetch aggregated fetch_peer_outcomes for these torrents
            infohashes = [r[0] for r in torrent_rows]
            cur.execute(
                """
                SELECT infohash,
                       count(*) AS total,
                       count(*) FILTER (WHERE result IN ('ok', 'metadata_ok')) AS successful,
                       count(*) FILTER (WHERE result IN ('timeout', 'metadata_timeout')) AS timeouts,
                       max(created_at) FILTER (WHERE result IN ('ok', 'metadata_ok')) AS last_success
                FROM fetch_peer_outcomes
                WHERE infohash = ANY(%s)
                  AND created_at > now() - interval '48 hours'
                GROUP BY infohash
                """,
                (infohashes,),
            )
            outcome_rows = cur.fetchall()
            outcomes_map = {r[0]: r[1:] for r in outcome_rows}

        records = []
        for r in torrent_rows:
            infohash = r[0]
            outcomes = outcomes_map.get(infohash, (0, 0, 0, None))
            records.append(
                LegacyTorrentRecord(
                    infohash=infohash,
                    last_seen=r[1],
                    swarm_peers=r[2] or 0,
                    seed_confirmed=r[3] or False,
                    total_seen=r[4] or 0,
                    last_health_check=r[5],
                    health_score=r[6],
                    fetch_total=outcomes[0] or 0,
                    fetch_successful=outcomes[1] or 0,
                    fetch_timeouts=outcomes[2] or 0,
                    fetch_last_success=outcomes[3],
                )
            )

        logger.debug(
            "Fetched %d legacy records (offset=%d)", len(records), offset
        )
        return records

    finally:
        db.release_conn(pool, conn)


def derive_evidence_from_legacy(record: LegacyTorrentRecord) -> EvidenceFamilies:
    """Convert a legacy torrent record into EvidenceFamilies.

    This maps existing columns to the new evidence family structure:
    - seed_confirmed → seed family (age derived from last_seen)
    - swarm_peers → peer family (age derived from last_seen)
    - fetch_peer_outcomes → direct success/failure families
    - total_seen → DHT evidence (very rough approximation)

    The source is labeled 'legacy_adapter' in the shadow log.

    Args:
        record: Legacy torrent record from the database.

    Returns:
        EvidenceFamilies with provisional evidence.
    """
    now = datetime.now(timezone.utc)

    # Direct success: from fetch_peer_outcomes
    direct_success_age = None
    if record.fetch_last_success is not None:
        direct_success_age = _hours_since(record.fetch_last_success, now)

    # Seed: from seed_confirmed column (age = time since last_seen)
    seed_age = None
    if record.seed_confirmed and record.last_seen is not None:
        seed_age = _hours_since(record.last_seen, now)

    # Peer: from swarm_peers column (age = time since last_seen)
    peer_count = record.swarm_peers
    peer_age = None
    if peer_count > 0 and record.last_seen is not None:
        peer_age = _hours_since(record.last_seen, now)

    # DHT: rough approximation from total_seen
    # We don't have per-sighting timestamps, so we use total_seen as a
    # weak proxy. This is intentionally conservative.
    dht_count = 0
    dht_age = None
    if record.total_seen > 0 and record.last_seen is not None:
        # Treat last_seen as the latest DHT evidence
        dht_age = _hours_since(record.last_seen, now)
        # Use min(total_seen, 12) as a rough 12h window approximation
        dht_count = min(record.total_seen, 12)

    # Failures: from fetch_peer_outcomes
    recent_failures = 0
    latest_failure_age = None
    if record.fetch_timeouts > 0 and record.fetch_last_success is not None:
        # Only count failures as "after latest success" if we have fetch data
        # We can't distinguish failure timestamps from aggregate counts,
        # so we use a conservative estimate
        recent_failures = min(record.fetch_timeouts, 3)
        # Assume failures are roughly as old as the time since last success
        if direct_success_age is not None:
            latest_failure_age = direct_success_age + 1.0  # slightly older

    return EvidenceFamilies(
        direct_success_age_hours=direct_success_age,
        seed_age_hours=seed_age,
        max_recent_peer_count=peer_count,
        peer_evidence_age_hours=peer_age,
        dht_sighting_count_12h=dht_count,
        latest_dht_sighting_age_hours=dht_age,
        recent_failures_after_latest_success=recent_failures,
        latest_failure_age_hours=latest_failure_age,
    )


def derive_observations_from_legacy(record: LegacyTorrentRecord) -> List[Observation]:
    """Convert a legacy record into synthetic Observation objects for logging.

    These are not real observations; they are derived for the shadow scorer's
    comparison logs. The evidence_source field will be set to 'legacy_adapter'.
    """
    observations = []
    base_id = 0  # Synthetic IDs for legacy records

    if record.fetch_successful > 0 and record.fetch_last_success is not None:
        observations.append(
            Observation(
                id=base_id,
                infohash=record.infohash,
                observed_at=record.fetch_last_success,
                observation_type="metadata_fetch_success",
                peer_count=0,
                seed_count=0,
                source="legacy_adapter",
            )
        )

    if record.seed_confirmed and record.last_seen is not None:
        observations.append(
            Observation(
                id=base_id + 1,
                infohash=record.infohash,
                observed_at=record.last_seen,
                observation_type="seed_confirmed",
                peer_count=0,
                seed_count=1,
                source="legacy_adapter",
            )
        )

    if record.swarm_peers > 0 and record.last_seen is not None:
        observations.append(
            Observation(
                id=base_id + 2,
                infohash=record.infohash,
                observed_at=record.last_seen,
                observation_type="peer_seen",
                peer_count=record.swarm_peers,
                seed_count=0,
                source="legacy_adapter",
            )
        )

    return observations
