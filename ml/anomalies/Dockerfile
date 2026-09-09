FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONPATH=/app

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc \
    libpq-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY config.py ./
COPY src/ ./src/
COPY scripts/ ./scripts/

RUN mkdir -p /app/models_storage /app/data

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD test -f /tmp/worker_heartbeat || exit 1

CMD ["python", "src/worker.py"]
