import logging
import requests
import pandas as pd
from typing import Optional, List, Dict, Any
from datetime import datetime, date
from config import LOG_ANALYZER_URL

logger = logging.getLogger(__name__)

class LogExtractor:
    """
    Client for querying operational JSONL traces via gaia-log-analyzer DuckDB API.
    Enforces safe, partition-bounded queries to prevent memory contention on production.
    """
    def __init__(self, base_url: str = LOG_ANALYZER_URL, timeout: int = 15):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def get_slow_queries_predefined(self) -> pd.DataFrame:
        """
        Fetch top slow queries from the predefined /api/metrics/slow-queries endpoint.
        """
        url = f"{self.base_url}/api/metrics/slow-queries"
        try:
            resp = requests.get(url, timeout=self.timeout)
            resp.raise_for_status()
            data = resp.json()
            df = pd.DataFrame(data)
            if not df.empty and "time" in df.columns:
                df["time"] = pd.to_datetime(df["time"])
            return df
        except Exception as e:
            logger.warning(f"Failed to fetch slow queries from log-analyzer: {e}")
            return pd.DataFrame()

    def query_safe_partition(self, date_str: str, message_filter: str) -> pd.DataFrame:
        """
        Query DuckDB using a safe single-day or single-hour glob partition:
        e.g. date_str='2026-09-08' -> /logs/gaia-node/crawler-2026-09-08*.jsonl
        """
        target_path = f"/logs/gaia-node/crawler-{date_str}*.jsonl"
        safe_sql = f"""
            SELECT 
                (json->>'ts')::TIMESTAMP AS time,
                json->>'message' AS message,
                (json->>'elapsed_secs')::DOUBLE AS elapsed_secs,
                json->>'stream' AS stream,
                json->>'stage' AS stage,
                json->>'result' AS result,
                json->>'transport' AS transport
            FROM read_json_objects('{target_path}')
            WHERE json->>'message' = '{message_filter}'
            ORDER BY time ASC
        """
        url = f"{self.base_url}/api/query"
        try:
            resp = requests.post(url, json={"sql": safe_sql}, timeout=self.timeout)
            resp.raise_for_status()
            data = resp.json()
            df = pd.DataFrame(data)
            if not df.empty and "time" in df.columns:
                df["time"] = pd.to_datetime(df["time"])
            return df
        except Exception as e:
            logger.warning(f"Partition query for {date_str} failed or timed out: {e}")
            return pd.DataFrame()

    def extract_db_latency_incidents(self, min_duration_secs: float = 5.0) -> pd.DataFrame:
        """
        Extract database latency spike incidents from log analyzer.
        """
        df = self.get_slow_queries_predefined()
        if df.empty:
            return pd.DataFrame(columns=["time", "duration_secs", "query_statement", "rows_affected"])
        if "duration_secs" in df.columns:
            df = df[df["duration_secs"] >= min_duration_secs].copy()
        return df
