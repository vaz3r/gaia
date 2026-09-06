"""Unofficial OpenAI-compatible client for chat.deepseek.com."""

from .auth import Session, get_session, login
from .client import DeepSeekClient, RateLimitError, Reply
from .pow import DeepSeekPow

__all__ = ["Session", "get_session", "login", "DeepSeekClient", "RateLimitError", "Reply", "DeepSeekPow"]
