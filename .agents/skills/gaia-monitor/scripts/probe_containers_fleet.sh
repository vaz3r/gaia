#!/usr/bin/env bash
# probe_containers_fleet.sh — Inspects all containers across all 4 GAIA deployment targets
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

FORMAT="text"
TARGET_FILTER="all"

while [ $# -gt 0 ]; do
    case "$1" in
        --json) FORMAT="json" ;;
        --target) TARGET_FILTER="$2"; shift ;;
        *) ;;
    esac
    shift
done

RESULTS_TMP=$(mktemp)
trap 'rm -f "$RESULTS_TMP"' EXIT

# Helper for container inspect and stats
# Format: Target\tContainerName\tStatus\tHealth\tRestarts\tCPUPerc\tMemUsage
get_local_containers() {
    local target="workspace-production"
    # Get stats
    local stats_file=$(mktemp)
    timeout 15 docker stats --no-stream --format "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}" > "$stats_file" 2>/dev/null || true

    local c_ids
    c_ids=$(docker ps -aq 2>/dev/null || true)
    if [ -n "$c_ids" ]; then
        docker inspect $c_ids --format '{{.Name}}	{{.State.Status}}	{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}	{{.RestartCount}}' 2>/dev/null | while IFS=$'\t' read -r raw_name state health restarts; do
            local name="${raw_name#/}"
            local cpu="0.0%"
            local mem="0B"
            if [ -f "$stats_file" ]; then
                local stat_match=$(grep -E "^${name}\s" "$stats_file" || true)
                if [ -n "$stat_match" ]; then
                    cpu=$(echo "$stat_match" | awk -F'\t' '{print $2}')
                    mem=$(echo "$stat_match" | awk -F'\t' '{print $3}')
                fi
            fi
            printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\n" "$target" "$name" "$state" "$health" "$restarts" "$cpu" "$mem" >> "$RESULTS_TMP.local"
        done
    fi
    rm -f "$stats_file"
}

get_remote_containers() {
    local target="$1"
    local env_file="$REPO_ROOT/deploy/targets/$target/.env"
    [ -f "$env_file" ] || return 0

    local DEPLOY_HOST DEPLOY_USER DEPLOY_PASSWORD DEPLOY_SSH_KEY
    DEPLOY_HOST=$(grep -E '^DEPLOY_HOST=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ')
    DEPLOY_USER=$(grep -E '^DEPLOY_USER=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ')
    DEPLOY_PASSWORD=$(grep -E '^DEPLOY_PASSWORD=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ' || true)
    DEPLOY_SSH_KEY=$(grep -E '^DEPLOY_SSH_KEY=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ' || true)
    eval DEPLOY_SSH_KEY="$DEPLOY_SSH_KEY"

    local SSH_CMD=""
    if [ -n "$DEPLOY_PASSWORD" ]; then
        SSH_CMD="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    elif [ -n "$DEPLOY_SSH_KEY" ]; then
        SSH_CMD="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    else
        SSH_CMD="ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    fi

    # Run remote probe script
    local remote_script='
    stats_file=$(mktemp)
    timeout 15 docker stats --no-stream --format "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}" > "$stats_file" 2>/dev/null || true
    c_ids=$(docker ps -aq 2>/dev/null || true)
    if [ -n "$c_ids" ]; then
        docker inspect $c_ids --format "{{.Name}}	{{.State.Status}}	{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}	{{.RestartCount}}" 2>/dev/null | while IFS="	" read -r raw_name state health restarts; do
            name="${raw_name#/}"
            cpu="0.0%"
            mem="0B"
            if [ -f "$stats_file" ]; then
                stat_match=$(grep -E "^${name}\s" "$stats_file" || true)
                if [ -n "$stat_match" ]; then
                    cpu=$(echo "$stat_match" | awk -F"\t" "{print \$2}")
                    mem=$(echo "$stat_match" | awk -F"\t" "{print \$3}")
                fi
            fi
            printf "%s\t%s\t%s\t%s\t%s\t%s\n" "$name" "$state" "$health" "$restarts" "$cpu" "$mem"
        done
    fi
    rm -f "$stats_file"
    '

    local out
    out=$($SSH_CMD "$remote_script" 2>/dev/null || true)
    while IFS=$'\t' read -r name state health restarts cpu mem; do
        [ -n "$name" ] && printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\n" "$target" "$name" "$state" "$health" "$restarts" "$cpu" "$mem" >> "$RESULTS_TMP.$target"
    done <<< "$out"
}

# Collect based on filter concurrently
if [ "$TARGET_FILTER" == "all" ]; then
    get_local_containers &
    get_remote_containers "gaia-gateway" &
    get_remote_containers "gaia-portal" &
    get_remote_containers "gaia-node" &
    wait
    cat "$RESULTS_TMP.local" "$RESULTS_TMP.gaia-gateway" "$RESULTS_TMP.gaia-portal" "$RESULTS_TMP.gaia-node" >> "$RESULTS_TMP" 2>/dev/null || true
    rm -f "$RESULTS_TMP.local" "$RESULTS_TMP.gaia-gateway" "$RESULTS_TMP.gaia-portal" "$RESULTS_TMP.gaia-node"
else
    if [ "$TARGET_FILTER" == "workspace-production" ]; then
        get_local_containers
        cat "$RESULTS_TMP.local" >> "$RESULTS_TMP" 2>/dev/null || true
        rm -f "$RESULTS_TMP.local"
    else
        get_remote_containers "$TARGET_FILTER"
        cat "$RESULTS_TMP.$TARGET_FILTER" >> "$RESULTS_TMP" 2>/dev/null || true
        rm -f "$RESULTS_TMP.$TARGET_FILTER"
    fi
fi

if [[ "$FORMAT" == "json" ]]; then
    python3 -c "
import json, sys

containers = []
with open('$RESULTS_TMP') as f:
    for line in f:
        line = line.strip()
        if not line: continue
        parts = line.split('\t')
        if len(parts) >= 7:
            containers.append({
                'target': parts[0],
                'name': parts[1],
                'state': parts[2],
                'health': parts[3],
                'restarts': int(parts[4]),
                'cpu_pct': parts[5],
                'mem_usage': parts[6]
            })

print(json.dumps({'total_containers': len(containers), 'containers': containers}, indent=2))
"
else
    echo "=== GAIA CONTAINER FLEET STATUS ==="
    printf "%-22s | %-28s | %-9s | %-11s | %-8s | %-8s | %-16s\n" "Target" "Container Name" "State" "Health" "Restarts" "CPU %" "Memory"
    printf "%s\n" "------------------------------------------------------------------------------------------------------------------------"
    while IFS=$'\t' read -r target name state health restarts cpu mem; do
        [ -n "$target" ] && printf "%-22s | %-28s | %-9s | %-11s | %-8s | %-8s | %-16s\n" "$target" "$name" "$state" "$health" "$restarts" "$cpu" "$mem"
    done < "$RESULTS_TMP"
fi
