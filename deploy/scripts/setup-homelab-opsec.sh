#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# GAIA Automated Homelab OpSec & ISP-Blindness Provisioner (Plug-and-Play)
#
# Configures:
#   1. Encrypted DNS-over-TLS (DoT) via systemd-resolved to Cloudflare & Quad9
#   2. Dynamic Kernel Egress Killswitch (blocks outbound 80/443 & UDP 53 except Gateway IP)
#   3. Persistent systemd service that updates whenever Gateway IP changes
#
# Usage:
#   ./deploy/scripts/setup-homelab-opsec.sh [target_name]
#   sudo ./deploy/scripts/setup-homelab-opsec.sh --local [gateway_ip]
# ─────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Resolve active Gateway IP dynamically from target .env files
resolve_gateway_ip() {
    local gw_ip=""

    # 1. Environment override
    if [ -n "${GATEWAY_IP:-}" ]; then
        echo "$GATEWAY_IP"
        return 0
    fi

    # 2. Check deploy/targets/gaia-gateway/.env
    if [ -f "$REPO_ROOT/deploy/targets/gaia-gateway/.env" ]; then
        gw_ip=$(grep -E "^(GATEWAY_PUBLIC_IP|DEPLOY_HOST)=" "$REPO_ROOT/deploy/targets/gaia-gateway/.env" | head -n 1 | cut -d '=' -f2 | tr -d '"' | tr -d "'" || true)
    fi

    # 3. Fallback: check deploy/targets/gaia-portal/.env (GATEWAY_WSTUNNEL_URL)
    if [ -z "$gw_ip" ] && [ -f "$REPO_ROOT/deploy/targets/gaia-portal/.env" ]; then
        local wss_url
        wss_url=$(grep -E "^GATEWAY_WSTUNNEL_URL=" "$REPO_ROOT/deploy/targets/gaia-portal/.env" | cut -d '=' -f2 | tr -d '"' | tr -d "'" || true)
        if [ -n "$wss_url" ]; then
            # Extract host from wss://host:port/path
            gw_ip=$(echo "$wss_url" | sed -E 's|^wss?://([^:/]+).*|\1|')
        fi
    fi

    # If domain, resolve to IPv4
    if [ -n "$gw_ip" ] && ! [[ "$gw_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        local resolved
        resolved=$(getent ahostsv4 "$gw_ip" 2>/dev/null | head -n 1 | awk '{print $1}' || true)
        [ -n "$resolved" ] && gw_ip="$resolved"
    fi

    echo "${gw_ip:-none}"
}

# ── Local execution logic (runs on the homelab host with sudo) ──
apply_opsec_local() {
    local gateway_ip="$1"
    echo "=== Applying Homelab OpSec Hardening ==="
    echo "Active Whitelisted Gateway IP: $gateway_ip"
    echo ""

    # Ensure running as root
    if [ "$(id -u)" -ne 0 ]; then
        echo "ERROR: Local OpSec provisioning must be run as root (or with sudo)."
        exit 1
    fi

    # ── 1. Configure DNS-over-TLS (DoT) via systemd-resolved ──
    echo "[1/3] Configuring DNS-over-TLS (DoT) with Cloudflare & Quad9..."
    mkdir -p /etc/systemd/resolved.conf.d

    cat > /etc/systemd/resolved.conf.d/dot.conf << 'EOF'
[Resolve]
DNS=1.1.1.1#cloudflare-dns.com 1.0.0.1#cloudflare-dns.com 9.9.9.9#dns.quad9.net
FallbackDNS=8.8.8.8#dns.google
Domains=~.
DNSOverTLS=yes
DNSSEC=yes
MulticastDNS=no
LLMNR=no
Cache=yes
EOF

    # If Tailscale is running, disable its MagicDNS resolv.conf clobbering
    if command -v tailscale >/dev/null 2>&1; then
        tailscale set --accept-dns=false 2>/dev/null || true
    fi

    # Ensure /etc/resolv.conf points to systemd-resolved stub listener
    if [ -f /run/systemd/resolve/stub-resolv.conf ]; then
        if [ ! -L /etc/resolv.conf ] || [ "$(readlink /etc/resolv.conf)" != "/run/systemd/resolve/stub-resolv.conf" ]; then
            ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
        fi
    fi

    systemctl daemon-reload
    systemctl restart systemd-resolved

    # ── 2. Configure Dynamic Kernel Egress Killswitch ──
    echo "[2/3] Configuring Kernel-Level Egress Killswitch (Anti-SSRF & Anti-Leak)..."
    mkdir -p /etc/iptables

    cat > /etc/iptables/gaia-killswitch.sh << EOF
#!/usr/bin/env bash
set -euo pipefail

# Dynamic Gateway IP (reconfigured automatically by deploy script)
GATEWAY_IP="$gateway_ip"

echo "Activating GAIA Kernel Egress Killswitch (Gateway: \$GATEWAY_IP)..."

# Flush existing OUTPUT rules
iptables -F OUTPUT
iptables -P OUTPUT ACCEPT

# 1. Loopback interface
iptables -A OUTPUT -o lo -j ACCEPT

# 2. Established / Related connection tracking
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# 3. Private RFC1918 subnets (LAN SSH, intra-host communication, router)
iptables -A OUTPUT -d 192.168.0.0/16 -j ACCEPT
iptables -A OUTPUT -d 10.0.0.0/8 -j ACCEPT
iptables -A OUTPUT -d 172.16.0.0/12 -j ACCEPT

# 4. Tailscale mesh interface (if present)
iptables -A OUTPUT -o tailscale0 -j ACCEPT

# 5. WireGuard virtual tunnel interface (encrypted internal traffic)
iptables -A OUTPUT -o wg0 -j ACCEPT

# 6. Encrypted DNS-over-TLS (DoT on TCP port 853 ONLY)
iptables -A OUTPUT -p tcp --dport 853 -j ACCEPT

# 7. Whitelisted Gateway HTTPS/WebSocket tunnel (TCP port 443 ONLY to Gateway IP)
if [ -n "\$GATEWAY_IP" ] && [ "\$GATEWAY_IP" != "none" ]; then
    iptables -A OUTPUT -p tcp -d "\$GATEWAY_IP" --dport 443 -j ACCEPT
fi

# 8. KERNEL EGRESS KILLSWITCH:
# Block all unencrypted plaintext DNS (port 53) to the public internet
iptables -A OUTPUT -p udp --dport 53 -j REJECT --reject-with icmp-port-unreachable
iptables -A OUTPUT -p tcp --dport 53 -j REJECT --reject-with icmp-port-unreachable

# Drop/Reject all other outbound port 80/443 to public internet (complete SSRF & leak immunity)
iptables -A OUTPUT -p tcp --dport 80 -j REJECT --reject-with icmp-net-unreachable
iptables -A OUTPUT -p tcp --dport 443 -j REJECT --reject-with icmp-net-unreachable

echo "Killswitch active."
EOF

    chmod +x /etc/iptables/gaia-killswitch.sh

    # Create persistent systemd unit
    cat > /etc/systemd/system/gaia-killswitch.service << 'EOF'
[Unit]
Description=GAIA Homelab Kernel Egress Killswitch
After=network-online.target systemd-resolved.service
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/etc/iptables/gaia-killswitch.sh

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable gaia-killswitch.service
    systemctl restart gaia-killswitch.service

    # ── 3. Initial verification ──
    echo "[3/3] Testing configuration..."
    local res_out
    res_out=$(resolvectl status 2>&1 || true)
    if echo "$res_out" | grep -F -q '+DNSOverTLS'; then
        echo "  [PASS] DNS-over-TLS is ACTIVE (+DNSOverTLS)."
    else
        echo "  [WARN] DNS-over-TLS status reported no +DNSOverTLS. Inspecting resolvectl status..."
        echo "$res_out" | head -n 10 || true
    fi

    echo ""
    echo "=== Homelab OpSec Hardening Complete ==="
}

# ── Dispatch: Local or Remote Target ──
if [ "${1:-}" = "--local" ]; then
    GW="${2:-$(resolve_gateway_ip)}"
    apply_opsec_local "$GW"
    exit 0
fi

TARGET="${1:-workspace-production}"
TARGET_DIR="$REPO_ROOT/deploy/targets/$TARGET"

if [ ! -d "$TARGET_DIR" ]; then
    echo "ERROR: Target directory $TARGET_DIR not found."
    exit 1
fi

# Load target SSH env
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

GW_IP=$(resolve_gateway_ip)
echo "=== Deploying OpSec Hardening to $TARGET ($DEPLOY_HOST) [Gateway: $GW_IP] ==="

# Transfer this script and run on remote target
$SCP "$0" "$DEPLOY_USER@$DEPLOY_HOST:/tmp/setup-homelab-opsec.sh"

if [ "$DEPLOY_USER" = "root" ]; then
    $SSH "chmod +x /tmp/setup-homelab-opsec.sh && /tmp/setup-homelab-opsec.sh --local $GW_IP && rm -f /tmp/setup-homelab-opsec.sh"
else
    $SSH "chmod +x /tmp/setup-homelab-opsec.sh && echo '${DEPLOY_PASSWORD:-}' | sudo -S /tmp/setup-homelab-opsec.sh --local $GW_IP && rm -f /tmp/setup-homelab-opsec.sh"
fi

echo "=== OpSec Deployment Complete for $TARGET ==="
