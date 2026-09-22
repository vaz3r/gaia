"""
Pytest configuration and fixtures for the health scorer test suite.

Provides:
- PostgreSQL test database setup/teardown
- Migration runner
- Fixture data seeder
- Advisory lock coverage (two-worker test)
"""

import os
import sys
import subprocess
import time
import threading
from pathlib import Path

import psycopg2
import pytest

# Ensure src is on the path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(SRC_DIR))

MIGRATIONS_DIR = Path(__file__).parent.parent / "migrations"

# Default test database URL (overridden by TEST_DATABASE_URL env var)
DEFAULT_TEST_DB_URL = "postgresql://testuser:testpass@localhost:5432/test_health_scorer"


def pytest_configure(config):
    """Register custom markers."""
    config.addinivalue_line("markers", "integration: marks tests requiring PostgreSQL")


def _get_db_url():
    return os.environ.get("TEST_DATABASE_URL", DEFAULT_TEST_DB_URL)


def _parse_db_url(url):
    """Parse postgresql://user:pass@host:port/dbname into components."""
    from urllib.parse import urlparse
    parsed = urlparse(url)
    return {
        "host": parsed.hostname or "localhost",
        "port": parsed.port or 5432,
        "user": parsed.username or "postgres",
        "password": parsed.password or "",
        "dbname": parsed.path.lstrip("/") or "postgres",
    }


def _connect_super(url):
    """Connect to the 'postgres' maintenance database for CREATE/DROP."""
    parts = _parse_db_url(url)
    # Connect to 'postgres' db for admin operations
    conn = psycopg2.connect(
        host=parts["host"],
        port=parts["port"],
        user=parts["user"],
        password=parts["password"],
        dbname="postgres",
    )
    conn.autocommit = True
    return conn


def _run_test_schema(url):
    """Apply the test schema (minimal production-compatible tables)."""
    parts = _parse_db_url(url)
    schema_file = Path(__file__).parent / "schema.sql"
    conn = psycopg2.connect(**parts)
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(schema_file.read_text())
    finally:
        conn.close()


def _run_migration(url):
    """Apply the health-scorer migration files to the test database.

    For 0001b (CONCURRENTLY indexes), each statement is executed individually
    with autocommit to avoid the 'cannot run inside a transaction block' error.
    """
    parts = _parse_db_url(url)

    # 0001a — transactional (single execute is fine)
    m_a = MIGRATIONS_DIR / "0001a_health_scoring_tables.sql"
    if m_a.exists():
        conn = psycopg2.connect(**parts)
        conn.autocommit = True
        try:
            with conn.cursor() as cur:
                cur.execute(m_a.read_text())
        finally:
            conn.close()

    # 0001b — non-transactional (CONCURRENTLY): must execute each statement
    # separately with autocommit, otherwise psycopg2 wraps in a transaction.
    m_b = MIGRATIONS_DIR / "0001b_health_scoring_indexes.sql"
    if m_b.exists():
        conn = psycopg2.connect(**parts)
        conn.autocommit = True
        try:
            sql = m_b.read_text()
            # Split on semicolons, filter comments and blanks
            statements = []
            current = []
            for line in sql.split("\n"):
                stripped = line.strip()
                if stripped.startswith("--") or stripped == "":
                    continue
                current.append(line)
                if stripped.endswith(";"):
                    stmt = "\n".join(current).strip()
                    if stmt:
                        statements.append(stmt)
                    current = []
            # Execute each statement individually
            with conn.cursor() as cur:
                for stmt in statements:
                    cur.execute(stmt)
        finally:
            conn.close()


def _seed_torrents(conn, count=5):
    """Insert fixture torrent records for legacy adapter testing."""
    with conn.cursor() as cur:
        for i in range(count):
            infohash = f"\\x{ i:040x}".encode()
            cur.execute(
                """
                INSERT INTO torrents (
                    infohash, name, category, total_size, verified_at,
                    last_seen, swarm_peers, seed_confirmed, total_seen,
                    health_score, last_health_check
                ) VALUES (
                    %s, %s, 'movie', 1000000,
                    now() - interval '%s hours',
                    now() - interval '%s hours',
                    %s, %s, %s,
                    %s, now() - interval '%s hours'
                )
                ON CONFLICT (infohash) DO NOTHING
                """,
                (
                    infohash,
                    f"Test Torrent {i}",
                    i * 12,   # verified_at age
                    i * 6,    # last_seen age
                    max(0, 50 - i * 10),  # swarm_peers
                    i % 2 == 0,  # seed_confirmed
                    i * 3,  # total_seen
                    max(0, 90 - i * 15),  # health_score
                    i * 8,  # last_health_check age
                ),
            )
    conn.commit()


def _seed_observations(conn, infohash, count=5):
    """Insert fixture observations for a given infohash."""
    types = [
        "metadata_fetch_success", "probe_success",
        "peer_seen", "dht_sighting", "seed_confirmed",
    ]
    with conn.cursor() as cur:
        for i in range(count):
            obs_type = types[i % len(types)]
            peers = 10 if obs_type == "peer_seen" else 0
            seeds = 1 if obs_type == "seed_confirmed" else 0
            cur.execute(
                """
                INSERT INTO torrent_availability_observations (
                    infohash, observation_type, peer_count, seed_count,
                    source, observed_at, latency_ms
                ) VALUES (
                    %s, %s, %s, %s, 'test_fixture',
                    now() - interval '%s hours',
                    %s
                )
                """,
                (infohash, obs_type, peers, seeds, i * 2, 50 + i * 10),
            )
    conn.commit()


# ============================================================
# Fixtures
# ============================================================

@pytest.fixture(scope="session")
def db_url():
    """The test database URL."""
    return _get_db_url()


@pytest.fixture(scope="session")
def setup_test_database(db_url):
    """Create and migrate the test database (session-scoped)."""
    parts = _parse_db_url(db_url)
    dbname = parts["dbname"]

    # Connect to maintenance DB and (re)create the test database
    super_conn = _connect_super(db_url)
    with super_conn.cursor() as cur:
        # Terminate existing connections
        cur.execute(
            "SELECT pg_terminate_backend(pid) "
            "FROM pg_stat_activity WHERE datname = %s AND pid != pg_backend_pid()",
            (dbname,),
        )
        cur.execute(f"DROP DATABASE IF EXISTS {dbname}")
        cur.execute(f"CREATE DATABASE {dbname}")
    super_conn.close()

    # Run migrations
    _run_test_schema(db_url)
    _run_migration(db_url)

    yield db_url

    # Teardown: drop the test database
    super_conn = _connect_super(db_url)
    with super_conn.cursor() as cur:
        cur.execute(
            "SELECT pg_terminate_backend(pid) "
            "FROM pg_stat_activity WHERE datname = %s AND pid != pg_backend_pid()",
            (dbname,),
        )
        cur.execute(f"DROP DATABASE IF EXISTS {dbname}")
    super_conn.close()


@pytest.fixture(scope="session", autouse=True)
def _configure_db_env(setup_test_database):
    """Set environment variables so all code paths connect to the test database."""
    parts = _parse_db_url(setup_test_database)
    os.environ["DB_HOST"] = parts["host"]
    os.environ["PG_PORT"] = str(parts["port"])
    os.environ["POSTGRES_USER"] = parts["user"]
    os.environ["PG_PASSWORD"] = parts["password"]
    os.environ["POSTGRES_DB"] = parts["dbname"]
    os.environ["SHADOW_MODE"] = "true"
    # Reset the module-level pool so it picks up new env vars
    import db
    db._connection_pool = None
    yield
    db._connection_pool = None


@pytest.fixture(scope="function")
def db_conn(setup_test_database):
    """A fresh database connection for each test (autocommit, rolled back after)."""
    parts = _parse_db_url(setup_test_database)
    conn = psycopg2.connect(**parts)
    conn.autocommit = True
    yield conn
    if not conn.closed:
        conn.close()


@pytest.fixture(scope="function")
def db_pool(setup_test_database):
    """A ThreadedConnectionPool for testing scorer modules."""
    from psycopg2 import pool as pg_pool
    parts = _parse_db_url(setup_test_database)
    p = pg_pool.ThreadedConnectionPool(
        minconn=2,
        maxconn=5,
        **parts,
    )
    yield p
    p.closeall()


@pytest.fixture(scope="function")
def seeded_db(setup_test_database):
    """Database with fixture torrents and observations seeded."""
    parts = _parse_db_url(setup_test_database)
    conn = psycopg2.connect(**parts)
    conn.autocommit = True

    _seed_torrents(conn, count=5)

    # Seed observations for first torrent
    with conn.cursor() as cur:
        cur.execute("SELECT infohash FROM torrents ORDER BY verified_at LIMIT 1")
        ih = cur.fetchone()[0]

    _seed_observations(conn, ih, count=5)

    yield conn
    if not conn.closed:
        conn.close()


@pytest.fixture(scope="function")
def cursor(setup_test_database):
    """A ScorerCursor connected to the test database."""
    # Patch db module to use test database
    parts = _parse_db_url(setup_test_database)
    os.environ["DB_HOST"] = parts["host"]
    os.environ["PG_PORT"] = str(parts["port"])
    os.environ["POSTGRES_USER"] = parts["user"]
    os.environ["PG_PASSWORD"] = parts["password"]
    os.environ["POSTGRES_DB"] = parts["dbname"]

    # Reset the module-level pool
    import db
    db._connection_pool = None

    from cursor import ScorerCursor
    c = ScorerCursor()
    yield c
    c.release()
    db._connection_pool = None


@pytest.fixture(scope="function")
def scorer(setup_test_database):
    """A HealthScorer in shadow mode connected to the test database."""
    parts = _parse_db_url(setup_test_database)
    os.environ["DB_HOST"] = parts["host"]
    os.environ["PG_PORT"] = str(parts["port"])
    os.environ["POSTGRES_USER"] = parts["user"]
    os.environ["PG_PASSWORD"] = parts["password"]
    os.environ["POSTGRES_DB"] = parts["dbname"]
    os.environ["SHADOW_MODE"] = "true"

    import db
    db._connection_pool = None

    from scorer import HealthScorer
    s = HealthScorer(shadow_mode=True)
    yield s
    db._connection_pool = None
