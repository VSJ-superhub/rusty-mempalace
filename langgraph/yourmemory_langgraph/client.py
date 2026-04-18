"""Manages a single yourmemory-mcp subprocess and exposes wakeup/persist calls."""
from __future__ import annotations

import atexit
import json
import os
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any


_DEFAULT_BINARY = os.environ.get(
    "YOURMEMORY_MCP_BIN",
    str(Path(__file__).parent.parent.parent / "target" / "debug" / "yourmemory-mcp"),
)


class McpError(RuntimeError):
    pass


class YourmemoryClient:
    """Subprocess MCP client.  One instance per process; thread-safe."""

    def __init__(self, binary: str = _DEFAULT_BINARY) -> None:
        self._binary = binary
        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()
        self._id = 0
        atexit.register(self.shutdown)

    # ── lifecycle ────────────────────────────────────────────────────────────

    def _ensure_started(self) -> None:
        if self._proc and self._proc.poll() is None:
            return
        self._proc = subprocess.Popen(
            [self._binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self._initialize()

    def shutdown(self) -> None:
        if self._proc and self._proc.poll() is None:
            try:
                self._proc.stdin.close()  # type: ignore[union-attr]
                self._proc.wait(timeout=3)
            except Exception:
                self._proc.kill()
        self._proc = None

    # ── transport ────────────────────────────────────────────────────────────

    def _send(self, payload: dict[str, Any]) -> dict[str, Any]:
        body = json.dumps(payload).encode()
        header = f"Content-Length: {len(body)}\r\n\r\n".encode()
        assert self._proc and self._proc.stdin and self._proc.stdout
        self._proc.stdin.write(header + body)
        self._proc.stdin.flush()
        return self._read_response()

    def _read_response(self) -> dict[str, Any]:
        stdout = self._proc.stdout  # type: ignore[union-attr]
        # Read headers
        content_length = 0
        while True:
            line = stdout.readline().decode()
            if not line.strip():
                break
            if line.lower().startswith("content-length:"):
                content_length = int(line.split(":", 1)[1].strip())
        if content_length == 0:
            raise McpError("MCP server returned empty response")
        raw = stdout.read(content_length)
        return json.loads(raw)

    def _next_id(self) -> int:
        self._id += 1
        return self._id

    # ── MCP methods ──────────────────────────────────────────────────────────

    def _initialize(self) -> None:
        resp = self._send({
            "jsonrpc": "2.0",
            "id": self._next_id(),
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "yourmemory-langgraph", "version": "0.1.0"},
            },
        })
        if "error" in resp:
            raise McpError(f"initialize failed: {resp['error']}")
        # Send initialized notification (no response expected)
        body = json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}).encode()
        header = f"Content-Length: {len(body)}\r\n\r\n".encode()
        self._proc.stdin.write(header + body)  # type: ignore[union-attr]
        self._proc.stdin.flush()

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None) -> Any:
        with self._lock:
            self._ensure_started()
            resp = self._send({
                "jsonrpc": "2.0",
                "id": self._next_id(),
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments or {}},
            })
        if "error" in resp:
            raise McpError(f"tools/call {name!r} failed: {resp['error']}")
        return resp.get("result", {})

    # ── high-level helpers ────────────────────────────────────────────────────

    def wakeup(self) -> str:
        """Return the memory context string for the current session."""
        result = self.call_tool("wakeup")
        content = result.get("content", [])
        parts = [c.get("text", "") for c in content if c.get("type") == "text"]
        return "\n".join(parts)

    def persist(self, message: str) -> None:
        """Persist an assistant message to long-term memory."""
        self.call_tool("persist", {"message": message})


# Module-level singleton — lazy-initialised on first use.
_client: YourmemoryClient | None = None
_client_lock = threading.Lock()


def get_client(binary: str = _DEFAULT_BINARY) -> YourmemoryClient:
    global _client
    with _client_lock:
        if _client is None:
            _client = YourmemoryClient(binary)
    return _client
