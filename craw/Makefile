HOST      ?= zerone
DB_HOST   ?= workspace-production
TAG       := $(shell git rev-parse --short HEAD)
REMOTE    ?= ubuntu@$(HOST)
DB_REMOTE ?= core@$(DB_HOST)
SSH_KEY   ?= zerone
REMOTE_GIT ?= /home/ubuntu/craw-stack
REMOTE_DIR ?= $(REMOTE_GIT)/craw
DB_REMOTE_DIR ?= /home/core/craw-stack

.PHONY: dev down deploy deploy-db rollback ps logs backup db-ps db-logs db-backup init-remote clean-remote

# ── Local development ──

dev:
	docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d

down:
	docker compose -f docker-compose.yml -f docker-compose.dev.yml down

# ── Production deploy ──

deploy:
	./deploy.sh $(HOST) HEAD

rollback:
	./deploy.sh $(HOST) $(ROLLBACK_TAG)

# ── Production ops ──

ps:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_DIR) && docker compose ps"

logs:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_DIR) && docker compose logs -f --tail=100 crawler dashboard"

# ── One-time remote setup ──

init-remote:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd /home/ubuntu && \
		 git clone $(shell git remote get-url origin) craw-stack || true && \
		 cd craw-stack && git fetch origin && \
		 cp craw-stack/.env craw/.env 2>/dev/null || true"

# ── Backup ──

backup:
	mkdir -p backups
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_DIR) && docker compose exec -T db pg_dump -U crawler craw" \
		> backups/craw-$$(date +%F-%H%M).sql

# ── Database ops (workspace-production) ──

deploy-db:
	./deploy-db.sh

db-ps:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"cd $(DB_REMOTE_DIR) && docker compose ps"

db-logs:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker logs -f --tail=100 craw-db"

db-backup:
	mkdir -p backups
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker exec craw-db pg_dump -U crawler craw" \
		> backups/craw-db-$$(date +%F-%H%M).sql

db-status:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker exec craw-db psql -U crawler -d craw -c \"SELECT name, setting FROM pg_settings WHERE name IN ('shared_buffers','effective_cache_size','synchronous_commit','max_connections') ORDER BY name;\""

# ── Cleanup stale artifacts on remote ──

clean-remote:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd /home/ubuntu && rm -rf craw-stack-old && \
		 cd $(REMOTE_DIR) && rm -f craw craw.prev craw.prev.broken craw.prev.x86 \
		 craw.golden craw-golden craw-new craw-old craw-current \
		 craw.current-backup craw.extidfix-running-backup \
		 craw_trace.log session-*.md"
