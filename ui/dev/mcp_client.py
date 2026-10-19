"""Development only: an MCP client over stdio, driven by ui/dev/integrate.mjs.

Starts `target\\release\\kurasi.exe mcp --project <folder> --mode <mode>` (or the exe named by
KURASI_EXE) as a subprocess through the stdio client of the `mcp` package, then reads one JSON
object per line from stdin and answers each with one JSON line on stdout:

    {"tool": "<name>", "args": {...}}   calls the tool
    {"quit": true}                      closes the session

The first line written reports the start times. KURASI_HOME must point at a temporary app-data
folder: the client refuses to run against the real %LOCALAPPDATA%\\Kurasi, and it passes
KURASI_HOME to the server explicitly because the stdio client hands a subprocess only a short list
of inherited environment variables.

The mode defaults to write, the default of `kurasi.exe mcp` too; the integration run calls
highlight_add, a write tool.

KURASI_MCP_CONNECT picks the handshake of the `mcp` package: `auto` (the default: server/discover
of protocol 2026-07-28, the client name travels in the `_meta` of every request) or `legacy` (the
initialize handshake, the client name travels once in `clientInfo`). The ready line reports the
negotiated protocol version.

Usage: python mcp_client.py <project folder or id> [client name] [read|propose|write]
"""

import asyncio
import json
import os
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
KURASI_EXE = os.environ.get("KURASI_EXE") or os.path.join(REPO, "target", "release", "kurasi.exe")


def emit(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def guard_home():
    home = os.environ.get("KURASI_HOME")
    real = os.path.join(os.environ.get("LOCALAPPDATA") or "", "Kurasi")
    if not home:
        raise SystemExit("Set KURASI_HOME to a temporary folder first.")
    if os.path.normcase(os.path.abspath(home)) == os.path.normcase(os.path.abspath(real)):
        raise SystemExit("KURASI_HOME points at the real app data; use a temporary folder.")
    return home


async def main(folder, client_name, mode):
    home = guard_home()
    t0 = time.perf_counter()
    from mcp import Client, StdioServerParameters
    import mcp.types as T
    from mcp.client.stdio import get_default_environment
    t_import = time.perf_counter() - t0
    env = dict(get_default_environment(), KURASI_HOME=home, PYTHONIOENCODING="utf-8")
    params = StdioServerParameters(command=KURASI_EXE, args=["mcp", "--project", folder, "--mode", mode],
                                   env=env, cwd=os.path.dirname(os.path.abspath(home)))
    connect = os.environ.get("KURASI_MCP_CONNECT") or "auto"
    t1 = time.perf_counter()
    async with Client(params, client_info=T.Implementation(name=client_name, version="1"),
                      read_timeout_seconds=120, mode=connect) as c:
        t_init = time.perf_counter() - t1
        tools = [t.name for t in (await c.list_tools()).tools]
        t_tools = time.perf_counter() - t1
        emit({"ready": True, "client_import_s": round(t_import, 3), "spawn_to_initialized_s": round(t_init, 3),
              "spawn_to_tools_s": round(t_tools, 3), "mode": mode, "connect": connect,
              "protocol": c.protocol_version, "tools": tools})
        while True:
            line = await asyncio.to_thread(sys.stdin.readline)
            if not line:
                break
            line = line.strip()
            if not line:
                continue
            msg = json.loads(line)
            if msg.get("quit"):
                break
            s = time.perf_counter()
            try:
                res = await c.call_tool(msg["tool"], msg.get("args") or {})
                text = res.content[0].text if res.content else ""
                try:
                    data = json.loads(text)
                except ValueError:
                    data = text
                emit({"tool": msg["tool"], "ok": not res.is_error, "data": data,
                      "s": round(time.perf_counter() - s, 3)})
            except Exception as exc:
                emit({"tool": msg.get("tool"), "ok": False, "data": "%s: %s" % (type(exc).__name__, exc)})
    emit({"closed": True})


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    asyncio.run(main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "kurasi-integration",
                     sys.argv[3] if len(sys.argv) > 3 else "write"))
