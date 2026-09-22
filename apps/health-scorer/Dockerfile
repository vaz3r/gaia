FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONPATH=/app

WORKDIR /app

# System deps
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc libpq-dev curl \
    && rm -rf /var/lib/apt/lists/*

# Python deps
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

# Application code
COPY . .

# Healthcheck: verify heartbeat file exists AND was written recently (within 90s)
# This catches stuck workers that hold the advisory lock but stop processing
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD test -f /tmp/health_scorer_heartbeat && \
        test $(($(date +%s) - $(stat -c %Y /tmp/health_scorer_heartbeat))) -lt 90 || exit 1

CMD ["python", "scripts/worker.py"]
