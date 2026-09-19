#!/usr/bin/env python3
"""
Unified Entrypoint Supervisor for Gaia Classifier.
Runs Uvicorn (FastAPI) and the continuous Queue Worker concurrently in a single container.
"""

import os
import sys
import time
import signal
import subprocess
from pathlib import Path

procs = []
_terminating = False


def handle_shutdown(signum, frame):
    global _terminating
    if _terminating:
        return
    _terminating = True
    print(f"\n[supervisor] Received signal {signum}. Initiating graceful shutdown...", flush=True)

    for p in procs:
        if p.poll() is None:
            try:
                p.terminate()
            except Exception as e:
                print(f"[supervisor] Error sending SIGTERM to PID {p.pid}: {e}", flush=True)

    deadline = time.time() + 10.0
    for p in procs:
        remaining = max(0.1, deadline - time.time())
        try:
            p.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            print(f"[supervisor] PID {p.pid} did not exit in time. Sending SIGKILL...", flush=True)
            try:
                p.kill()
            except Exception:
                pass

    print("[supervisor] All subprocesses stopped. Exiting.", flush=True)
    sys.exit(0)


def main():
    signal.signal(signal.SIGINT, handle_shutdown)
    signal.signal(signal.SIGTERM, handle_shutdown)

    mode = os.environ.get("MODE", "all").strip().lower()
    host = os.environ.get("HOST", "0.0.0.0")
    port = os.environ.get("PORT", os.environ.get("CLASSIFIER_PORT", "8000"))
    batch_size = os.environ.get("WORKER_BATCH_SIZE", "2000")
    poll_interval = os.environ.get("WORKER_POLL_INTERVAL", "15")

    app_root = Path(__file__).resolve().parent.parent

    print("=" * 80, flush=True)
    print("GAIA CLASSIFIER UNIFIED SUPERVISOR", flush=True)
    print(f"Mode: {mode} | Host: {host} | Port: {port} | Batch: {batch_size} | Poll: {poll_interval}s", flush=True)
    print("=" * 80, flush=True)

    # Launch API
    if mode in ("all", "api"):
        api_cmd = [
            sys.executable, "-m", "uvicorn", "web.app:app",
            "--host", str(host),
            "--port", str(port),
            "--proxy-headers"
        ]
        print(f"[supervisor] Starting Classifier API: {' '.join(api_cmd)}", flush=True)
        p_api = subprocess.Popen(api_cmd, cwd=str(app_root))
        procs.append(p_api)

    # Launch Worker
    if mode in ("all", "worker"):
        worker_cmd = [
            sys.executable, "scripts/worker.py",
            "--batch-size", str(batch_size),
            "--poll-interval", str(poll_interval)
        ]
        print(f"[supervisor] Starting Classifier Worker: {' '.join(worker_cmd)}", flush=True)
        p_worker = subprocess.Popen(worker_cmd, cwd=str(app_root))
        procs.append(p_worker)

    if not procs:
        print(f"[supervisor] No processes started for mode '{mode}'. Exiting.", flush=True)
        sys.exit(1)

    # Monitor subprocesses
    while not _terminating:
        for p in procs:
            ret = p.poll()
            if ret is not None:
                print(f"[supervisor] Child process (PID {p.pid}) exited unexpectedly with code {ret}.", flush=True)
                handle_shutdown(signal.SIGTERM, None)
                sys.exit(ret if ret != 0 else 1)
        time.sleep(1)


if __name__ == "__main__":
    main()
