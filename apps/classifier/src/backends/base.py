from __future__ import annotations

from abc import ABC, abstractmethod


class ModelBackend(ABC):
    @abstractmethod
    def generate(self, messages: list[dict]) -> str:
        ...
