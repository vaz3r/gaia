-- Operational alerts table for ML-based crawler anomaly detection and monitoring
CREATE TABLE IF NOT EXISTS operational_alerts (
    id               BIGSERIAL PRIMARY KEY,
    ts               TIMESTAMPTZ NOT NULL DEFAULT now(),
    anomaly_score    DOUBLE PRECISION NOT NULL,
    severity         TEXT NOT NULL,
    incident_type    TEXT NOT NULL,
    confidence       DOUBLE PRECISION,
    top_features     JSONB,
    guidance         TEXT,
    resolved_at      TIMESTAMPTZ,
    CONSTRAINT valid_alert_severity CHECK (severity IN ('INFO', 'WARNING', 'CRITICAL'))
);

CREATE INDEX IF NOT EXISTS idx_operational_alerts_ts ON operational_alerts (ts DESC);
CREATE INDEX IF NOT EXISTS idx_operational_alerts_severity ON operational_alerts (severity);
CREATE INDEX IF NOT EXISTS idx_operational_alerts_incident ON operational_alerts (incident_type);
