import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.core.xml_guardrails import (
    build_retry_system_prompt,
    build_torrent_xml,
    parse_classification_xml,
    safe_xml_text,
)
from src.core.types import TorrentInput, ClassificationResult


class TestSafeXmlText:
    def test_escapes_xml_chars(self):
        assert safe_xml_text("<tag>", 100) == "&lt;tag&gt;"

    def test_escapes_quotes(self):
        result = safe_xml_text('He said "hello"', 100)
        assert "&quot;" in result

    def test_truncates(self):
        result = safe_xml_text("a" * 200, 50)
        assert len(result) == 50

    def test_normal_text_passes(self):
        assert safe_xml_text("hello world", 100) == "hello world"


class TestBuildTorrentXml:
    def _make_torrent(self):
        return TorrentInput(
            infohash="abcd1234",
            name="Test Movie 2024",
            file_count=2,
            total_size_bytes=1000000,
            files=["test.mkv", "test.srt"],
        )

    def test_valid_xml(self):
        torrent = self._make_torrent()
        xml = build_torrent_xml(torrent)
        assert "<torrent>" in xml
        assert "</torrent>" in xml
        assert "<name>Test Movie 2024</name>" in xml
        assert "<file>test.mkv</file>" in xml
        assert "<file>test.srt</file>" in xml

    def test_escapes_name(self):
        torrent = TorrentInput(
            infohash="abcd",
            name="Movie <2024>",
            file_count=1,
            total_size_bytes=1000,
            files=[],
        )
        xml = build_torrent_xml(torrent)
        assert "&lt;2024&gt;" in xml
        assert "<2024>" not in xml

    def test_truncates_files(self):
        torrent = TorrentInput(
            infohash="abcd",
            name="Test",
            file_count=50,
            total_size_bytes=1000,
            files=[f"file{i}.mkv" for i in range(50)],
        )
        config = {"prompt": {"max_input_files": 5}}
        xml = build_torrent_xml(torrent, config)
        assert xml.count("<file>") == 5


class TestParseClassificationXml:
    def test_valid_xml(self):
        xml = "<classification>\n  <category>Movies</category>\n  <confidence>0.98</confidence>\n</classification>"
        result = parse_classification_xml(xml)
        assert result is not None
        assert result.category == "Movies"
        assert result.confidence == 0.98

    def test_markdown_fenced(self):
        xml = "```xml\n<classification>\n  <category>Television</category>\n  <confidence>0.90</confidence>\n</classification>\n```"
        result = parse_classification_xml(xml)
        assert result is not None
        assert result.category == "Television"

    def test_surrounded_by_text(self):
        text = "Here is the result:\n<classification>\n  <category>Music</category>\n  <confidence>0.85</confidence>\n</classification>\nDone."
        result = parse_classification_xml(text)
        assert result is not None
        assert result.category == "Music"

    def test_invalid_category(self):
        xml = "<classification>\n  <category>InvalidCat</category>\n  <confidence>0.90</confidence>\n</classification>"
        result = parse_classification_xml(xml)
        assert result is None

    def test_invalid_confidence(self):
        xml = "<classification>\n  <category>Movies</category>\n  <confidence>1.5</confidence>\n</classification>"
        result = parse_classification_xml(xml)
        assert result is None

    def test_malformed_xml(self):
        result = parse_classification_xml("not xml at all")
        assert result is None

    def test_empty_string(self):
        result = parse_classification_xml("")
        assert result is None

    def test_confidence_zero(self):
        xml = "<classification>\n  <category>Other</category>\n  <confidence>0.0</confidence>\n</classification>"
        result = parse_classification_xml(xml)
        assert result is not None
        assert result.confidence == 0.0

    def test_all_categories(self):
        for cat in ["Movies", "Television", "Games", "Music", "Applications", "Anime", "Documentaries", "Other", "Unwanted"]:
            xml = f"<classification>\n  <category>{cat}</category>\n  <confidence>0.50</confidence>\n</classification>"
            result = parse_classification_xml(xml)
            assert result is not None
            assert result.category == cat


class TestBuildRetrySystemPrompt:
    def test_adds_suffix(self):
        original = "You are a classifier."
        result = build_retry_system_prompt(original)
        assert "You must output exactly one XML block" in result
        assert result.startswith(original)


class TestClassificationResult:
    def test_valid(self):
        r = ClassificationResult(category="Movies", confidence=0.9)
        assert r.category == "Movies"

    def test_invalid_category(self):
        try:
            ClassificationResult(category="Bogus", confidence=0.9)
            assert False, "Should have raised"
        except ValueError:
            pass

    def test_invalid_confidence(self):
        try:
            ClassificationResult(category="Movies", confidence=2.0)
            assert False, "Should have raised"
        except ValueError:
            pass
