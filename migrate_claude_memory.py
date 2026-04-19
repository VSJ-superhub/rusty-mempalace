"""
Migrate Claude Code file-based memories into yourmemory palace.

Wings/rooms layout:
  feedback/   → pipeline lessons, architecture rules, dispatch rules, etc.
  project/    → project state, architecture pointers
  reference/  → external resource pointers
  user/       → user profile facts
"""
import json
import subprocess
import sys
import os
from pathlib import Path

MCP_EXE = r"C:\Users\alway\Projects\yourmemory\target\debug\yourmemory-mcp.exe"
MEMORY_DIR = Path(r"C:\Users\alway\.claude\projects\c--Users-alway-Projects-eng-team\memory")
# Store memories in the global palace (no project_path → falls back to ~/.yourmemory/global/)
PALACE_PATH = str(Path.home() / ".yourmemory" / "global")

_id = 0

def next_id():
    global _id
    _id += 1
    return _id

def rpc(proc, method, params):
    req = {"jsonrpc": "2.0", "id": next_id(), "method": method, "params": params}
    line = json.dumps(req) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()
    raw = proc.stdout.readline()
    return json.loads(raw)

def call_tool(proc, name, arguments):
    return rpc(proc, "tools/call", {"name": name, "arguments": arguments})

def parse_frontmatter(text):
    lines = text.strip().splitlines()
    if not lines or lines[0].strip() != "---":
        return {}, text
    end = None
    for i, line in enumerate(lines[1:], 1):
        if line.strip() == "---":
            end = i
            break
    if end is None:
        return {}, text
    fm = {}
    for line in lines[1:end]:
        if ":" in line:
            k, _, v = line.partition(":")
            fm[k.strip()] = v.strip()
    body = "\n".join(lines[end + 1:]).strip()
    return fm, body

def type_to_wing(mem_type):
    mapping = {
        "feedback": "feedback",
        "project": "project",
        "reference": "reference",
        "user": "user",
    }
    return mapping.get(mem_type, "general")

def main():
    proc = subprocess.Popen(
        [MCP_EXE],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Handshake — initialize expects a response, initialized is a notification (no response)
    rpc(proc, "initialize", {"protocolVersion": "2024-11-05", "clientInfo": {"name": "migrator", "version": "1.0"}})
    notify = json.dumps({"jsonrpc": "2.0", "method": "initialized", "params": {}}) + "\n"
    proc.stdin.write(notify.encode())
    proc.stdin.flush()

    md_files = [f for f in MEMORY_DIR.glob("*.md") if f.name != "MEMORY.md"]
    print(f"Found {len(md_files)} memory files to migrate")

    for md_path in sorted(md_files):
        text = md_path.read_text(encoding="utf-8")
        fm, body = parse_frontmatter(text)

        name = fm.get("name", md_path.stem)
        description = fm.get("description", "")
        mem_type = fm.get("type", "general")
        wing = type_to_wing(mem_type)
        room = md_path.stem  # use filename as room for precise recall

        content = f"# {name}\n{description}\n\n{body}".strip()

        resp = call_tool(proc, "store_fact", {
            "wing": wing,
            "room": room,
            "content": content,
            "source": "config",
            "confidence": "high",
        })
        result_text = ""
        if "result" in resp:
            content_arr = resp["result"].get("content", [])
            result_text = content_arr[0].get("text", "") if content_arr else str(resp["result"])
        print(f"  [{mem_type}] {md_path.name} → {wing}/{room}: {result_text[:80]}")

    print("\nMigration complete.")
    proc.stdin.close()
    proc.wait()

if __name__ == "__main__":
    main()
