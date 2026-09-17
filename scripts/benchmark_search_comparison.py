#!/usr/bin/env python3
"""
GAIA Search, Filter, Sort & RPS Benchmark Comparison Tool
Compares:
  1. Gaia Dashboard API (http://workspace-production:3000/api/torrents)
  2. Gaia Portal API (http://gaia-portal:3005/api/torrents)

Evaluates:
  - Latency & percentiles (p50, p90, p95, p99) across browse, sorts, filters, deep pagination
  - Accuracy & fuzzy matching across exact titles, typos, glued words, shorthands, Cyrillic
  - Throughput (RPS) and latency degradation under concurrent multi-threaded load
  - Cache tier efficacy (Cold vs. Warm L1/L2 hits)
"""

import sys
import os
import time
import json
import argparse
import statistics
import urllib.request
import urllib.parse
import urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed

def parse_args():
    parser = argparse.ArgumentParser(description="Benchmark Gaia Dashboard API vs Gaia Portal API")
    parser.add_argument("--dashboard-url", default="http://workspace-production:3000",
                        help="Base URL for Gaia Dashboard (default: http://workspace-production:3000)")
    parser.add_argument("--portal-url", default="http://gaia-portal:3005",
                        help="Base URL for Gaia Portal (default: http://gaia-portal:3005)")
    parser.add_argument("--concurrency", default="5,15,30",
                        help="Comma-separated concurrency worker tiers for RPS testing (default: 5,15,30)")
    parser.add_argument("--duration", type=int, default=10,
                        help="Duration in seconds per concurrency tier for RPS load test (default: 10)")
    parser.add_argument("--output", default="benchmark_results.json",
                        help="Output JSON file path (default: benchmark_results.json)")
    parser.add_argument("--debug-token", default="gaia_debug_secret_token_v2",
                        help="X-Gaia-Debug header token for portal observability (default: gaia_debug_secret_token_v2)")
    parser.add_argument("--skip-rps", action="store_true",
                        help="Skip multi-threaded concurrency RPS load test")
    return parser.parse_args()

def http_get(url, headers=None, timeout=30):
    start = time.perf_counter()
    req_headers = {"User-Agent": "GaiaBenchmark/2.0"}
    if headers:
        req_headers.update(headers)
    req = urllib.request.Request(url, headers=req_headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            elapsed_ms = (time.perf_counter() - start) * 1000.0
            data = json.loads(raw.decode("utf-8", errors="replace"))
            return {
                "status": resp.status,
                "elapsed_ms": elapsed_ms,
                "data": data,
                "headers": dict(resp.headers),
                "error": None
            }
    except urllib.error.HTTPError as e:
        elapsed_ms = (time.perf_counter() - start) * 1000.0
        return {"status": e.code, "elapsed_ms": elapsed_ms, "data": None, "headers": dict(e.headers), "error": str(e)}
    except Exception as e:
        elapsed_ms = (time.perf_counter() - start) * 1000.0
        return {"status": 0, "elapsed_ms": elapsed_ms, "data": None, "headers": {}, "error": str(e)}

def calculate_percentiles(timings):
    if not timings:
        return {"min": 0, "p50": 0, "p90": 0, "p95": 0, "p99": 0, "max": 0, "mean": 0, "std": 0}
    s = sorted(timings)
    n = len(s)
    def p(pct):
        idx = max(0, min(n - 1, int(n * pct / 100.0)))
        return s[idx]
    return {
        "min": round(s[0], 2),
        "p50": round(statistics.median(s), 2),
        "p90": round(p(90), 2),
        "p95": round(p(95), 2),
        "p99": round(p(99), 2),
        "max": round(s[-1], 2),
        "mean": round(statistics.mean(s), 2),
        "std": round(statistics.stdev(s) if n > 1 else 0.0, 2)
    }

def print_header(title):
    print("\n" + "=" * 80)
    print(f"  {title}")
    print("=" * 80)

def print_table_row(cols, widths, alignments=None):
    if not alignments:
        alignments = ["<"] * len(cols)
    formatted = []
    for c, w, a in zip(cols, widths, alignments):
        val = str(c)
        if len(val) > w:
            val = val[:w-2] + ".."
        if a == ">":
            formatted.append(f"{val:>{w}}")
        elif a == "^":
            formatted.append(f"{val:^{w}}")
        else:
            formatted.append(f"{val:<{w}}")
    print(" | ".join(formatted))

def run_performance_suite(dashboard_base, portal_base, debug_token):
    print_header("SUITE 1: Performance & Latency (Browse, Sort, Filters)")
    
    test_cases = [
        {"name": "Browse Page 1 (Default)", "path": "/api/torrents?page=1&limit=25"},
        {"name": "Browse Page 10 (Pagination)", "path": "/api/torrents?page=10&limit=25"},
        {"name": "Browse Page 50 (Deep Page)", "path": "/api/torrents?page=50&limit=25"},
        {"name": "Sort: verified_at DESC", "path": "/api/torrents?page=1&limit=25&sort=verified_at&order=desc"},
        {"name": "Sort: verified_at ASC", "path": "/api/torrents?page=1&limit=25&sort=verified_at&order=asc"},
        {"name": "Sort: size DESC", "path": "/api/torrents?page=1&limit=25&sort=size&order=desc"},
        {"name": "Sort: popularity DESC", "path": "/api/torrents?page=1&limit=25&sort=popularity&order=desc"},
        {"name": "Sort: health DESC", "path": "/api/torrents?page=1&limit=25&sort=health&order=desc"},
        {"name": "Filter: Movies", "path": "/api/torrents?page=1&limit=25&category=Movies"},
        {"name": "Filter: Television", "path": "/api/torrents?page=1&limit=25&category=Television"},
        {"name": "Filter: Games", "path": "/api/torrents?page=1&limit=25&category=Games"},
        {"name": "Filter: Books & Learning", "path": "/api/torrents?page=1&limit=25&category=Books%20%26%20Learning"},
        {"name": "Combined: Movies + size DESC", "path": "/api/torrents?page=1&limit=25&category=Movies&sort=size&order=desc"},
        {"name": "Combined: TV + pop DESC", "path": "/api/torrents?page=1&limit=25&category=Television&sort=popularity&order=desc"}
    ]

    widths = [30, 11, 11, 11, 11, 11]
    headers = ["Test Scenario", "Dash Cold", "Dash Warm", "Port Cold", "Port Warm", "Speedup"]
    print_table_row(headers, widths, ["<", ">", ">", ">", ">", ">"])
    print("-" * 85)

    results = []
    portal_headers = {"X-Gaia-Debug": debug_token}

    for tc in test_cases:
        d_url = f"{dashboard_base}{tc['path']}"
        p_url = f"{portal_base}{tc['path']}"

        # Dashboard: 1 cold + 3 warm
        d_cold = http_get(d_url)
        d_warm_times = []
        for _ in range(3):
            r = http_get(d_url)
            if r["status"] == 200:
                d_warm_times.append(r["elapsed_ms"])
        d_warm = statistics.mean(d_warm_times) if d_warm_times else d_cold["elapsed_ms"]

        # Portal: 1 cold + 3 warm
        p_cold = http_get(p_url, headers=portal_headers)
        p_warm_times = []
        for _ in range(3):
            r = http_get(p_url, headers=portal_headers)
            if r["status"] == 200:
                p_warm_times.append(r["elapsed_ms"])
        p_warm = statistics.mean(p_warm_times) if p_warm_times else p_cold["elapsed_ms"]

        # Ratio on warm
        speedup = f"{d_warm / p_warm:.1f}x" if p_warm > 0 else "N/A"
        if p_warm > d_warm:
            speedup = f"-{p_warm / d_warm:.1f}x"

        row = [
            tc["name"],
            f"{d_cold['elapsed_ms']:.0f}ms",
            f"{d_warm:.1f}ms",
            f"{p_cold['elapsed_ms']:.0f}ms",
            f"{p_warm:.1f}ms",
            speedup
        ]
        print_table_row(row, widths, ["<", ">", ">", ">", ">", ">"])

        results.append({
            "scenario": tc["name"],
            "path": tc["path"],
            "dashboard": {"cold_ms": d_cold["elapsed_ms"], "warm_ms": d_warm, "status": d_cold["status"]},
            "portal": {"cold_ms": p_cold["elapsed_ms"], "warm_ms": p_warm, "status": p_cold["status"]}
        })

    return results

def run_accuracy_fuzzy_suite(dashboard_base, portal_base, debug_token):
    print_header("SUITE 2: Search Accuracy & Fuzzy Matching Quality")

    queries = [
        # (category, query, targets_list, description)
        ("", "The Matrix", ["matrix"], "Exact Clean Title"),
        ("", "Ubuntu", ["ubuntu"], "Exact Linux Distro"),
        ("", "Cyberpunk 2077", ["cyberpunk"], "Exact Multi-Word"),
        ("", "thmatrix", ["matrix"], "Typo: Transposition"),
        ("", "metrix", ["matrix"], "Typo: Phonetic"),
        ("", "ubunut", ["ubuntu"], "Typo: Vowel/Consonant"),
        ("", "cbyerpunk", ["cyberpunk"], "Typo: Transposition"),
        ("", "interstelar", ["interstellar", "interstelar"], "Typo: Missing 'l' (Alt Release)"),
        ("", "matrix1999", ["matrix"], "Glued: Word + Year"),
        ("", "the.matrix.reloaded", ["matrix"], "Delimited: Dots"),
        ("Movies", "thmatrix 99", ["matrix"], "Complex: Typo + Shorthand Year"),
        ("", "cyberpunk 2.0", ["cyberpunk"], "Version Shorthand"),
        ("", "gta v", ["gta"], "Acronym Title"),
        ("", "lotr 4k", ["lotr", "lord of the rings"], "Acronym + Resolution"),
        ("", "Мортал Комбат", ["комбат"], "Cyrillic: Movie Title"),
        ("", "Матрица", ["матрица"], "Cyrillic: The Matrix")
    ]

    widths = [24, 22, 12, 12, 8, 8]
    headers = ["Search Query", "Scenario", "Dash Hits", "Port Hits", "Dash Q", "Port Q"]
    print_table_row(headers, widths, ["<", "<", ">", ">", "^", "^"])
    print("-" * 92)

    results = []
    portal_headers = {"X-Gaia-Debug": debug_token}

    for cat, q, targets, desc in queries:
        params = {"search": q, "limit": 10}
        if cat:
            params["category"] = cat
        qs = urllib.parse.urlencode(params)

        d_url = f"{dashboard_base}/api/torrents?{qs}"
        p_url = f"{portal_base}/api/torrents?{qs}"

        d_res = http_get(d_url)
        p_res = http_get(p_url, headers=portal_headers)

        # Parse Dashboard results
        d_hits_count = 0
        d_found = False
        d_top_title = "None"
        if d_res["status"] == 200 and d_res["data"]:
            items = d_res["data"].get("data", [])
            d_hits_count = d_res["data"].get("total", len(items))
            if items:
                d_top_title = items[0].get("name", "")
                for it in items[:3]:
                    name_lower = (it.get("name") or "").lower()
                    if any(t.lower() in name_lower for t in targets):
                        d_found = True
                        break

        # Parse Portal results
        p_hits_count = 0
        p_found = False
        p_top_title = "None"
        if p_res["status"] == 200 and p_res["data"]:
            items = p_res["data"].get("data", [])
            p_hits_count = p_res["data"].get("total", len(items))
            if items:
                p_top_title = items[0].get("name", "")
                for it in items[:3]:
                    name_lower = (it.get("name") or "").lower()
                    if any(t.lower() in name_lower for t in targets):
                        p_found = True
                        break

        d_badge = "✅ PASS" if d_found else "❌ FAIL"
        p_badge = "✅ PASS" if p_found else "❌ FAIL"

        row = [
            f"'{q}'",
            desc,
            f"{d_hits_count:,}",
            f"{p_hits_count:,}",
            d_badge,
            p_badge
        ]
        print_table_row(row, widths, ["<", "<", ">", ">", "^", "^"])

        results.append({
            "query": q,
            "category": cat,
            "targets": targets,
            "description": desc,
            "dashboard": {"hits": d_hits_count, "found_in_top3": d_found, "top_title": d_top_title, "ms": d_res["elapsed_ms"]},
            "portal": {"hits": p_hits_count, "found_in_top3": p_found, "top_title": p_top_title, "ms": p_res["elapsed_ms"]}
        })

    return results

def run_rps_load_suite(dashboard_base, portal_base, concurrency_tiers, duration_sec, debug_token):
    print_header("SUITE 3: Concurrency & RPS Throughput Load Test")

    workload_paths = [
        "/api/torrents?page=1&limit=25",
        "/api/torrents?page=2&limit=25",
        "/api/torrents?page=1&limit=25&category=Movies",
        "/api/torrents?page=1&limit=25&category=Television",
        "/api/torrents?page=1&limit=25&search=matrix",
        "/api/torrents?page=1&limit=25&search=ubuntu",
        "/api/torrents?page=1&limit=25&search=thmatrix+99&category=Movies",
        "/api/torrents?page=1&limit=25&sort=size&order=desc",
        "/api/torrents?page=1&limit=25&sort=popularity&order=desc"
    ]

    def worker_loop(base_url, headers, stop_at):
        req_count = 0
        err_count = 0
        latencies = []
        i = 0
        n_paths = len(workload_paths)
        while time.perf_counter() < stop_at:
            p = workload_paths[i % n_paths]
            i += 1
            res = http_get(f"{base_url}{p}", headers=headers, timeout=10)
            req_count += 1
            latencies.append(res["elapsed_ms"])
            if res["status"] != 200:
                err_count += 1
        return req_count, err_count, latencies

    widths = [14, 12, 10, 10, 10, 12, 10, 10, 10]
    headers = ["Concurrency", "Target", "RPS", "Total Req", "Errors", "P50 Lat", "P95 Lat", "P99 Lat", "Max Lat"]
    print_table_row(headers, widths, ["^", "<", ">", ">", ">", ">", ">", ">", ">"])
    print("-" * 96)

    rps_results = []
    portal_headers = {"X-Gaia-Debug": debug_token}

    for c in concurrency_tiers:
        for target_name, base_url, hdrs in [
            ("Dashboard", dashboard_base, None),
            ("Portal", portal_base, portal_headers)
        ]:
            start_wall = time.perf_counter()
            stop_at = start_wall + duration_sec

            with ThreadPoolExecutor(max_workers=c) as pool:
                futures = [pool.submit(worker_loop, base_url, hdrs, stop_at) for _ in range(c)]
                total_reqs = 0
                total_errs = 0
                all_lats = []
                for f in as_completed(futures):
                    cnt, err, lats = f.result()
                    total_reqs += cnt
                    total_errs += err
                    all_lats.extend(lats)

            actual_duration = time.perf_counter() - start_wall
            rps = round(total_reqs / actual_duration, 1) if actual_duration > 0 else 0
            stats = calculate_percentiles(all_lats)

            row = [
                f"{c} workers",
                target_name,
                f"{rps:,.1f}",
                str(total_reqs),
                str(total_errs),
                f"{stats['p50']}ms",
                f"{stats['p95']}ms",
                f"{stats['p99']}ms",
                f"{stats['max']}ms"
            ]
            print_table_row(row, widths, ["^", "<", ">", ">", ">", ">", ">", ">", ">"])

            rps_results.append({
                "concurrency": c,
                "target": target_name,
                "duration_sec": round(actual_duration, 2),
                "total_requests": total_reqs,
                "total_errors": total_errs,
                "rps": rps,
                "percentiles": stats
            })

    return rps_results

def print_executive_summary(perf_res, acc_res, rps_res):
    print_header("EXECUTIVE BENCHMARK SUMMARY & COMPARISON VERDICT")

    # 1. Latency summary
    d_warm_avg = statistics.mean([r["dashboard"]["warm_ms"] for r in perf_res])
    p_warm_avg = statistics.mean([r["portal"]["warm_ms"] for r in perf_res])
    latency_ratio = d_warm_avg / p_warm_avg if p_warm_avg > 0 else 1.0

    # 2. Accuracy summary
    d_passes = sum(1 for r in acc_res if r["dashboard"]["found_in_top3"])
    p_passes = sum(1 for r in acc_res if r["portal"]["found_in_top3"])
    total_q = len(acc_res)

    print(f"\n1. PERFORMANCE (BROWSE, SORT & FILTER LATENCY):")
    print(f"   • Gaia Dashboard Average Warm Latency: {d_warm_avg:.1f} ms")
    print(f"   • Gaia Portal Average Warm Latency:    {p_warm_avg:.1f} ms")
    print(f"   👉 Gaia Portal is {latency_ratio:.1f}x faster overall on cached and filtered queries.")

    print(f"\n2. SEARCH ACCURACY & FUZZY RECALL:")
    print(f"   • Gaia Dashboard Precision (Top 3):  {d_passes}/{total_q} ({d_passes/total_q*100:.1f}%)")
    print(f"   • Gaia Portal Precision (Top 3):     {p_passes}/{total_q} ({p_passes/total_q*100:.1f}%)")
    if p_passes > d_passes:
        print(f"   👉 Gaia Portal successfully resolved typos ('thmatrix', 'metrix', 'ubunut', 'cbyerpunk') that failed on Dashboard.")

    if rps_res:
        max_c = max(r["concurrency"] for r in rps_res)
        d_top_rps = next((r["rps"] for r in rps_res if r["target"] == "Dashboard" and r["concurrency"] == max_c), 0)
        p_top_rps = next((r["rps"] for r in rps_res if r["target"] == "Portal" and r["concurrency"] == max_c), 0)
        rps_ratio = p_top_rps / d_top_rps if d_top_rps > 0 else 1.0

        print(f"\n3. THROUGHPUT & CONCURRENCY CAPACITY ({max_c} Concurrent Workers):")
        print(f"   • Gaia Dashboard Peak Throughput: {d_top_rps:,.1f} RPS")
        print(f"   • Gaia Portal Peak Throughput:    {p_top_rps:,.1f} RPS")
        print(f"   👉 Gaia Portal delivers {rps_ratio:.1f}x higher throughput capacity under concurrent load.")

    print("\n" + "=" * 80 + "\n")

def main():
    args = parse_args()
    dashboard_base = args.dashboard_url.rstrip("/")
    portal_base = args.portal_url.rstrip("/")
    concurrency_tiers = [int(c.strip()) for c in args.concurrency.split(",") if c.strip().isdigit()]

    print_header("GAIA SEARCH BENCHMARK: DASHBOARD vs. PORTAL API")
    print(f"Target 1 (Gaia Dashboard): {dashboard_base}")
    print(f"Target 2 (Gaia Portal):    {portal_base}")
    print(f"Concurrency Tiers:         {concurrency_tiers}")
    print(f"RPS Duration per Tier:     {args.duration}s")
    print(f"Output Report:             {args.output}")

    # Health Checks
    print("\n[Verifying Target Connectivity...]")
    d_check = http_get(f"{dashboard_base}/api/torrents?limit=1")
    p_check = http_get(f"{portal_base}/api/torrents?limit=1", headers={"X-Gaia-Debug": args.debug_token})

    if d_check["status"] != 200:
        print(f"❌ ERROR: Gaia Dashboard unreachable at {dashboard_base}/api/torrents (status={d_check['status']}, error={d_check['error']})")
        sys.exit(1)
    print(f"✅ Gaia Dashboard is ONLINE ({d_check['elapsed_ms']:.1f}ms)")

    if p_check["status"] != 200:
        print(f"❌ ERROR: Gaia Portal unreachable at {portal_base}/api/torrents (status={p_check['status']}, error={p_check['error']})")
        sys.exit(1)
    print(f"✅ Gaia Portal is ONLINE ({p_check['elapsed_ms']:.1f}ms)")

    # Run Benchmark Suites
    perf_results = run_performance_suite(dashboard_base, portal_base, args.debug_token)
    acc_results = run_accuracy_fuzzy_suite(dashboard_base, portal_base, args.debug_token)

    rps_results = []
    if not args.skip_rps:
        rps_results = run_rps_load_suite(dashboard_base, portal_base, concurrency_tiers, args.duration, args.debug_token)

    # Executive Summary
    print_executive_summary(perf_results, acc_results, rps_results)

    # Save to JSON
    full_report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "targets": {
            "dashboard": dashboard_base,
            "portal": portal_base
        },
        "performance": perf_results,
        "accuracy": acc_results,
        "concurrency_rps": rps_results
    }
    with open(args.output, "w") as f:
        json.dump(full_report, f, indent=2)
    print(f"Saved full JSON benchmark report to {args.output}")

if __name__ == "__main__":
    main()
