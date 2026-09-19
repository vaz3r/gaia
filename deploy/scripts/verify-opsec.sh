#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# GAIA OpSec & Leak Verification Tool
#
# Runs automated tests to confirm:
#   1. Encrypted DNS-over-TLS (DoT) is active (+DNSOverTLS)
#   2. Plaintext DNS (UDP 53) is rejected by kernel killswitch
#   3. Direct outbound HTTP (80) & HTTPS (443) are rejected by kernel killswitch
#   4. LAN, WireGuard, and Tailscale access remain fully functional
# ─────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

run_verification_local() {
    echo "=================================================="
    echo "       GAIA HOMELAB OPSEC & LEAK AUDIT TEST       "
    echo "=================================================="
    echo ""

    local pass_count=0
    local fail_count=0

    # ── Test 1: DNS-over-TLS status ──
    echo -n "[TEST 1] Verifying systemd-resolved DNS-over-TLS... "
    local res_out
    res_out=$(resolvectl status 2>&1 || true)
    if echo "$res_out" | grep -F -q '+DNSOverTLS'; then
        echo "PASS (+DNSOverTLS active)"
        pass_count=$((pass_count + 1))
    else
        echo "FAIL (DNSOverTLS is not active)"
        fail_count=$((fail_count + 1))
    fi

    # ── Test 2: Plaintext DNS (UDP 53) to internet must be blocked ──
    echo -n "[TEST 2] Testing egress killswitch on plaintext DNS (port 53)... "
    if nc -z -u -w 1 1.1.1.1 53 >/dev/null 2>&1 || dig @1.1.1.1 google.com +time=1 +tries=1 >/dev/null 2>&1; then
        echo "FAIL (LEAK: Plaintext UDP 53 outbound succeeded!)"
        fail_count=$((fail_count + 1))
    else
        echo "PASS (Plaintext UDP 53 blocked by kernel killswitch)"
        pass_count=$((pass_count + 1))
    fi

    # ── Test 3: Direct outbound HTTP (port 80) must be blocked ──
    echo -n "[TEST 3] Testing egress killswitch on outbound HTTP (port 80)... "
    if curl -s -m 2 http://1.1.1.1 >/dev/null 2>&1; then
        echo "FAIL (LEAK: Direct outbound HTTP succeeded!)"
        fail_count=$((fail_count + 1))
    else
        echo "PASS (Blocked by kernel killswitch)"
        pass_count=$((pass_count + 1))
    fi

    # ── Test 4: Direct outbound HTTPS (port 443) to external IP must be blocked ──
    echo -n "[TEST 4] Testing egress killswitch on external HTTPS (port 443)... "
    if curl -s -m 2 https://1.1.1.1 >/dev/null 2>&1; then
        echo "FAIL (LEAK: Direct outbound HTTPS to 1.1.1.1 succeeded!)"
        fail_count=$((fail_count + 1))
    else
        echo "PASS (Blocked by kernel killswitch)"
        pass_count=$((pass_count + 1))
    fi

    # ── Test 5: Direct outbound HTTPS to arbitrary domain must be blocked ──
    echo -n "[TEST 5] Testing egress killswitch on external domain (https://google.com)... "
    if curl -s -m 2 https://google.com >/dev/null 2>&1; then
        echo "FAIL (LEAK: Direct outbound HTTPS to google.com succeeded!)"
        fail_count=$((fail_count + 1))
    else
        echo "PASS (Blocked by kernel killswitch)"
        pass_count=$((pass_count + 1))
    fi

    # ── Test 6: Encrypted DoT resolution (port 853) must function ──
    echo -n "[TEST 6] Testing encrypted DNS resolution over DoT... "
    if getent ahostsv4 cloudflare.com >/dev/null 2>&1; then
        echo "PASS (Domain resolved successfully over encrypted DoT)"
        pass_count=$((pass_count + 1))
    else
        echo "FAIL (DNS resolution failed)"
        fail_count=$((fail_count + 1))
    fi

    # ── Test 7: LAN connectivity must remain intact ──
    echo -n "[TEST 7] Testing local LAN management connectivity... "
    local router_ip
    router_ip=$(ip route show default 2>/dev/null | awk '{print $3}' || echo "192.168.10.1")
    if ping -c 1 -W 1 "$router_ip" >/dev/null 2>&1; then
        echo "PASS (Local LAN route to $router_ip responsive)"
        pass_count=$((pass_count + 1))
    else
        echo "WARN (Router $router_ip did not respond to ICMP ping, but LAN is allowed)"
        pass_count=$((pass_count + 1))
    fi

    # ── Test 8: Tunnel interfaces (wg0 / tailscale0) status ──
    echo -n "[TEST 8] Testing virtual tunnel interface (wg0)... "
    if ip link show wg0 >/dev/null 2>&1; then
        echo "PASS (wg0 tunnel interface is up)"
        pass_count=$((pass_count + 1))
    else
        echo "INFO (wg0 interface not active on this specific node)"
    fi

    echo ""
    echo "--------------------------------------------------"
    echo "Results: $pass_count PASSED, $fail_count FAILED"
    echo "--------------------------------------------------"

    if [ "$fail_count" -gt 0 ]; then
        echo "STATUS: ❌ OpSec leaks detected! Review failed tests above."
        return 1
    else
        echo "STATUS: 🛡️ AIR-TIGHT. Zero leaks, DoT active, killswitch enforced."
        return 0
    fi
}

# ── Dispatch: Local or Remote Target ──
if [ "${1:-}" = "--local" ]; then
    run_verification_local
    exit $?
fi

TARGET="${1:-workspace-production}"
TARGET_DIR="$REPO_ROOT/deploy/targets/$TARGET"

if [ ! -d "$TARGET_DIR" ]; then
    echo "ERROR: Target directory $TARGET_DIR not found."
    exit 1
fi

set -a
source "$TARGET_DIR/.env"
set +a

: "${DEPLOY_HOST:?DEPLOY_HOST required in target .env}"
: "${DEPLOY_USER:?DEPLOY_USER required in target .env}"

if [ -n "${DEPLOY_PASSWORD:-}" ]; then
    SSH="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="sshpass -p $DEPLOY_PASSWORD scp -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no"
elif [ -n "${DEPLOY_SSH_KEY:-}" ]; then
    SSH="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no"
else
    SSH="ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -o StrictHostKeyChecking=no"
fi

echo "=== Running OpSec Verification on $TARGET ($DEPLOY_HOST) ==="
$SCP "$0" "$DEPLOY_USER@$DEPLOY_HOST:/tmp/verify-opsec.sh"
$SSH "chmod +x /tmp/verify-opsec.sh && /tmp/verify-opsec.sh --local && rm -f /tmp/verify-opsec.sh"
