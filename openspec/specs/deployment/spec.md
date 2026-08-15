# Deployment Specification

## Purpose

Root-level Docker Compose orchestration of the whole platform: gluetun, redis, crawler, postgres, api, and dashboard, with pinned images, healthchecks, log rotation, and non-root containers.

## Requirements

### Requirement: Single compose stack
The platform SHALL be deployed via a root-level `docker-compose.yml` that defines all services and their dependencies, with the crawler running in gluetun's network namespace and all other services on the default compose network.

#### Scenario: Full stack starts
- **WHEN** `docker compose up -d` runs at the repo root
- **THEN** gluetun, redis, postgres, api, and dashboard start, and the crawler starts only after its dependencies are healthy

#### Scenario: Crawler reaches postgres and redis
- **WHEN** the crawler is running
- **THEN** it can reach Redis and Postgres over the compose network without tunnel egress

### Requirement: Pinned, versioned images
All container images SHALL be pinned to a specific version tag (not `latest`).

#### Scenario: Reproducible deployment
- **WHEN** the stack is rebuilt
- **THEN** it uses the pinned image versions, not whatever `latest` resolves to at build time

### Requirement: Healthchecks on all long-running services
Each service SHALL define a healthcheck that reflects its liveness, and dependent services SHALL wait on their dependencies' health.

#### Scenario: Dependency ordering enforced
- **WHEN** Postgres or Redis is unhealthy
- **THEN** the crawler does not start until they become healthy

#### Scenario: Crawler health reflected
- **WHEN** the crawler process is alive and writing
- **THEN** its healthcheck reports healthy

### Requirement: Bounded resources and logs
Services SHALL declare memory limits appropriate to their footprint, log rotation (size and file count caps), and SHALL run as non-root where feasible.

#### Scenario: Logs are bounded
- **WHEN** a service writes logs continuously
- **THEN** log files are rotated at the configured size and capped in count

#### Scenario: Non-root crawler
- **WHEN** the crawler container runs
- **THEN** its main process runs as a non-root user with ownership of its data volume
