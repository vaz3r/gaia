#!/usr/bin/env bash
# Run as root (Docker ENTRYPOINT).
#
# 1. Fix ownership of the persisted named volume (created as root by Docker) so
#    the non-root `crawler` user can write the DB + routing state.
# 2. Point /etc/resolv.conf at Docker's embedded DNS (127.0.0.11). The crawler
#    shares gluetun's network namespace, whose resolv.conf targets gluetun's
#    own DNS proxy (127.0.0.1) — that proxy only resolves tunnel/public names,
#    not compose service names. Docker's embedded DNS resolves redis/postgres.
# 3. Drop privileges via gosu and exec the crawler.
set -euo pipefail

chown -R crawler:crawler /data

# Docker's embedded DNS lives at 127.0.0.11 on every user-defined network; the
# crawler shares gluetun's netns so it is reachable from here. Override the
# inherited resolv.conf so `redis` / `postgres` resolve by hostname.
printf 'nameserver 127.0.0.11\noptions ndots:0\n' > /etc/resolv.conf

exec gosu crawler crawler "$@"
