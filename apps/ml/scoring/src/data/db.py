"""
Database connection and session utilities for scoring.
"""
import os
import logging
import psycopg2
from psycopg2.extras import RealDictCursor
from typing import Generator
from contextlib import contextmanager

logger = logging.getLogger(__name__)

DB_HOST = os.getenv("DB_HOST", "workspace-production")
DB_PORT = int(os.getenv("PG_PORT", os.getenv("DB_PORT", "5432")))
DB_USER = os.getenv("POSTGRES_USER", os.getenv("DB_USER", "crawler"))
DB_NAME = os.getenv("POSTGRES_DB", os.getenv("DB_NAME", "craw"))
DB_PASSWORD = os.getenv(
    "PG_PASSWORD",
    os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
)


def get_connection_params():
    return {
        "host": DB_HOST,
        "port": DB_PORT,
        "user": DB_USER,
        "password": DB_PASSWORD,
        "dbname": DB_NAME,
        "connect_timeout": 15,
    }


def get_db_connection():
    return psycopg2.connect(**get_connection_params())


@contextmanager
def get_db_cursor(commit: bool = False) -> Generator[psycopg2.extensions.cursor, None, None]:
    conn = get_db_connection()
    try:
        with conn.cursor(cursor_factory=RealDictCursor) as cur:
            yield cur
        if commit:
            conn.commit()
    except Exception as e:
        if commit:
            conn.rollback()
        raise e
    finally:
        conn.close()
