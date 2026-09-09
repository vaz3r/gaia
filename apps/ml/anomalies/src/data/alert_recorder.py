import json
import logging
import requests
import psycopg2
from typing import Dict, List, Any, Optional
from datetime import datetime
from config import DB_HOST, DB_PORT, DB_USER, DB_PASSWORD, DB_NAME

logger = logging.getLogger(__name__)

class AlertRecorder:
    def __init__(
        self,
        host: str = DB_HOST,
        port: int = DB_PORT,
        user: str = DB_USER,
        password: str = DB_PASSWORD,
        dbname: str = DB_NAME,
        webhook_url: Optional[str] = None,
    ):
        self.conn_params = {
            "host": host,
            "port": port,
            "user": user,
            "password": password,
            "dbname": dbname,
            "connect_timeout": 10,
        }
        self.webhook_url = webhook_url

    def get_connection(self):
        return psycopg2.connect(**self.conn_params)

    def should_suppress_duplicate(self, incident_type: str, severity: str, window_minutes: int = 30) -> bool:
        """
        Check if an alert of the same incident_type and severity was already recorded recently.
        """
        query = """
            SELECT 1 FROM operational_alerts
            WHERE incident_type = %s AND severity = %s
              AND ts >= now() - INTERVAL '%s minutes'
            LIMIT 1
        """
        try:
            with self.get_connection() as conn:
                with conn.cursor() as cur:
                    cur.execute(query, (incident_type, severity, window_minutes))
                    return cur.fetchone() is not None
        except Exception as e:
            logger.warning(f"Could not check duplicate alerts: {e}")
            return False

    def record_alert(self, alert: Dict[str, Any], deduplicate: bool = True) -> Optional[int]:
        """
        Insert an operational alert record into operational_alerts table.
        """
        severity = alert.get("severity", "INFO")
        incident_type = alert.get("predicted_incident", "UNKNOWN")

        if deduplicate and self.should_suppress_duplicate(incident_type, severity):
            logger.info(f"Suppressing duplicate {severity} alert for {incident_type} within cool-off window.")
            return None

        query = """
            INSERT INTO operational_alerts (
                ts, anomaly_score, severity, incident_type, confidence, top_features, guidance
            ) VALUES (
                %s, %s, %s, %s, %s, %s, %s
            ) RETURNING id
        """
        ts = alert.get("timestamp")
        if isinstance(ts, str):
            ts = datetime.fromisoformat(ts)
        elif ts is None:
            ts = datetime.utcnow()

        top_features_json = json.dumps(alert.get("top_contributing_features", []))

        try:
            with self.get_connection() as conn:
                with conn.cursor() as cur:
                    cur.execute(
                        query,
                        (
                            ts,
                            alert.get("anomaly_score", 0.0),
                            severity,
                            incident_type,
                            alert.get("incident_confidence", 0.0),
                            top_features_json,
                            alert.get("actionable_guidance", ""),
                        ),
                    )
                    alert_id = cur.fetchone()[0]
                    logger.info(f"Recorded alert #{alert_id}: {severity} [{incident_type}] (Score: {alert.get('anomaly_score')})")

            # Optional webhook notification
            if self.webhook_url:
                self._dispatch_webhook(alert, alert_id)

            return alert_id
        except Exception as e:
            logger.error(f"Failed to record operational alert to Postgres: {e}")
            return None

    def _dispatch_webhook(self, alert: Dict[str, Any], alert_id: int):
        try:
            payload = {
                "alert_id": alert_id,
                "timestamp": str(alert.get("timestamp")),
                "severity": alert.get("severity"),
                "incident_type": alert.get("predicted_incident"),
                "anomaly_score": alert.get("anomaly_score"),
                "guidance": alert.get("actionable_guidance"),
                "top_features": alert.get("top_contributing_features"),
            }
            requests.post(self.webhook_url, json=payload, timeout=5)
        except Exception as e:
            logger.warning(f"Failed to dispatch webhook alert: {e}")
