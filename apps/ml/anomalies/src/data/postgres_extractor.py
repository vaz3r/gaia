import logging
import psycopg2
import pandas as pd
from typing import Optional, List, Dict, Any
from datetime import datetime
from config import DB_HOST, DB_PORT, DB_USER, DB_PASSWORD, DB_NAME

logger = logging.getLogger(__name__)

class PostgresExtractor:
    def __init__(
        self,
        host: str = DB_HOST,
        port: int = DB_PORT,
        user: str = DB_USER,
        password: str = DB_PASSWORD,
        dbname: str = DB_NAME,
    ):
        self.conn_params = {
            "host": host,
            "port": port,
            "user": user,
            "password": password,
            "dbname": dbname,
            "connect_timeout": 10,
        }

    def get_connection(self):
        return psycopg2.connect(**self.conn_params)

    def extract_raw_metrics(
        self,
        start_time: Optional[datetime] = None,
        end_time: Optional[datetime] = None,
        metric_names: Optional[List[str]] = None,
    ) -> pd.DataFrame:
        """
        Extract raw metric rows (ts, metric_name, metric_value) from PostgreSQL.
        """
        query = "SELECT ts, metric_name, metric_value FROM metrics"
        clauses = []
        params = []

        if start_time:
            clauses.append("ts >= %s")
            params.append(start_time)
        if end_time:
            clauses.append("ts <= %s")
            params.append(end_time)
        if metric_names:
            clauses.append("metric_name = ANY(%s)")
            params.append(metric_names)

        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += " ORDER BY ts ASC, metric_name ASC"

        logger.info("Executing metrics query...")
        with self.get_connection() as conn:
            df = pd.read_sql_query(query, conn, params=params if params else None)
        
        logger.info(f"Extracted {len(df):,} metric rows.")
        return df

    def extract_restart_events(self) -> List[datetime]:
        """
        Extract crawler restart timestamps from _session_start markers.
        """
        query = "SELECT ts FROM metrics WHERE metric_name = '_session_start' ORDER BY ts ASC"
        with self.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute(query)
                rows = cur.fetchall()
        restarts = [r[0] for r in rows]
        logger.info(f"Found {len(restarts)} restart markers (_session_start).")
        return restarts

    def extract_verification_job_hourly_stats(
        self,
        start_time: Optional[datetime] = None,
        end_time: Optional[datetime] = None,
    ) -> pd.DataFrame:
        """
        Extract hourly status breakdowns from verification_jobs.
        """
        query = """
            SELECT 
                date_trunc('hour', updated_at) AS ts_hour,
                status,
                count(*) AS count
            FROM verification_jobs
        """
        clauses = []
        params = []
        if start_time:
            clauses.append("updated_at >= %s")
            params.append(start_time)
        if end_time:
            clauses.append("updated_at <= %s")
            params.append(end_time)

        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += " GROUP BY 1, 2 ORDER BY 1 ASC, 2 ASC"

        with self.get_connection() as conn:
            df = pd.read_sql_query(query, conn, params=params if params else None)
        return df
