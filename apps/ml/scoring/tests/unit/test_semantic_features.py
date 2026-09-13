"""
Unit tests for Semantic Threat Feature Extraction and Integration.
"""
import pytest
from src.features.semantic import extract_semantic_features
from src.features.integrity import extract_integrity_features


def test_deceptive_player_and_codec_tokens():
    # Deceptive codec pack in Movies
    feats = extract_semantic_features(
        name="Spider-Man.No.Way.Home.1080p.XviD-Codec-Pack.exe",
        files=[
            {"path": "Spider-Man.No.Way.Home.1080p.mp4", "length": 2500000000},
            {"path": "XviD_Codec_Pack_Installer.exe", "length": 15000000},
        ],
        category="Movies",
    )
    assert feats["has_deceptive_player_token"] == 1.0

    # Benign movie without player tokens
    benign_feats = extract_semantic_features(
        name="Inception.2010.1080p.BluRay.x264",
        files=[
            {"path": "Inception.2010.1080p.BluRay.x264.mkv", "length": 8500000000},
            {"path": "Inception.2010.1080p.BluRay.x264.nfo", "length": 4500},
        ],
        category="Movies",
    )
    assert benign_feats["has_deceptive_player_token"] == 0.0


def test_address_publisher_detection():
    # Chinese address publisher in Adult release
    feats = extract_semantic_features(
        name="FC2-PPV-3128491 社区最新网址发布器",
        files=[
            {"path": "FC2-PPV-3128491.mp4", "length": 3400000000},
            {"path": "1024社区-安卓发布器.apk", "length": 5000000},
            {"path": "4096社区新网址发布器.exe", "length": 2500000},
        ],
        category="Adult",
    )
    assert feats["has_address_publisher_token"] == 1.0
    assert feats["has_trojan_apk_token"] == 1.0


def test_phishing_password_trap_tokens():
    feats = extract_semantic_features(
        name="Top.Secret.Archive.2026.Confidential",
        files=[
            {"path": "Secret_Archive.zip", "length": 100000000},
            {"path": "password_unlock.txt", "length": 120},
            {"path": "survey_unlock.url", "length": 85},
        ],
        category="Applications",
    )
    assert feats["has_phishing_trap_token"] == 1.0


def test_trojan_viewer_in_documents():
    # Fake PDF reader in Books & Learning
    feats = extract_semantic_features(
        name="Python CookBook 4th Edition PDF",
        files=[
            {"path": "Python_CookBook.pdf", "length": 25000000},
            {"path": "PDF_Reader.exe", "length": 3500000},
        ],
        category="Books & Learning",
    )
    assert feats["has_trojan_viewer_token"] == 1.0

    # Legitimate ebook without reader exe
    benign_feats = extract_semantic_features(
        name="Clean Code by Robert Martin",
        files=[
            {"path": "Clean_Code.epub", "length": 12000000},
            {"path": "Clean_Code.pdf", "length": 15000000},
        ],
        category="Books & Learning",
    )
    assert benign_feats["has_trojan_viewer_token"] == 0.0


def test_hidden_script_dropper_detection():
    # Batch script dropper in Music album
    feats = extract_semantic_features(
        name="Taylor Swift - 1989 (Taylor's Version) FLAC",
        files=[
            {"path": "01. Welcome to New York.flac", "length": 35000000},
            {"path": "02. Blank Space.flac", "length": 38000000},
            {"path": "Listen_Album.bat", "length": 450},
        ],
        category="Music",
    )
    assert feats["has_script_dropper_token"] == 1.0


def test_integrity_features_includes_semantic_threats():
    feats = extract_integrity_features(
        name="Dune.Part.Two.2024.1080p.HDTS.XviD-Codec-Pack.exe",
        total_size=1500000000,
        file_count=2,
        files=[
            {"path": "Dune.Part.Two.mp4", "length": 1485000000},
            {"path": "XviD_Codec_Pack_Installer.exe", "length": 15000000},
        ],
        category="Movies",
    )
    assert "has_deceptive_player_token" in feats
    assert feats["has_deceptive_player_token"] == 1.0
    assert "has_address_publisher_token" in feats
    assert "has_phishing_trap_token" in feats
    assert "has_trojan_viewer_token" in feats
    assert "has_script_dropper_token" in feats
    assert "has_trojan_apk_token" in feats
    assert "payload_basename_entropy" in feats
