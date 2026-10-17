#!/usr/bin/env python3
"""Query Gin LSP hover and diagnostics for a source position.

Example:
  python3 tools/ginlsp/scripts/inspect_lsp.py path/to/mod.flask path/to/file.gin 12 8

Line and column arguments are one-based. Output is one JSON object on stdout.
The script uses only Python's standard library and communicates with ginlsp over
its normal LSP stdio transport.
"""

import argparse
import json
import os
import select
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import quote


class LspClient:
    def __init__(self, process):
        self.process = process
        self.buffer = bytearray()
        self.next_id = 1

    def send(self, message):
        body = json.dumps(message, separators=(",", ":")).encode()
        self.process.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        self.process.stdin.flush()

    def request(self, method, params, timeout):
        request_id = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            message = self.receive(deadline - time.monotonic())
            if message is None:
                continue
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(f"LSP {method} failed: {message['error']}")
                return message.get("result")
        raise TimeoutError(f"timed out waiting for LSP {method}")

    def receive(self, timeout):
        while b"\r\n\r\n" not in self.buffer:
            if not self._read(timeout):
                return None
        header, _, remainder = self.buffer.partition(b"\r\n\r\n")
        length = None
        for line in header.split(b"\r\n"):
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1].strip())
        if length is None:
            raise RuntimeError("LSP server sent a message without Content-Length")
        while len(remainder) < length:
            if not self._read(timeout):
                return None
            _, _, remainder = self.buffer.partition(b"\r\n\r\n")
        payload = bytes(remainder[:length])
        self.buffer = bytearray(remainder[length:])
        return json.loads(payload)

    def _read(self, timeout):
        ready, _, _ = select.select([self.process.stdout], [], [], max(0, timeout))
        if not ready:
            return False
        chunk = os.read(self.process.stdout.fileno(), 65536)
        if not chunk:
            raise RuntimeError("ginlsp exited before responding")
        self.buffer.extend(chunk)
        return True


def file_uri(path):
    path = path.resolve()
    return "file://" + quote(str(path), safe="/")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path, help="directory containing mod.flask")
    parser.add_argument("file", type=Path, help="Gin source file to inspect")
    parser.add_argument("line", type=int, help="one-based source line")
    parser.add_argument("column", type=int, help="one-based source column")
    parser.add_argument("--server", type=Path, help="ginlsp executable (default: target/debug/ginlsp)")
    parser.add_argument("--timeout", type=float, default=30, help="seconds to wait for each LSP response")
    args = parser.parse_args()

    package = args.package.resolve()
    source_file = args.file.resolve()
    if args.line < 1 or args.column < 1:
        parser.error("line and column must be one-based positive integers")
    if not (package / "mod.flask").is_file():
        parser.error(f"{package} does not contain mod.flask")
    try:
        relative_file = source_file.relative_to(package)
    except ValueError:
        parser.error("file must be inside the package directory")

    server = args.server.resolve() if args.server else Path(__file__).resolve().parents[3] / "target/debug/ginlsp"
    if not server.is_file():
        parser.error(f"{server} not found; build it with: cargo build -p ginlsp")

    source = source_file.read_text()
    uri = file_uri(source_file)
    process = subprocess.Popen(
        [str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr,
        cwd=package,
    )
    client = LspClient(process)
    diagnostics = None
    try:
        client.request("initialize", {
            "processId": os.getpid(),
            "rootUri": file_uri(package),
            "workspaceFolders": [{"uri": file_uri(package), "name": package.name}],
            "capabilities": {},
        }, args.timeout)
        client.send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        client.send({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": {"uri": uri, "languageId": "gin", "version": 1, "text": source}
        }})

        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            message = client.receive(deadline - time.monotonic())
            if message and message.get("method") == "textDocument/publishDiagnostics":
                if message.get("params", {}).get("uri") == uri:
                    diagnostics = message["params"].get("diagnostics", [])
                    break
        if diagnostics is None:
            raise TimeoutError(f"timed out waiting for diagnostics for {relative_file}")

        hover = client.request("textDocument/hover", {
            "textDocument": {"uri": uri},
            "position": {"line": args.line - 1, "character": args.column - 1},
        }, args.timeout)
        print(json.dumps({
            "file": str(relative_file),
            "position": {"line": args.line, "column": args.column},
            "hover": hover,
            "diagnostics": diagnostics,
        }, indent=2))
    finally:
        try:
            client.request("shutdown", None, min(args.timeout, 3))
            client.send({"jsonrpc": "2.0", "method": "exit", "params": None})
        except (BrokenPipeError, OSError, RuntimeError, TimeoutError):
            pass
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"inspect_lsp: {error}", file=sys.stderr)
        sys.exit(1)
