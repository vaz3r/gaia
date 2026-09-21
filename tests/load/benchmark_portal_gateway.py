#!/usr/bin/env python3
"""
Python Asynchronous Load Testing and Capacity Benchmark for GAIA Portal via Gateway.
Zero external dependencies (uses standard library: asyncio, ssl, urllib, time, statistics).
"""

import sys
import os
import time
import json
import random
import ssl
import asyncio
import argparse
import statistics
from urllib.parse import urlencode, quote

SEARCH_TERMS = [
    'ubuntu', 'linux', '1080p', '2024', 'remux',
    'flac', 'lossless', 's01', 'x265', 'hevc',
    'season', '720p', 'complete', 'soundtrack', 'iso'
]

CATEGORIES = [
    'Movies', 'Television', 'Anime', 'Games',
    'Music', 'Applications', 'Books & Learning', 'Documentaries'
]

class BenchmarkMetrics:
    def __init__(self):
        self.latencies = {}  # endpoint_name -> list of floats (ms)
        self.status_counts = {}  # endpoint_name -> status_code -> count
        self.total_requests = 0
        self.total_errors = 0
        self.start_time = 0.0
        self.end_time = 0.0

    def record(self, endpoint, status_code, duration_ms):
        self.total_requests += 1
        if status_code >= 400 or status_code == 0:
            self.total_errors += 1

        if endpoint not in self.latencies:
            self.latencies[endpoint] = []
            self.status_counts[endpoint] = {}

        self.latencies[endpoint].append(duration_ms)
        self.status_counts[endpoint][status_code] = self.status_counts[endpoint].get(status_code, 0) + 1

    def print_summary(self):
        elapsed = max(0.001, self.end_time - self.start_time)
        overall_rps = self.total_requests / elapsed

        print("\n" + "=" * 90)
        print("  GAIA PORTAL GATEWAY CAPACITY BENCHMARK RESULTS")
        print("=" * 90)
        print(f"Total Duration : {elapsed:.2f}s")
        print(f"Total Requests : {self.total_requests:,}")
        print(f"Total Errors   : {self.total_errors:,} ({self.total_errors / max(1, self.total_requests) * 100:.2f}%)")
        print(f"Throughput     : {overall_rps:.1f} req/s\n")

        print(f"{'Endpoint / Scenario':<26} | {'Reqs':>6} | {'p50':>7} | {'p90':>7} | {'p95':>7} | {'p99':>7} | {'Avg':>7} | {'Err':>5}")
        print("-" * 90)

        all_latencies = []
        for ep, lats in sorted(self.latencies.items()):
            all_latencies.extend(lats)
            lats.sort()
            n = len(lats)
            p50 = lats[int(n * 0.50)]
            p90 = lats[int(n * 0.90)]
            p95 = lats[int(n * 0.95)]
            p99 = lats[int(n * 0.99)]
            avg = statistics.mean(lats)
            err_count = sum(cnt for code, cnt in self.status_counts[ep].items() if code >= 400 or code == 0)

            print(f"{ep:<26} | {n:>6} | {p50:>6.1f}ms | {p90:>6.1f}ms | {p95:>6.1f}ms | {p99:>6.1f}ms | {avg:>6.1f}ms | {err_count:>5}")

        print("-" * 90)
        if all_latencies:
            all_latencies.sort()
            N = len(all_latencies)
            print(f"{'OVERALL ALL ENDPOINTS':<26} | {N:>6} | {all_latencies[int(N*0.50)]:>6.1f}ms | {all_latencies[int(N*0.90)]:>6.1f}ms | {all_latencies[int(N*0.95)]:>6.1f}ms | {all_latencies[int(N*0.99)]:>6.1f}ms | {statistics.mean(all_latencies):>6.1f}ms | {self.total_errors:>5}")
        print("=" * 90 + "\n")


async def http_get(ssl_ctx, host, port, path, headers=None, timeout=10.0):
    t0 = time.perf_counter()
    status_code = 0
    body = b""
    try:
        reader, writer = await asyncio.wait_for(
            asyncio.open_connection(host, port, ssl=ssl_ctx, server_hostname="gaia-gateway"),
            timeout=timeout
        )

        req = f"GET {path} HTTP/1.1\r\nHost: gaia-gateway\r\nUser-Agent: GaiaLoadTester/1.0\r\nConnection: close\r\n"
        if headers:
            for k, v in headers.items():
                req += f"{k}: {v}\r\n"
        req += "\r\n"

        writer.write(req.encode('utf-8'))
        await writer.drain()

        # Read status line
        status_line = await asyncio.wait_for(reader.readline(), timeout=timeout)
        if status_line:
            parts = status_line.decode('latin1').split()
            if len(parts) >= 2 and parts[1].isdigit():
                status_code = int(parts[1])

        # Read remaining body (drain socket)
        while True:
            chunk = await asyncio.wait_for(reader.read(8192), timeout=timeout)
            if not chunk:
                break
            body += chunk

        writer.close()
        await writer.wait_closed()
    except Exception as e:
        status_code = 0
    dt_ms = (time.perf_counter() - t0) * 1000.0
    return status_code, dt_ms, body


async def virtual_user_worker(worker_id, target_ip, target_port, ssl_ctx, metrics, sample_hashes, stop_event):
    while not stop_event.is_set():
        rand = random.random()

        # 1. Search Query (40%)
        if rand < 0.40:
            term = random.choice(SEARCH_TERMS)
            page = 2 if random.random() < 0.3 else 1
            path = f"/api/torrents?search={quote(term)}&page={page}&limit=25"
            status, dt, _ = await http_get(ssl_ctx, target_ip, target_port, path)
            metrics.record("API: Search", status, dt)
            await asyncio.sleep(random.uniform(0.5, 1.5))

        # 2. Homepage & Stats (30%)
        elif rand < 0.70:
            # Home
            status_h, dt_h, _ = await http_get(ssl_ctx, target_ip, target_port, "/")
            metrics.record("Page: Home HTML", status_h, dt_h)

            # Stats
            status_s, dt_s, _ = await http_get(ssl_ctx, target_ip, target_port, "/api/stats")
            metrics.record("API: Stats", status_s, dt_s)

            # Recent
            status_r, dt_r, _ = await http_get(ssl_ctx, target_ip, target_port, "/api/torrents?page=1&limit=25&sort=verified_at&order=desc")
            metrics.record("API: Recent Torrents", status_r, dt_r)
            await asyncio.sleep(random.uniform(1.0, 2.0))

        # 3. Category Filter (20%)
        elif rand < 0.90:
            cat = random.choice(CATEGORIES)
            path = f"/api/torrents?category={quote(cat)}&page=1&limit=25"
            status, dt, _ = await http_get(ssl_ctx, target_ip, target_port, path)
            metrics.record("API: Category Filter", status, dt)
            await asyncio.sleep(random.uniform(0.8, 1.8))

        # 4. Detail Inspection (10%)
        else:
            ih = random.choice(sample_hashes) if sample_hashes else "5753ddc7834817e283903861dba64a83a8fb1a4b"
            path = f"/api/torrents/{ih}"
            status, dt, _ = await http_get(ssl_ctx, target_ip, target_port, path)
            metrics.record("API: Torrent Details", status, dt)
            await asyncio.sleep(random.uniform(1.0, 2.5))


async def run_benchmark(target_ip="192.168.10.111", target_port=443, vus=25, duration=30):
    print(f"\n[+] Preparing load test against https://gaia-gateway ({target_ip}:{target_port})")
    print(f"[+] Concurrency: {vus} Virtual Users | Duration: {duration}s\n")

    ssl_ctx = ssl.create_default_context()
    ssl_ctx.check_hostname = False
    ssl_ctx.verify_mode = ssl.CERT_NONE

    # Pre-fetch sample infohashes from recent listing
    print("[*] Fetching live infohashes from portal...")
    sample_hashes = []
    try:
        code, dt, body = await http_get(ssl_ctx, target_ip, target_port, "/api/torrents?page=1&limit=50")
        if code == 200:
            # Find JSON payload in body
            json_start = body.find(b'{"data":')
            if json_start != -1:
                decoded_str = body[json_start:].decode('utf-8', errors='replace')
                data = json.loads(decoded_str, strict=False)
                sample_hashes = [t["infohash"] for t in data.get("data", []) if "infohash" in t]
    except Exception as e:
        print(f"[!] Warning: Could not prefetch hashes: {e}")

    if not sample_hashes:
        sample_hashes = ["5753ddc7834817e283903861dba64a83a8fb1a4b"]
    print(f"[+] Loaded {len(sample_hashes)} seed infohashes for inspection scenario.")

    metrics = BenchmarkMetrics()
    stop_event = asyncio.Event()

    print(f"[+] Launching {vus} simulated user tasks...")
    metrics.start_time = time.time()
    workers = [
        asyncio.create_task(virtual_user_worker(i, target_ip, target_port, ssl_ctx, metrics, sample_hashes, stop_event))
        for i in range(vus)
    ]

    # Progress reporting loop
    for second in range(1, duration + 1):
        await asyncio.sleep(1.0)
        elapsed = time.time() - metrics.start_time
        current_rps = metrics.total_requests / max(0.1, elapsed)
        print(f"\r    [{second:3d}/{duration}s] Req: {metrics.total_requests:5d} | Err: {metrics.total_errors:3d} | RPS: {current_rps:6.1f}", end="", flush=True)

    print("\n[*] Stopping virtual users and collecting metrics...")
    stop_event.set()
    await asyncio.gather(*workers, return_exceptions=True)
    metrics.end_time = time.time()

    metrics.print_summary()
    return metrics


def main():
    parser = argparse.ArgumentParser(description="GAIA Gateway Capacity Load Tester")
    parser.add_argument("--ip", default="192.168.10.111", help="Gateway IP (default: 192.168.10.111)")
    parser.add_argument("--port", type=int, default=443, help="Gateway port (default: 443)")
    parser.add_argument("--vus", type=int, default=25, help="Number of concurrent virtual users (default: 25)")
    parser.add_argument("--duration", type=int, default=30, help="Test duration in seconds (default: 30)")
    args = parser.parse_args()

    asyncio.run(run_benchmark(target_ip=args.ip, target_port=args.port, vus=args.vus, duration=args.duration))


if __name__ == "__main__":
    main()
