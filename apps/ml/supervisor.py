#!/usr/bin/env python3
"""
GAIA ML Multi-Process Supervisor (gaia-ml)
=========================================
Manages and supervises all machine learning workers across GAIA in isolated OS processes.

Key Features:
1. True Process Isolation: Each worker runs in an independent OS process with its own GIL.
2. Crash Resilience: Crashes in one model/worker trigger automatic restart with backoff
   without impacting other models.
3. Graceful Signal Propagation: Traps SIGTERM/SIGINT, cleanly terminating all child workers.
4. Log Tagging: Interleaves and streams worker logs with prefix tags [scoring] / [anomalies].
5. Health Watchdog: Monitors worker heartbeats and maintains /tmp/gaia_ml_healthy for Docker.
6. Extensible Registry: Easy plug-in for future ML models via ENABLED_WORKERS.
"""

import os
import sys
import time
import signal
import threading
import subprocess
import logging
from pathlib import Path
from typing import Dict, Any, List

logging.basicConfig(
    level=os.getenv("LOG_LEVEL", "INFO").upper(),
    format="%(asctime)s [%(levelname)s] [supervisor] %(message)s",
)
logger = logging.getLogger("supervisor")

BASE_DIR = Path(__file__).resolve().parent
HEALTHY_FILE = Path("/tmp/gaia_ml_healthy")

# Worker Registry Configuration
WORKER_REGISTRY: Dict[str, Dict[str, Any]] = {
    "scoring": {
        "cwd": BASE_DIR / "scoring",
        "cmd": [sys.executable, "src/worker.py"],
        "heartbeat_file": Path("/tmp/scoring_worker_heartbeat"),
        "max_heartbeat_age_sec": 180,
        "env_overrides": {
            "HEARTBEAT_PATH": "/tmp/scoring_worker_heartbeat",
        },
    },
    "anomalies": {
        "cwd": BASE_DIR / "anomalies",
        "cmd": [sys.executable, "src/worker.py"],
        "heartbeat_file": Path("/tmp/anomaly_worker_heartbeat"),
        "max_heartbeat_age_sec": 1200,  # 20 min (runs on 15m cadence)
        "env_overrides": {
            "HEARTBEAT_FILE": "/tmp/anomaly_worker_heartbeat",
        },
    },
}


class WorkerHandle:
    def __init__(self, name: str, config: Dict[str, Any]):
        self.name = name
        self.config = config
        self.process: subprocess.Popen | None = None
        self.restart_count = 0
        self.last_start_time = 0.0
        self.backoff_delay = 5.0
        self.threads: List[threading.Thread] = []

    def start(self):
        env = os.environ.copy()
        # Merge worker-specific overrides
        env.update(self.config.get("env_overrides", {}))
        # Ensure python unbuffered
        env["PYTHONUNBUFFERED"] = "1"

        logger.info(f"Starting worker [{self.name}] in {self.config['cwd']}...")
        self.last_start_time = time.time()

        try:
            self.process = subprocess.Popen(
                self.config["cmd"],
                cwd=str(self.config["cwd"]),
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
            )
            logger.info(f"Worker [{self.name}] started with PID {self.process.pid}.")

            # Start streaming stdout & stderr with prefix
            t_out = threading.Thread(target=self._stream_output, args=(self.process.stdout, "INFO"), daemon=True)
            t_err = threading.Thread(target=self._stream_output, args=(self.process.stderr, "ERROR"), daemon=True)
            t_out.start()
            t_err.start()
            self.threads = [t_out, t_err]
        except Exception as e:
            logger.error(f"Failed to start worker [{self.name}]: {e}", exc_info=True)
            self.process = None

    def _stream_output(self, stream, level_hint: str):
        try:
            for line in iter(stream.readline, ""):
                line_clean = line.rstrip()
                if line_clean:
                    print(f"[{self.name}] {line_clean}", flush=True)
        except Exception:
            pass
        finally:
            stream.close()

    def is_alive(self) -> bool:
        if self.process is None:
            return False
        return self.process.poll() is None

    def terminate(self, timeout: float = 10.0):
        if not self.is_alive():
            return
        logger.info(f"Sending SIGTERM to worker [{self.name}] (PID {self.process.pid})...")
        try:
            self.process.terminate()
            self.process.wait(timeout=timeout)
            logger.info(f"Worker [{self.name}] exited cleanly.")
        except subprocess.TimeoutExpired:
            logger.warning(f"Worker [{self.name}] did not exit within {timeout}s. Sending SIGKILL...")
            self.process.kill()
            self.process.wait()
            logger.info(f"Worker [{self.name}] killed.")
        except Exception as e:
            logger.error(f"Error terminating worker [{self.name}]: {e}")

    def is_healthy(self) -> bool:
        if not self.is_alive():
            return False
        hb_file: Path = self.config.get("heartbeat_file")
        if not hb_file:
            return True
        if not hb_file.exists():
            # Allow grace period of 30s after startup
            return (time.time() - self.last_start_time) < 30.0
        try:
            age = time.time() - hb_file.stat().st_mtime
            max_age = self.config.get("max_heartbeat_age_sec", 60)
            return age <= max_age
        except Exception:
            return False


class Supervisor:
    def __init__(self, enabled_workers: List[str]):
        self.enabled_workers = enabled_workers
        self.workers: Dict[str, WorkerHandle] = {}
        self.running = True

        signal.signal(signal.SIGTERM, self._handle_signal)
        signal.signal(signal.SIGINT, self._handle_signal)

        for name in self.enabled_workers:
            if name in WORKER_REGISTRY:
                self.workers[name] = WorkerHandle(name, WORKER_REGISTRY[name])
            else:
                logger.warning(f"Worker '{name}' not found in WORKER_REGISTRY. Skipping.")

    def _handle_signal(self, signum, frame):
        sig_name = signal.Signals(signum).name
        logger.info(f"Supervisor received {sig_name} ({signum}). Initiating shutdown...")
        self.running = False

    def start_all(self):
        logger.info(f"Supervisor launching {len(self.workers)} worker(s): {list(self.workers.keys())}")
        for handle in self.workers.values():
            handle.start()

    def shutdown_all(self):
        logger.info("Stopping all workers...")
        if HEALTHY_FILE.exists():
            try:
                HEALTHY_FILE.unlink()
            except Exception:
                pass

        for handle in self.workers.values():
            handle.terminate(timeout=10.0)
        logger.info("All workers terminated. Supervisor exiting.")

    def run(self):
        self.start_all()

        while self.running:
            time.sleep(2.0)
            all_healthy = True

            for name, handle in self.workers.items():
                if not handle.is_alive():
                    exit_code = handle.process.poll() if handle.process else -1
                    logger.error(f"Worker [{name}] died with exit code {exit_code}!")

                    handle.restart_count += 1
                    delay = min(handle.backoff_delay * (1.5 ** (handle.restart_count - 1)), 60.0)
                    logger.info(f"Restarting [{name}] in {delay:.1f}s (restart #{handle.restart_count})...")
                    time.sleep(delay)
                    handle.start()

                if not handle.is_healthy():
                    all_healthy = False

            if all_healthy and self.running:
                try:
                    HEALTHY_FILE.touch()
                except Exception:
                    pass
            else:
                if HEALTHY_FILE.exists():
                    try:
                        HEALTHY_FILE.unlink()
                    except Exception:
                        pass

        self.shutdown_all()


def main():
    enabled_env = os.getenv("ENABLED_WORKERS", "scoring,anomalies")
    enabled_workers = [w.strip() for w in enabled_env.split(",") if w.strip()]

    if "--dry-run" in sys.argv or "--test-dry-run" in sys.argv:
        logger.info(f"Dry-run verified: registry contains {list(WORKER_REGISTRY.keys())}, enabled={enabled_workers}")
        sys.exit(0)

    supervisor = Supervisor(enabled_workers)
    supervisor.run()


if __name__ == "__main__":
    main()
