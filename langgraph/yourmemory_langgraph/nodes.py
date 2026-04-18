"""MemoryReadNode and MemoryWriteNode — drop-in LangGraph nodes."""
from __future__ import annotations

from typing import Any

from .client import YourmemoryClient, get_client


class MemoryReadNode:
    """Calls MCP `wakeup` and injects result into state['memory_context']."""

    def __init__(self, client: YourmemoryClient | None = None) -> None:
        self._client = client

    def _get_client(self) -> YourmemoryClient:
        return self._client if self._client is not None else get_client()

    def __call__(self, state: dict[str, Any]) -> dict[str, Any]:
        context = self._get_client().wakeup()
        return {"memory_context": context}


class MemoryWriteNode:
    """Calls MCP `persist` with the last assistant message from state['messages']."""

    def __init__(self, client: YourmemoryClient | None = None) -> None:
        self._client = client

    def _get_client(self) -> YourmemoryClient:
        return self._client if self._client is not None else get_client()

    def __call__(self, state: dict[str, Any]) -> dict[str, Any]:
        messages = state.get("messages") or []
        if not messages:
            return {}
        last = messages[-1]
        # Support both dict-style and LangChain BaseMessage objects
        if hasattr(last, "content"):
            content = str(last.content)
            role = getattr(last, "type", "unknown")
        elif isinstance(last, dict):
            content = str(last.get("content", ""))
            role = last.get("role", "unknown")
        else:
            content = str(last)
            role = "unknown"

        if role in ("ai", "assistant") and content:
            self._get_client().persist(content)
        return {}
