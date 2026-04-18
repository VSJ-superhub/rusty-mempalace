"""Tests for MemoryReadNode and MemoryWriteNode — MCP subprocess is mocked."""
from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from yourmemory_langgraph import MemoryReadNode, MemoryWriteNode, MemoryState
from yourmemory_langgraph.client import YourmemoryClient


# ── helpers ──────────────────────────────────────────────────────────────────

def _make_client(wakeup_return: str = "prior context", persist_ok: bool = True) -> YourmemoryClient:
    client = MagicMock(spec=YourmemoryClient)
    client.wakeup.return_value = wakeup_return
    client.persist.return_value = None
    return client


# ── MemoryReadNode ────────────────────────────────────────────────────────────

class TestMemoryReadNode:
    def test_injects_memory_context(self):
        client = _make_client("some memory")
        node = MemoryReadNode(client=client)
        result = node({})
        assert result == {"memory_context": "some memory"}
        client.wakeup.assert_called_once()

    def test_empty_wakeup(self):
        client = _make_client("")
        node = MemoryReadNode(client=client)
        result = node({"messages": []})
        assert result["memory_context"] == ""

    def test_uses_module_singleton_when_no_client(self):
        mock_client = _make_client("ctx")
        with patch("yourmemory_langgraph.nodes.get_client", return_value=mock_client):
            node = MemoryReadNode()
            result = node({})
        assert result["memory_context"] == "ctx"


# ── MemoryWriteNode ───────────────────────────────────────────────────────────

class TestMemoryWriteNode:
    def test_persists_last_assistant_dict_message(self):
        client = _make_client()
        node = MemoryWriteNode(client=client)
        state = {"messages": [
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi there"},
        ]}
        result = node(state)
        assert result == {}
        client.persist.assert_called_once_with("hi there")

    def test_skips_user_message(self):
        client = _make_client()
        node = MemoryWriteNode(client=client)
        state = {"messages": [{"role": "user", "content": "hello"}]}
        node(state)
        client.persist.assert_not_called()

    def test_persists_langchain_ai_message(self):
        msg = MagicMock()
        msg.content = "LangChain response"
        msg.type = "ai"
        client = _make_client()
        node = MemoryWriteNode(client=client)
        node({"messages": [msg]})
        client.persist.assert_called_once_with("LangChain response")

    def test_empty_messages(self):
        client = _make_client()
        node = MemoryWriteNode(client=client)
        result = node({"messages": []})
        assert result == {}
        client.persist.assert_not_called()

    def test_no_messages_key(self):
        client = _make_client()
        node = MemoryWriteNode(client=client)
        result = node({})
        assert result == {}
        client.persist.assert_not_called()

    def test_uses_module_singleton_when_no_client(self):
        mock_client = _make_client()
        with patch("yourmemory_langgraph.nodes.get_client", return_value=mock_client):
            node = MemoryWriteNode()
            node({"messages": [{"role": "assistant", "content": "hi"}]})
        mock_client.persist.assert_called_once_with("hi")


# ── YourmemoryClient (unit) ───────────────────────────────────────────────────

class TestYourmemoryClient:
    def _make_proc(self, responses: list[bytes]) -> MagicMock:
        """Build a mock Popen that returns given JSON-RPC responses in order."""
        proc = MagicMock()
        proc.poll.return_value = None
        proc.stdin = MagicMock()
        # Each response is framed with Content-Length header
        framed = []
        for body in responses:
            framed.append(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        # readline yields header lines then blank line per response
        read_lines = []
        read_chunks = []
        for body in responses:
            read_lines.append(f"Content-Length: {len(body)}\r\n".encode())
            read_lines.append(b"\r\n")
            read_chunks.append(body)
        proc.stdout.readline.side_effect = read_lines
        proc.stdout.read.side_effect = read_chunks
        return proc

    def test_wakeup_extracts_text(self):
        import json as _json
        init_resp = _json.dumps({
            "jsonrpc": "2.0", "id": 1,
            "result": {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "t", "version": "0"}},
        }).encode()
        wakeup_resp = _json.dumps({
            "jsonrpc": "2.0", "id": 2,
            "result": {"content": [{"type": "text", "text": "remembered context"}]},
        }).encode()
        proc = self._make_proc([init_resp, wakeup_resp])
        client = YourmemoryClient(binary="/fake/bin")
        with patch("subprocess.Popen", return_value=proc):
            result = client.wakeup()
        assert result == "remembered context"

    def test_persist_calls_tool(self):
        import json as _json
        init_resp = _json.dumps({
            "jsonrpc": "2.0", "id": 1,
            "result": {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "t", "version": "0"}},
        }).encode()
        persist_resp = _json.dumps({
            "jsonrpc": "2.0", "id": 2, "result": {}
        }).encode()
        proc = self._make_proc([init_resp, persist_resp])
        client = YourmemoryClient(binary="/fake/bin")
        with patch("subprocess.Popen", return_value=proc):
            client.persist("hello world")
        calls = proc.stdin.write.call_args_list
        written = b"".join(c.args[0] for c in calls)
        assert b'"persist"' in written
        assert b"hello world" in written
