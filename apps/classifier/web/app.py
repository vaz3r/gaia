import sys
import threading
import subprocess
from pathlib import Path
from typing import Optional, List, Any, Union, Dict

from fastapi import FastAPI, HTTPException, Query, BackgroundTasks
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from pydantic import BaseModel

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

import db
from classifier_service import TorrentClassifierService
import model_manager
from reclassify_worker import ReclassifyManager

app = FastAPI(
    title="Gaia Torrent Classifier API",
    description="High-performance headless inference and active learning API daemon for DHT torrents in PostgreSQL",
    version="2.1.0"
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

classifier_service = TorrentClassifierService.get_instance()

# Global state for background retraining runs
_retrain_lock = threading.Lock()
_retrain_status = {
    "is_training": False,
    "last_run": None,
    "last_result": None
}


class ClassifyRequest(BaseModel):
    infohash: Optional[str] = None
    name: Optional[str] = None
    total_size: Optional[int] = 0
    file_count: Optional[int] = 1
    files: Optional[Union[List[Any], str]] = None
    confidence_threshold: Optional[float] = 0.65
    margin_threshold: Optional[float] = 0.35


class LabelRequest(BaseModel):
    infohash: str
    category: str
    reason: Optional[str] = None


class RollbackRequest(BaseModel):
    version: str


class ReclassifyRequest(BaseModel):
    batch_size: Optional[int] = 2000
    limit: Optional[int] = None
    dry_run: Optional[bool] = False


@app.get("/")
@app.head("/")
async def get_root():
    """Health check endpoint for headless classifier API daemon."""
    return {
        "status": "online",
        "service": "gaia-classifier-api",
        "version": "2.1.0"
    }


@app.get("/api/status")
def get_status():
    """Return model status and database connection info."""
    classifier_service.check_reload()
    return {
        "status": "online",
        "model_version": classifier_service.active_info.get("version", "v2"),
        "model_filename": classifier_service.active_info.get("filename", ""),
        "classes": classifier_service.classes,
        "database": {
            "host": db.DB_HOST,
            "database": db.POSTGRES_DB,
            "user": db.POSTGRES_USER
        }
    }


@app.get("/api/metrics")
def get_metrics():
    """Return review queue and system metrics."""
    return db.get_queue_metrics()


@app.get("/api/torrents")
def list_torrents(
    offset: int = Query(0, ge=0),
    limit: int = Query(25, ge=1, le=100),
    search: Optional[str] = Query(None),
    needs_review: Optional[bool] = Query(None),
    category: Optional[str] = Query(None)
):
    """Fetch paginated torrents from PostgreSQL."""
    try:
        data = db.get_torrents(offset=offset, limit=limit, search=search, needs_review=needs_review, category=category)
        return data
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Database error: {str(e)}")


@app.get("/api/torrents/{infohash}")
def get_torrent(infohash: str):
    """Fetch single torrent by infohash."""
    item = db.get_torrent_by_infohash(infohash)
    if not item:
        raise HTTPException(status_code=404, detail="Torrent not found in database")
    return item


@app.post("/api/classify")
def classify_torrent(req: ClassifyRequest):
    """Classify a torrent either by database infohash or custom metadata."""
    item_metadata = {}
    
    if req.infohash and not req.name:
        db_item = db.get_torrent_by_infohash(req.infohash)
        if not db_item:
            raise HTTPException(status_code=404, detail=f"Torrent infohash '{req.infohash}' not found in database")
        
        item_metadata = db_item
        payload = {
            "name": db_item.get("name") or "",
            "total_size": db_item.get("total_size") or 0,
            "file_count": db_item.get("file_count") or 1,
            "files": db_item.get("files") or []
        }
    elif req.name:
        payload = {
            "name": req.name,
            "total_size": req.total_size or 0,
            "file_count": req.file_count or 1,
            "files": req.files or []
        }
        item_metadata = {
            "infohash": req.infohash or "custom_test",
            "name": req.name,
            "total_size": req.total_size or 0,
            "total_size_formatted": db.format_size(req.total_size),
            "file_count": req.file_count or 1,
            "files": req.files or []
        }
    else:
        raise HTTPException(status_code=400, detail="Must provide either 'infohash' or 'name'")

    conf_thresh = req.confidence_threshold if req.confidence_threshold is not None else 0.65
    margin_thresh = req.margin_threshold if req.margin_threshold is not None else 0.35

    result = classifier_service.classify_single(
        payload,
        confidence_threshold=conf_thresh,
        margin_threshold=margin_thresh
    )

    return {
        "torrent": item_metadata,
        "classification": result
    }


@app.post("/api/labels")
def submit_label(req: LabelRequest):
    """Submit a verified or corrected label from the web review UI.
    Writes directly to PostgreSQL labeled_results with source 'manual_web'."""
    allowed_categories = set(classifier_service.classes) | {"Other"}
    if req.category not in allowed_categories:
        raise HTTPException(status_code=400, detail=f"Invalid category '{req.category}'. Allowed: {sorted(list(allowed_categories))}")

    try:
        res = db.upsert_label(
            infohash_hex=req.infohash,
            category=req.category,
            confidence="high",
            reason=req.reason or "Human verification via Web Studio",
            source="manual_web"
        )
        return res
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Failed to upsert label: {str(e)}")


@app.get("/api/models")
def get_models():
    """List available model artifacts and current active version."""
    return {
        "active": model_manager.get_active_model_info(),
        "available": model_manager.list_available_models()
    }


@app.post("/api/models/rollback")
def rollback_model(req: RollbackRequest):
    """Rollback active model to a previous version."""
    try:
        new_active = model_manager.rollback_to_version(req.version)
        classifier_service.check_reload()
        return {
            "status": "success",
            "active": new_active
        }
    except ValueError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Rollback failed: {str(e)}")


def _run_retrain_worker():
    global _retrain_status
    retrain_script = Path(__file__).parent.parent / "scripts" / "retrain.py"
    cmd = [sys.executable, str(retrain_script)]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        _retrain_status["last_run"] = {
            "exit_code": proc.returncode,
            "stdout": proc.stdout[-2000:],
            "stderr": proc.stderr[-2000:]
        }
    finally:
        _retrain_status["is_training"] = False


@app.post("/api/retrain")
def trigger_retraining(background_tasks: BackgroundTasks):
    """Trigger the direct DB retraining pipeline in background."""
    global _retrain_status
    if not _retrain_lock.acquire(blocking=False):
        raise HTTPException(status_code=409, detail="A retraining pipeline is already running.")

    try:
        if _retrain_status["is_training"]:
            raise HTTPException(status_code=409, detail="A retraining pipeline is already running.")
        _retrain_status["is_training"] = True
        background_tasks.add_task(_run_retrain_worker)
        return {"status": "started", "message": "Retraining started in background. Monitor via /api/retrain/status."}
    finally:
        _retrain_lock.release()


@app.get("/api/retrain/status")
def get_retrain_status():
    """Check current retraining progress."""
    return _retrain_status


@app.post("/api/reclassify/start")
def start_reclassification(req: Optional[ReclassifyRequest] = None):
    """Trigger background reclassification on review queue items."""
    manager = ReclassifyManager.get_instance()
    batch_size = req.batch_size if req and req.batch_size else 500
    limit = req.limit if req and req.limit else None
    dry_run = bool(req.dry_run) if req and req.dry_run is not None else False

    try:
        status = manager.start(batch_size=batch_size, limit=limit, dry_run=dry_run)
        return {"status": "started", "telemetry": status}
    except RuntimeError as e:
        raise HTTPException(status_code=409, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Failed to start reclassification: {str(e)}")


@app.get("/api/reclassify/status")
async def get_reclassification_status():
    """Retrieve live real-time reclassification telemetry."""
    manager = ReclassifyManager.get_instance()
    return manager.get_status()


@app.post("/api/reclassify/cancel")
async def cancel_reclassification():
    """Cancel any active background reclassification."""
    manager = ReclassifyManager.get_instance()
    cancelled = manager.cancel()
    if not cancelled:
        return {"status": "not_running", "message": "No reclassification job currently active."}
    return {"status": "cancelling", "message": "Reclassification cancellation requested. Worker will halt cleanly after current batch."}
