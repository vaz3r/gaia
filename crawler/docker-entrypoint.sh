#!/usr/bin/env bash
# Run as root (Docker ENTRYPOINT): fix ownership of the persisted named volume
# (created as root by Docker) so the non-root `crawler` user can write the DB
# + routing state, then drop privileges via gosu and exec the crawler.
set -euo pipefail

chown -R crawler:crawler /data

exec gosu crawler crawler "$@"
