import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.core.prompt_builder import PromptBuilder
from src.core.types import TorrentInput


class TestPromptBuilder:
    def _make_config(self, tmp_path):
        system_file = tmp_path / "system.txt"
        system_file.write_text("You are a classifier.")

        fewshot_file = tmp_path / "fewshot.jsonl"
        example = {
            "user": "<torrent>example</torrent>",
            "assistant": "<classification><category>Movies</category><confidence>0.9</confidence></classification>",
        }
        fewshot_file.write_text(json.dumps(example) + "\n")

        return {
            "prompt": {
                "system_file": str(system_file),
                "fewshot_file": str(fewshot_file),
                "max_input_files": 20,
                "max_file_name_chars": 200,
                "max_torrent_name_chars": 500,
            }
        }

    def test_build_messages(self, tmp_path):
        config = self._make_config(tmp_path)
        builder = PromptBuilder(config)

        torrent = TorrentInput(
            infohash="abc123",
            name="Test Movie",
            file_count=1,
            total_size_bytes=1000,
            files=["test.mkv"],
        )
        messages = builder.build_messages(torrent)

        assert messages[0]["role"] == "system"
        assert messages[0]["content"] == "You are a classifier."

        assert messages[1]["role"] == "user"
        assert messages[2]["role"] == "assistant"

        last = messages[-1]
        assert last["role"] == "user"
        assert "Test Movie" in last["content"]
        assert "<torrent>" in last["content"]

    def test_build_retry_messages(self, tmp_path):
        config = self._make_config(tmp_path)
        builder = PromptBuilder(config)

        torrent = TorrentInput(
            infohash="abc",
            name="Test",
            file_count=1,
            total_size_bytes=1000,
            files=[],
        )
        messages = builder.build_retry_messages(torrent)
        assert "You must output exactly one XML block" in messages[0]["content"]

    def test_truncates_large_file_list(self, tmp_path):
        config = self._make_config(tmp_path)
        config["prompt"]["max_input_files"] = 3
        builder = PromptBuilder(config)

        torrent = TorrentInput(
            infohash="abc",
            name="Test",
            file_count=10,
            total_size_bytes=1000,
            files=[f"file{i}.mkv" for i in range(10)],
        )
        messages = builder.build_messages(torrent)
        user_msg = messages[-1]["content"]
        assert user_msg.count("<file>") == 3
