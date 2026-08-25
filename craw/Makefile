HOST      ?= zerone
TAG       := $(shell git rev-parse --short HEAD)
REMOTE    ?= ubuntu@$(HOST)
SSH_KEY   ?= zerone
REMOTE_GIT ?= /home/ubuntu/craw-stack
REMOTE_DIR ?= $(REMOTE_GIT)/craw

.PHONY: dev down deploy rollback ps logs backup init-remote clean-remote

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

# ── Cleanup stale artifacts on remote ──

clean-remote:
	ssh -i $(SSH_KEY) -o StrictHostKeyChecking=no $(REMOTE) \
		"cd /home/ubuntu && rm -rf craw-stack-old && \
		 cd $(REMOTE_DIR) && rm -f craw craw.prev craw.prev.broken craw.prev.x86 \
		 craw.golden craw-golden craw-new craw-old craw-current \
		 craw.current-backup craw.extidfix-running-backup \
		 craw_trace.log session-*.md"
