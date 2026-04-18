"""
Minimal example: wire MemoryReadNode and MemoryWriteNode into a LangGraph graph.

The graph has three nodes:
    memory_read  →  chat  →  memory_write

- memory_read:  calls yourmemory-mcp `wakeup` and stores context in state
- chat:         a stub LLM node (replace with your real LLM call)
- memory_write: calls yourmemory-mcp `persist` with the last assistant message
"""
from __future__ import annotations

from typing import Annotated

from langgraph.graph import StateGraph, END
from langgraph.graph.message import add_messages
from typing_extensions import TypedDict

from yourmemory_langgraph import MemoryReadNode, MemoryWriteNode, MemoryState


# ── State ─────────────────────────────────────────────────────────────────────

class AppState(MemoryState):
    messages: Annotated[list, add_messages]


# ── Stub chat node ────────────────────────────────────────────────────────────

def chat_node(state: AppState) -> dict:
    context = state.get("memory_context", "")
    user_text = ""
    for m in reversed(state.get("messages", [])):
        role = m.get("role") if isinstance(m, dict) else getattr(m, "type", "")
        if role == "human":
            user_text = m.get("content") if isinstance(m, dict) else m.content
            break
    reply = f"[stub reply to '{user_text}' | context: {context[:60] or 'none'}]"
    return {"messages": [{"role": "assistant", "content": reply}]}


# ── Build graph ───────────────────────────────────────────────────────────────

def build_graph():
    memory_read = MemoryReadNode()
    memory_write = MemoryWriteNode()

    g = StateGraph(AppState)
    g.add_node("memory_read", memory_read)
    g.add_node("chat", chat_node)
    g.add_node("memory_write", memory_write)

    g.set_entry_point("memory_read")
    g.add_edge("memory_read", "chat")
    g.add_edge("chat", "memory_write")
    g.add_edge("memory_write", END)

    return g.compile()


if __name__ == "__main__":
    graph = build_graph()
    print("Graph compiled successfully.")
    print("Nodes:", list(graph.nodes))
    print(
        "\nTo run a turn (requires yourmemory-mcp binary):\n"
        "  result = graph.invoke({'messages': [{'role': 'human', 'content': 'hello'}]})\n"
        "  print(result['messages'][-1])"
    )
