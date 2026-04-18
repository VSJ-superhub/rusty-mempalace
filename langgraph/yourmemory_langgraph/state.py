from __future__ import annotations

from typing import Annotated
from langgraph.graph.message import add_messages
from typing_extensions import TypedDict


class MemoryState(TypedDict, total=False):
    """Base state mixin that adds a memory_context field to any LangGraph state."""
    memory_context: str
    messages: Annotated[list, add_messages]
