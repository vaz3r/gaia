COMPOSE ?= docker compose
STACK  ?= /home/ubuntu/craw-stack

compose:
	$(COMPOSE) --project-directory $(STACK) --env-file $(STACK)/.env -f $(STACK)/docker-compose.yml $(CMD)

bootstrap:
	docker volume create craw-pg-data || true
	docker volume create dht-crawler-data || true

up: bootstrap
	$(MAKE) compose CMD="up -d"

build:
	$(MAKE) compose CMD="up -d --build"

down:
	$(MAKE) compose CMD="down"

restart: up
	$(MAKE) compose CMD="restart"

ps:
	$(MAKE) compose CMD="ps"

logs:
	$(MAKE) compose CMD="logs -f --tail=100"

backup:
	mkdir -p backups
	$(COMPOSE) --project-directory $(STACK) --env-file $(STACK)/.env -f $(STACK)/docker-compose.yml exec -T db \
		pg_dump -U crawler craw > backups/craw-$$(date +%F-%H%M).sql

.PHONY: compose bootstrap up build down restart ps logs backup