HOST      ?= zerone
DB_HOST   ?= workspace-production
TAG       := $(shell git rev-parse --short HEAD)
REMOTE    ?= ubuntu@$(HOST)
DB_REMOTE ?= core@$(DB_HOST)
SSH_KEY   ?= $(HOME)/.ssh/zerone
REMOTE_GIT ?= /home/ubuntu/gaia
REMOTE_COMPOSE ?= $(REMOTE_GIT)/deploy/compose
DB_REMOTE_DIR ?= /home/core/gaia

COMPOSE_FILE ?= deploy/compose/docker-compose.yml
COMPOSE_DEV  ?= deploy/compose/docker-compose.dev.yml

.PHONY: dev dev-down deploy deploy-db rollback ps logs backup \
        db-ps db-logs db-backup db-status init-remote clean-remote

# ── Local development ──

dev:
	docker compose --env-file .env -f $(COMPOSE_DEV) up -d

dev-down:
	docker compose --env-file .env -f $(COMPOSE_DEV) down

# ── Production deploy ──

deploy:
	./deploy/scripts/deploy.sh $(HOST) HEAD

rollback:
	./deploy/scripts/deploy.sh $(HOST) $(ROLLBACK_TAG)

# ── Production ops ──

ps:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_COMPOSE) && docker compose --env-file $(REMOTE_GIT)/.env ps"

logs:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_COMPOSE) && docker compose --env-file $(REMOTE_GIT)/.env logs -f --tail=100 crawler dashboard"

# ── One-time remote setup ──

init-remote:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd /home/ubuntu && \
		 git clone $(shell git remote get-url origin) gaia || true && \
		 cd gaia && git fetch origin"

# ── Backup ──

backup:
	mkdir -p backups
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd $(REMOTE_COMPOSE) && docker compose --env-file $(REMOTE_GIT)/.env exec -T crawler pg_dump -U crawler craw" \
		> backups/gaia-$$$(date +%F-%H%M).sql

# ── Database ops (workspace-production) ──

deploy-db:
	./deploy/scripts/db-init.sh

db-ps:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"cd $(DB_REMOTE_DIR)/deploy/compose && docker compose ps"

db-logs:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker logs -f --tail=100 craw-db"

db-backup:
	mkdir -p backups
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker exec craw-db pg_dump -U crawler craw" \
		> backups/gaia-db-$$$(date +%F-%H%M).sql

db-status:
	ssh -o StrictHostKeyChecking=no $(DB_REMOTE) \
		"docker exec craw-db psql -U crawler -d craw -c \"SELECT name, setting FROM pg_settings WHERE name IN ('shared_buffers','effective_cache_size','synchronous_commit','max_connections') ORDER BY name;\""
