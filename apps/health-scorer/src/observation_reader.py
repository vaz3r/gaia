"""
Observation reader: fetches new observations from the database.

Uses the durable cursor to read only new observations since the last
processed batch. Returns Observation objects ready for aggregation.
"""

from __future__ import annotations

import logging
from typing import List

try:
    from . import db
    from .evidence_aggregator import Observation
except ImportError:
    import db
    from evidence_aggregator import Observation

logger = logging.getLogger(__name__)


def fetch_observations_batch(
    after_id: int,
    upper_bound: int,
    batch_size: int = 500,
    limit: Optional[int] = None,
) -> List[Observation]:
    if limit is not None:
        batch_size = limit
    """Fetch observations in the range (after_id, upper_bound].

    This range is consistent within a batch: rows inserted after upper_bound
    are not visible, ensuring idempotent processing.

    Args:
        after_id: Last successfully processed observation id.
        upper_bound: Maximum id to include (snapshot taken at batch start).
        batch_size: Maximum rows to fetch per query.

    Returns:
        List of Observation objects ordered by id ascending.
    """
    if after_id >= upper_bound:
        return []

    pool = db.get_pool()
    conn = pool.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            cur.execute(
                """
                SELECT id, infohash, observed_at, observation_type,
                       peer_count, seed_count, source, latency_ms,
                       failure_reason
                FROM torrent_availability_observations
                WHERE id > %s AND id <= %s
                ORDER BY id ASC
                LIMIT %s
                """,
                (after_id, upper_bound, batch_size),
            )
            rows = cur.fetchall()

        observations = []
        for r in rows:
            observations.append(
                Observation(
                    id=r[0],
                    infohash=r[1],
                    observed_at=r[2],
                    observation_type=r[3],
                    peer_count=r[4] or 0,
                    seed_count=r[5] or 0,
                    source=r[6] or "crawler",
                    latency_ms=r[7],
                    failure_reason=r[8],
                )
            )

        logger.debug(
            "Fetched %d observations in range (%d, %d]",
            len(observations),
            after_id,
            upper_bound,
        )
        return observations

    finally:
        db.release_conn(pool, conn)


def group_observations_by_infohash(
    observations: List[Observation],
) -> dict[bytes, List[Observation]]:
    """Group observations by infohash for per-torrent aggregation.

    Args:
        observations: Flat list of observations.

    Returns:
        Dict mapping infohash bytes to list of observations for that torrent.
    """
    groups: dict[bytes, List[Observation]] = {}
    for obs in observations:
        groups.setdefault(obs.infohash, []).append(obs)
    return groups
