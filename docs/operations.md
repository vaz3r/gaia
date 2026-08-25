# Operations

## Config-only restart

Live tuning requires no code change or image rebuild. The crawler config is
bind-mounted from `apps/crawler/config/` into the container at
`/etc/crawler/config` (read-only). A TOML edit + container recreate is enough.

Precedence (highest wins):
1. `CRAW_*` env vars set in `deploy/compose/docker-compose.yml` / `.env`
2. `{CRAW_PROFILE}.toml` profile file (default profile: `production`)
3. `default.toml`
4. built-in defaults in `src/config.rs`

### Steps

```bash
# 1. Edit the target profile (usually production.toml)
#    apps/crawler/config/production.toml

# 2. Commit and push
git add apps/crawler/config/production.toml && git commit -m "..." && git push

# 3. Pull on the host so the bind-mounted config updates
ssh zerone "cd /home/ubuntu/gaia && git pull"

# 4. Restart with the running image tag (no rebuild)
./deploy/scripts/config-restart.sh zerone
#    or: make restart-config

# 5. Confirm the resolved value in the startup log
ssh zerone "ls -t /home/ubuntu/gaia-data/logs/*.jsonl | head -1 | \
    xargs grep 'effective config' | tail -1"
```

### Notes

- `config-restart.sh` reuses the **running** image tag — it never rebuilds, and
  it never assumes the remote git HEAD matches a built image.
- Full redeploy (code changes + rebuild, ~8 min on ARM64): use
  `./deploy/scripts/deploy.sh` or `make deploy`.
- Env vars always win over TOML. Keys that are env-overridden in
  `docker-compose.yml` (e.g. `source_deadline_ms` via `CRAW_SOURCE_DEADLINE_MS`)
  cannot be tuned by editing TOML alone — remove the env override first.
- Do not commit secrets. `PG_PASSWORD` / `DATABASE_URL` live only in `.env`.