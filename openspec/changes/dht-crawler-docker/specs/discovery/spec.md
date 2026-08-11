## Purpose

Keeps the crawler's discovery and fetch logic unchanged while moving it to a public-IP egress, so the DHT node is reachable and the verify rate is no longer NAT-bound.

## ADDED Requirements

### Requirement: Unchanged discovery/fetch behavior
Moving to Docker SHALL NOT alter the crawler's discovery or fetch logic; the same binary and configuration run containerized.

#### Scenario: Same runtime behavior
- **WHEN** the containerized crawler starts
- **THEN** it runs the same sampler/grower/fetch pipeline with the same CLI defaults as the pm2 deployment

#### Scenario: Warm routing state reused
- **WHEN** the stack starts
- **THEN** the routing table from the migrated `state/` directory is loaded so the node resumes warm

### Requirement: Reachable DHT node
The crawler SHALL bind its DHT UDP ports inside the Gluetun network namespace so it is reachable on the public IP through the tunnel.

#### Scenario: Node is publicly reachable
- **WHEN** the stack is up and the tunnel is connected
- **THEN** the DHT node answers queries on `132.145.189.201` ports 6881-6884 (subject to the Oracle security list)
