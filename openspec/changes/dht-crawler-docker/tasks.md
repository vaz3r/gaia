# Tasks

## 1. Containerize the crawler (D24)

- [ ] 1.1 Write `dht-crawler/Dockerfile` (multi-stage rust build → slim runtime)
- [ ] 1.2 Verify the image builds and the binary runs `--help`

## 2. Compose stack with Gluetun (D25, D26)

- [ ] 2.1 Write `dht-crawler/docker-compose.yml` (gluetun + crawler, network_mode service:gluetun)
- [ ] 2.2 Set `FIREWALL_VPN_INPUT_PORTS=6881,6882,6883,6884`
- [ ] 2.3 Add a Gluetun healthcheck; crawler `depends_on` gluetun healthy

## 3. Secrets + data (D27)

- [ ] 3.1 Create gitignored `dht-crawler/.env` with WireGuard secrets
- [ ] 3.2 Update `.gitignore` for `dht-crawler/.env`, `dht-crawler/data/`, `dht-crawler/gluetun/`
- [ ] 3.3 Migrate `crawler.sqlite` + `state/` into `dht-crawler/data/`

## 4. Deploy + verify

- [ ] 4.1 Stop pm2 crawler
- [ ] 4.2 `docker compose up -d --build`
- [ ] 4.3 Verify tunnel up + egress IP is `132.145.189.201`
- [ ] 4.4 Verify 4 instances, stats flowing, verify rate rising

## 5. Docs + commit

- [ ] 5.1 README: docker compose usage
- [ ] 5.2 Commit the change set
