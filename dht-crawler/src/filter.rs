use std::cmp::Reverse;

use crate::storage::Category;

/// Quality / container tags that mark a movie release.
const QUALITY_TAGS: &[&str] = &[
    "1080p", "720p", "2160p", "480p", "4k", "bluray", "brrip", "web-dl", "webrip", "hdtv", "dvdrip",
];

/// Video container extensions used to spot candidate filenames.
const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "avi", "ts", "m2ts", "mov", "wmv", "webm", "flv", "m4v"];

/// Tags stripped from titles (codecs, containers, scene packaging).
const EXTRA_TAGS: &[&str] = &[
    "x264", "x265", "h264", "h265", "hevc", "remux", "repack", "proper", "extended", "complete",
    "retail",
];

/// The result of classifying a release name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub category: Category,
    pub title: String,
    pub year: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

/// Deterministic movie/TV name classifier.
#[derive(Debug, Default)]
pub struct MediaFilter;

impl MediaFilter {
    /// Normalize a release name: punctuation → space, collapse whitespace,
    /// lowercase. Hyphens are preserved (so `x264-YIFY` stays one token).
    pub fn normalize(name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        let mut in_space = false;
        for c in name.chars() {
            if c.is_ascii_alphanumeric() || c == '-' {
                out.push(c.to_ascii_lowercase());
                in_space = false;
            } else if !in_space {
                out.push(' ');
                in_space = true;
            }
        }
        out.trim().to_string()
    }

    /// Classify a raw release name, or `None` if it is neither movie nor TV.
    pub fn classify(&self, name: &str) -> Option<Classification> {
        let norm = Self::normalize(name);
        let tokens: Vec<&str> = norm.split_whitespace().collect();
        if tokens.is_empty() {
            return None;
        }

        if let Some((season, episode)) = tv_marker(&tokens) {
            return Some(Classification {
                category: Category::Tv,
                title: clean_title(&tokens, &norm, None),
                year: extract_year(&tokens),
                season: Some(season),
                episode,
            });
        }

        let year = extract_year(&tokens)?;
        if !has_quality(&tokens) {
            return None;
        }

        Some(Classification {
            category: Category::Movie,
            title: clean_title(&tokens, &norm, Some(year)),
            year: Some(year),
            season: None,
            episode: None,
        })
    }

    /// Classify a torrent from its contained files when the root name is
    /// generic or missing. Only video files are considered; the largest file
    /// that yields a confident movie/TV classification wins.
    pub fn classify_by_files(&self, files: &[(String, i64)]) -> Option<Classification> {
        let mut candidates: Vec<(String, i64)> = files
            .iter()
            .filter_map(|(path, size)| {
                let name = basename(path)?;
                let ext = extension(name)?;
                if !VIDEO_EXTS.contains(&ext) {
                    return None;
                }
                let stem = strip_extension(name)?;
                Some((stem.to_string(), *size))
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by_key(|c| Reverse(c.1));
        for (stem, _) in candidates {
            if let Some(c) = self.classify(&stem) {
                return Some(c);
            }
        }
        None
    }
}

fn extract_year(tokens: &[&str]) -> Option<i64> {
    tokens.iter().find_map(|t| {
        if t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
            let y = t.parse::<i64>().ok()?;
            if (1900..=2099).contains(&y) {
                return Some(y);
            }
        }
        None
    })
}

fn has_quality(tokens: &[&str]) -> bool {
    tokens.iter().any(|t| QUALITY_TAGS.contains(t))
}

/// Detect a TV season/episode marker. Returns `(season, episode)` where
/// episode is `None` for whole-season packs.
fn tv_marker(tokens: &[&str]) -> Option<(i64, Option<i64>)> {
    // SxxExx / SxEx
    for t in tokens {
        if let Some((s, e)) = parse_sxxexx(t) {
            return Some((s, Some(e)));
        }
    }

    // "season 5 episode 9" or "season 5"
    for (i, t) in tokens.iter().enumerate() {
        if *t == "season" {
            let season = tokens.get(i + 1).and_then(|n| n.parse::<i64>().ok())?;
            if tokens.get(i + 2) == Some(&"episode") {
                let episode = tokens.get(i + 3).and_then(|n| n.parse::<i64>().ok());
                return Some((season, episode));
            }
            return Some((season, None));
        }
    }

    None
}

/// Parse a `s05e09`-style token into (season, episode).
fn parse_sxxexx(t: &str) -> Option<(i64, i64)> {
    let bytes = t.as_bytes();
    if bytes.len() < 3 || bytes[0] != b's' {
        return None;
    }
    let e_pos = t.find('e')?;
    let season = t[1..e_pos].parse::<i64>().ok()?;
    let ep = &t[e_pos + 1..];
    if ep.is_empty() {
        return None;
    }
    let ep_digits: String = ep.chars().take_while(|c| c.is_ascii_digit()).collect();
    if ep_digits.is_empty() {
        return None;
    }
    let episode = ep_digits.parse::<i64>().ok()?;
    if !(1..=999).contains(&season) || !(1..=999).contains(&episode) {
        return None;
    }
    Some((season, episode))
}

/// Strip tags to recover the clean title (e.g. `the matrix`).
///
/// Release names order tags after the title, so we collect tokens up to the
/// first tag-like token (year, quality, codec, season/episode marker, or
/// scene packaging word) and drop everything from there on.
fn clean_title(tokens: &[&str], _norm: &str, _year: Option<i64>) -> String {
    let title: Vec<&str> = tokens
        .iter()
        .take_while(|t| !is_tag_token(t))
        .copied()
        .collect();

    if title.is_empty() {
        return "".to_string();
    }
    title.join(" ")
}

fn is_tag_token(t: &str) -> bool {
    is_year(t)
        || QUALITY_TAGS.contains(&t)
        || EXTRA_TAGS.contains(&t)
        || parse_sxxexx(t).is_some()
        || t == "season"
        || t == "episode"
}

fn is_year(t: &str) -> bool {
    if t.len() != 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    t.parse::<i64>()
        .is_ok_and(|y| (1900..=2099).contains(&y))
}

/// Last path segment of a slash-joined path.
fn basename(path: &str) -> Option<&str> {
    let seg = path.rsplit('/').next().unwrap_or(path);
    (!seg.is_empty()).then_some(seg)
}

/// File extension (lowercased, no dot), or `None` if there is none.
fn extension(name: &str) -> Option<&str> {
    let idx = name.rfind('.')?;
    let ext = &name[idx + 1..];
    if ext.is_empty() || ext.contains('.') {
        return None;
    }
    Some(ext)
}

/// Filename without its extension.
fn strip_extension(name: &str) -> Option<&str> {
    let idx = name.rfind('.')?;
    let stem = &name[..idx];
    (!stem.is_empty()).then_some(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_punctuation() {
        assert_eq!(
            MediaFilter::normalize("The.Matrix.1999.1080p.BluRay.x264-YIFY"),
            "the matrix 1999 1080p bluray x264-yify"
        );
    }

    #[test]
    fn movie_pass() {
        let f = MediaFilter;
        let c = f
            .classify("The Matrix 1999 1080p BluRay x264 YIFY")
            .expect("movie");
        assert_eq!(c.category, Category::Movie);
        assert_eq!(c.year, Some(1999));
        assert_eq!(c.title, "the matrix");
    }

    #[test]
    fn movie_missing_quality_rejected() {
        let f = MediaFilter;
        assert!(f.classify("Some Cool Film 1998").is_none());
    }

    #[test]
    fn tv_episode_marker() {
        let f = MediaFilter;
        let c = f
            .classify("Breaking Bad S05E09 1080p HDTV x264")
            .expect("tv");
        assert_eq!(c.category, Category::Tv);
        assert_eq!(c.season, Some(5));
        assert_eq!(c.episode, Some(9));
        assert_eq!(c.title, "breaking bad");
    }

    #[test]
    fn tv_season_pack() {
        let f = MediaFilter;
        let c = f
            .classify("Game of Thrones Season 1 Complete 720p")
            .expect("tv");
        assert_eq!(c.category, Category::Tv);
        assert_eq!(c.season, Some(1));
        assert_eq!(c.episode, None);
        assert_eq!(c.title, "game of thrones");
    }

    #[test]
    fn tv_single_digit_season() {
        let f = MediaFilter;
        let c = f.classify("The Wire S1E3 720p").expect("tv");
        assert_eq!(c.season, Some(1));
        assert_eq!(c.episode, Some(3));
    }

    #[test]
    fn software_skipped() {
        let f = MediaFilter;
        assert!(f.classify("Adobe Photoshop 2024 macOS").is_none());
    }

    #[test]
    fn deterministic_rerun_equality() {
        let f = MediaFilter;
        let a = f.classify("The Matrix 1999 1080p BluRay x264 YIFY");
        let b = f.classify("The Matrix 1999 1080p BluRay x264 YIFY");
        assert_eq!(a, b);
    }

    #[test]
    fn file_based_classification_fallback() {
        let f = MediaFilter;
        // Root name too generic for movie classification (no quality tag).
        let files = vec![
            ("readme.txt".to_string(), 1),
            ("VIDEO".to_string(), 2),
            ("The Matrix 1999 1080p BluRay x264.mkv".to_string(), 1024),
        ];
        let c = f
            .classify_by_files(&files)
            .expect("largest video file classifies");
        assert_eq!(c.category, Category::Movie);
        assert_eq!(c.title, "the matrix");
        assert_eq!(c.year, Some(1999));
    }

    #[test]
    fn file_based_classification_prefers_largest() {
        let f = MediaFilter;
        let files = vec![
            ("Some.2001.720p.mkv".to_string(), 100),
            ("Good.Film.1999.1080p.mkv".to_string(), 900),
        ];
        let c = f.classify_by_files(&files).unwrap();
        assert_eq!(c.title, "good film");
        assert_eq!(c.year, Some(1999));
    }

    #[test]
    fn file_based_classification_ignores_non_video() {
        let f = MediaFilter;
        let files = vec![
            ("album.cover.jpg".to_string(), 50),
            ("notes.txt".to_string(), 10),
        ];
        assert!(f.classify_by_files(&files).is_none());
    }

    #[test]
    fn file_stem_helpers() {
        assert_eq!(basename("A/B/c.mkv"), Some("c.mkv"));
        assert_eq!(strip_extension("The.Matrix.1999.1080p.mkv"), Some("The.Matrix.1999.1080p"));
        assert_eq!(extension("x.mp4"), Some("mp4"));
        assert_eq!(extension("noext"), None);
    }
}
