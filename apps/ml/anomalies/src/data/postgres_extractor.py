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

    def extract_calibration_sample(self, lookback_hours: int = 24) -> Dict[str, Any]:
        """
        Evaluate empirical ground-truth accuracy by comparing recent wire verification
        probe observations against predicted Bayesian health scores.
        """
        query = """
            SELECT 
                COUNT(*) as total_probes,
                COUNT(CASE WHEN tao.observation_type IN ('metadata_fetch_success', 'seed_confirmed', 'probe_success') THEN 1 END) as successful_probes,
                COUNT(CASE WHEN hs.health_score >= 40 THEN 1 END) as predicted_viable,
                COUNT(CASE WHEN hs.health_score >= 40 AND tao.observation_type IN ('metadata_fetch_success', 'seed_confirmed', 'probe_success') THEN 1 END) as viable_successes,
                COUNT(CASE WHEN hs.health_score < 20 AND tao.observation_type IN ('metadata_fetch_failure', 'probe_failure') THEN 1 END) as dead_confirmed,
                COUNT(CASE WHEN hs.health_score < 20 THEN 1 END) as predicted_dead
            FROM (
                SELECT infohash, observation_type
                FROM torrent_availability_observations
                WHERE observed_at >= NOW() - make_interval(hours => %s)
                  AND observation_type IN ('metadata_fetch_success', 'metadata_fetch_failure', 'seed_confirmed', 'probe_success', 'probe_failure')
                ORDER BY observed_at DESC
                LIMIT 5000
            ) tao
            JOIN health_scores hs ON hs.infohash = encode(tao.infohash, 'hex')
        """
        with self.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute(query, (lookback_hours,))
                row = cur.fetchone()
                if not row or row[0] == 0:
                    return {"total_probes": 0, "viable_accuracy": 1.0, "sample_size": 0}
                total_probes, successful_probes, predicted_viable, viable_successes, dead_confirmed, predicted_dead = row
                
                # Accuracy across viable predictions
                precision = (viable_successes / predicted_viable) if predicted_viable and predicted_viable > 0 else 1.0
                return {
                    "total_probes": int(total_probes or 0),
                    "successful_probes": int(successful_probes or 0),
                    "predicted_viable": int(predicted_viable or 0),
                    "viable_successes": int(viable_successes or 0),
                    "viable_accuracy": round(float(precision), 4),
                    "sample_size": int(predicted_viable or 0),
                }

