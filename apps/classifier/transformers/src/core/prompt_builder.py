from __future__ import annotations

import json
from pathlib import Path

from .xml_guardrails import build_torrent_xml
from .types import TorrentInput


class PromptBuilder:
    def __init__(self, config: dict):
        prompt_cfg = config.get("prompt", {})
        self.system_prompt = self._load_text(prompt_cfg.get("system_file", "prompts/system.txt"))
        self.fewshot = self._load_fewshot(prompt_cfg.get("fewshot_file", "prompts/fewshot.jsonl"))
        self.config = config

    def build_messages(self, torrent: TorrentInput) -> list[dict]:
        messages = [{"role": "system", "content": self.system_prompt}]

        for example in self.fewshot:
            messages.append({"role": "user", "content": example["user"]})
            messages.append({"role": "assistant", "content": example["assistant"]})

        torrent_xml = build_torrent_xml(torrent, self.config)
        messages.append({"role": "user", "content": f"Torrent metadata:\n{torrent_xml}"})

        return messages

    def build_retry_messages(self, torrent: TorrentInput) -> list[dict]:
        from .xml_guardrails import build_retry_system_prompt

        messages = [{"role": "system", "content": build_retry_system_prompt(self.system_prompt)}]

        for example in self.fewshot:
            messages.append({"role": "user", "content": example["user"]})
            messages.append({"role": "assistant", "content": example["assistant"]})

        torrent_xml = build_torrent_xml(torrent, self.config)
        messages.append({"role": "user", "content": f"Torrent metadata:\n{torrent_xml}"})

        return messages

    @staticmethod
    def _load_text(path: str) -> str:
        return Path(path).read_text(encoding="utf-8").strip()

    @staticmethod
    def _load_fewshot(path: str) -> list[dict]:
        examples = []
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    examples.append(json.loads(line))
        return examples
