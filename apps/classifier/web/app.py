import sys
from pathlib import Path
from typing import Optional, List, Any, Union, Dict

from fastapi import FastAPI, HTTPException, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from pydantic import BaseModel

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

import db
from classifier_service import TorrentClassifierService

app = FastAPI(
    title="Gaia Torrent Classifier Studio",
    description="Interactive exploration and classification of DHT torrents in PostgreSQL (READ_ONLY)",
    version="2.0.0"
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

classifier_service = TorrentClassifierService.get_instance()
INDEX_HTML_PATH = Path(__file__).parent / "templates" / "index.html"


class ClassifyRequest(BaseModel):
    infohash: Optional[str] = None
    name: Optional[str] = None
    total_size: Optional[int] = 0
    file_count: Optional[int] = 1
    files: Optional[Union[List[Any], str]] = None
    confidence_threshold: Optional[float] = 0.65
    margin_threshold: Optional[float] = 0.35


@app.get("/")
@app.head("/")
def get_index():
    """Serve the interactive web UI."""
    if not INDEX_HTML_PATH.exists():
        raise HTTPException(status_code=404, detail="Frontend template not found")
    return FileResponse(INDEX_HTML_PATH)


@app.get("/api/status")
def get_status():
    """Return model status and database connection info."""
    return {
        "status": "online",
        "model_version": "v2_calibrated_sgd",
        "classes": classifier_service.classes,
        "metadata": classifier_service.metadata,
        "database": {
            "host": db.DB_HOST,
            "database": db.POSTGRES_DB,
            "user": db.POSTGRES_USER,
            "mode": "STRICT_READ_ONLY"
        }
    }


@app.get("/api/torrents")
def list_torrents(
    offset: int = Query(0, ge=0),
    limit: int = Query(25, ge=1, le=100),
    search: Optional[str] = Query(None)
):
    """Fetch paginated torrents from PostgreSQL."""
    try:
        data = db.get_torrents(offset=offset, limit=limit, search=search)
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
