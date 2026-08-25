from dataclasses import dataclass, field

ALLOWED_CATEGORIES = frozenset({
    "Movies",
    "Television",
    "Games",
    "Music",
    "Applications",
    "Anime",
    "Documentaries",
    "Other",
    "Porn",
})


@dataclass
class TorrentInput:
    infohash: str
    name: str
    file_count: int
    total_size_bytes: int
    files: list[str] = field(default_factory=list)


@dataclass
class ClassificationResult:
    category: str
    confidence: float

    def __post_init__(self):
        if self.category not in ALLOWED_CATEGORIES:
            raise ValueError(
                f"Invalid category '{self.category}'. "
                f"Must be one of: {', '.join(sorted(ALLOWED_CATEGORIES))}"
            )
        if not (0.0 <= self.confidence <= 1.0):
            raise ValueError(
                f"Confidence {self.confidence} out of range [0.0, 1.0]"
            )
