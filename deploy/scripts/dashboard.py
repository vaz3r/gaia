#!/usr/bin/env python3
import urllib.request
import json
import sys

HOST = "100.87.194.112"
PORT = 3000

def fetch(path):
    url = f"http://{HOST}:{PORT}{path}"
    try:
        with urllib.request.urlopen(url) as response:
            return json.loads(response.read().decode())
    except Exception as e:
        print(f"Error fetching {url}: {e}")
        sys.exit(1)

def format_num(n):
    if n is None: return "—"
    if n >= 1_000_000: return f"{n/1_000_000:.1f}M"
    if n >= 1_000: return f"{n/1000:.1f}k"
    if isinstance(n, float): return f"{n:.1f}"
    return str(n)

stats = fetch("/api/stats")
metrics = fetch("/api/metrics/current")
rates = metrics.get("rates", {})

print("=== GAIA DASHBOARD (LIVE) ===")
print(f"Total verified:   {format_num(stats.get('total_torrents'))}")
print(f"Verified 24h:     {format_num(stats.get('verified_last_24h'))}")
print(f"Verified 1h:      {format_num(stats.get('verified_last_1h'))}")
print(f"Unique infohashes:{format_num(stats.get('seen_last_1h'))}")
print(f"New infohashes:   {format_num(stats.get('new_last_1h'))}")
print(f"Queue backlog:    {stats.get('queue_backlog')} ({stats.get('verifying')} verifying)")
print(f"Session uptime:   {stats.get('session_uptime_s')}s")
print()

print("--- THROUGHPUT RATES (/hr) ---")
print(f"Verified /hr:         {format_num(rates.get('verify_success', 0))}/hr")
print(f"Fetch attempts /hr:   {format_num(rates.get('fetch_attempts', 0))}/hr")
print(f"Failures /hr:         {format_num(rates.get('verify_fail', 0))}/hr")
print(f"Inbound get_peers:    {format_num(rates.get('inbound_get_peers_bep42', 0))}/hr")
print(f"Source queries:       {format_num(rates.get('source_queries', 0))}/hr")
print(f"Source timeouts:      {format_num(rates.get('source_timeout', 0))}/hr")
print(f"Source cache filters: {format_num(rates.get('source_filtered_by_cache', 0))}/hr")

print("\n--- CONNECTIONS ---")
tcp_attempts = rates.get('tcp_attempts', 1) or 1
utp_attempts = rates.get('utp_attempts', 1) or 1
print(f"TCP attempts: {format_num(tcp_attempts)}/hr  ({rates.get('tcp_connect_ok', 0) / tcp_attempts * 100:.1f}% ok)")
print(f"uTP attempts: {format_num(utp_attempts)}/hr  ({rates.get('utp_connect_ok', 0) / utp_attempts * 100:.1f}% ok)")
