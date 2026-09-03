#!/usr/bin/env python3
"""
Release Normalizer for Audit Leakage Detection.
Constructs multi-level normalization keys to detect exact and release-family leakage
across dataset splits without altering production preprocessing.
"""
from __future__ import annotations
import re
import unicodedata

# Punctuation / separator normalization
RE_PUNCT = re.compile(r'[\._\-\+\[\]\(\)\{\},:;!@#\$%\^&\*~`\'"|/\\]+')

# Resolution tags
RE_RESOLUTION = re.compile(r'\b(2160p|1080p|1080i|720p|576p|480p|360p|4k|8k|uhd|fhd|hd|sd)\b', re.IGNORECASE)

# Codec tags
RE_CODEC = re.compile(
    r'\b(x264|x265|h264|h265|hevc|avc|xvid|divx|mpeg2|10bit|8bit|'
    r'aac|mp3|flac|dts|dd5\.?1|ac3|eac3|truehd|atmos|opus|lossless|320kbps|v0|vbr|cbr)\b',
    re.IGNORECASE
)

# Source / container / media tags
RE_SOURCE = re.compile(
    r'\b(bluray|blu-ray|bdrip|brrip|web-dl|webdl|webrip|web|hdtv|pdtv|dvdrip|dvd|remux|'
    r'vhsrip|cam|telesync|ts|proper|repack|multi|multisubs?|subbed|dubbed|vostfr|raw|dlraw)\b',
    re.IGNORECASE
)

# Fansub / Scene groups commonly found in brackets or suffixes
RE_BRACKETS_GROUP = re.compile(r'\[(subsplease|erai-raws|horriblesubs|judas|dkb|asw|commie|fff|coalgirls|animetime|neoae|baha|ani|vcb-studio|eztv|tgx|rarbg|yts|yify|fitgirl|dodi|codex|plaza|skidrow|rune|tenoke|gjm)\]', re.IGNORECASE)
RE_SUFFIX_GROUP = re.compile(r'-[a-zA-Z0-9_]{2,15}$', re.IGNORECASE)

# Season / Episode pattern
RE_S_E = re.compile(r'\bS(\d{1,2})E(\d{1,3})\b', re.IGNORECASE)
RE_SEASON_ONLY = re.compile(r'\bSeason\s*(\d{1,2})\b', re.IGNORECASE)
RE_EP_ONLY = re.compile(r'\b(?:Episode|Ep)\s*(\d{1,3})\b', re.IGNORECASE)

# File extensions to strip
RE_EXT = re.compile(r'\.(mkv|mp4|avi|wmv|flv|mov|ts|m4v|mpg|mpeg|webm|flac|mp3|wav|aac|ogg|opus|m4a|wma|srt|ass|ssa|zip|rar|7z|tar|gz|bz2|iso|nsp|xci|pkg|exe|msi|dmg|deb|rpm|pdf|epub|mobi|txt|doc|docx|cbz|cbr)$', re.IGNORECASE)

# Hash / CRC32 in brackets e.g. [3356AB2D]
RE_CRC = re.compile(r'\[[0-9a-fA-F]{8}\]')


def normalize_full_name(raw_name: str) -> str:
    """Level 2: Basic cleaned name (lowercase, NFKD, alphanumeric tokens only)."""
    if not raw_name:
        return ""
    # NFKD unicode normalization
    s = unicodedata.normalize('NFKD', raw_name)
    s = RE_EXT.sub('', s)
    s = RE_PUNCT.sub(' ', s).lower()
    return re.sub(r'\s+', ' ', s).strip()


def normalize_release_family(raw_name: str) -> str:
    """
    Level 3: Content release-family key.
    Strips resolution, codecs, sources, release groups, and CRCs while normalizing
    season/episode and preserving title, franchise, and year.
    """
    if not raw_name:
        return ""
    s = unicodedata.normalize('NFKD', raw_name)
    s = RE_EXT.sub('', s)
    s = RE_CRC.sub('', s)
    s = RE_BRACKETS_GROUP.sub('', s)
    s = RE_SUFFIX_GROUP.sub('', s)

    # Standardize Season/Episode notation
    def rep_se(m):
        return f" s{int(m.group(1)):02d}e{int(m.group(2)):02d} "
    s = RE_S_E.sub(rep_se, s)

    def rep_s(m):
        return f" s{int(m.group(1)):02d} "
    s = RE_SEASON_ONLY.sub(rep_s, s)

    def rep_e(m):
        return f" e{int(m.group(1)):02d} "
    s = RE_EP_ONLY.sub(rep_e, s)

    # Strip resolution, codec, source tags
    s = RE_RESOLUTION.sub(' ', s)
    s = RE_CODEC.sub(' ', s)
    s = RE_SOURCE.sub(' ', s)

    # Clean punctuation
    s = RE_PUNCT.sub(' ', s).lower()
    s = re.sub(r'\s+', ' ', s).strip()
    return s
