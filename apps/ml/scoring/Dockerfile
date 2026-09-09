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

COPY configs/ ./configs/
COPY model_registry/ ./model_registry/
COPY src/ ./src/
COPY scripts/ ./scripts/

RUN mkdir -p /app/data /app/reports

CMD ["python", "src/worker.py"]
