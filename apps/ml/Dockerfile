FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONPATH=/app:/app/scoring:/app/anomalies

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc \
    g++ \
    libpq-dev \
    curl \
    procps \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY . .

RUN mkdir -p /app/scoring/model_registry /app/scoring/reports /app/anomalies/models_storage /app/anomalies/data

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD test -f /tmp/gaia_ml_healthy || exit 1

ENTRYPOINT ["python", "supervisor.py"]
