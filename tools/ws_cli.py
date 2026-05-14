#!/usr/bin/env python3
import argparse
import base64
import hashlib
import math
import os
import socket
import struct
import sys
import time
from urllib.parse import urlparse


WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
KEY_EVENT_PLAY_DELAY_SECONDS = 0.1


def parse_url(url):
    parsed = urlparse(url)
    if parsed.scheme != "ws":
        raise SystemExit("only ws:// URLs are supported")
    if not parsed.hostname:
        raise SystemExit("URL must include a host")
    port = parsed.port or 80
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    return parsed.hostname, port, path


def connect_ws(url, timeout):
    host, port, path = parse_url(url)
    sock = socket.create_connection((host, port), timeout=timeout)
    sock.settimeout(timeout)
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    request = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    )
    sock.sendall(request.encode("ascii"))

    header = bytearray()
    while not header.endswith(b"\r\n\r\n"):
        chunk = sock.recv(1)
        if not chunk:
            raise SystemExit("websocket handshake closed early")
        header += chunk
        if len(header) > 8192:
            raise SystemExit("websocket handshake header too large")

    text = header.decode("iso-8859-1")
    lines = text.split("\r\n")
    if not lines or " 101 " not in lines[0]:
        raise SystemExit(f"websocket upgrade failed: {lines[0] if lines else text!r}")
    headers = {}
    for line in lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            headers[name.lower()] = value.strip()
    expected = base64.b64encode(hashlib.sha1((key + WS_GUID).encode("ascii")).digest()).decode(
        "ascii"
    )
    if headers.get("sec-websocket-accept") != expected:
        raise SystemExit("websocket accept key mismatch")
    return sock


def recv_exact(sock, size):
    data = bytearray()
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise EOFError("websocket closed")
        data += chunk
    return bytes(data)


def send_frame(sock, opcode, payload):
    if isinstance(payload, str):
        payload = payload.encode("utf-8")
    mask = os.urandom(4)
    head = bytearray([0x80 | (opcode & 0x0F)])
    if len(payload) < 126:
        head.append(0x80 | len(payload))
    elif len(payload) <= 0xFFFF:
        head.append(0x80 | 126)
        head.extend(struct.pack("!H", len(payload)))
    else:
        head.append(0x80 | 127)
        head.extend(struct.pack("!Q", len(payload)))
    masked = bytes(byte ^ mask[i & 3] for i, byte in enumerate(payload))
    sock.sendall(bytes(head) + mask + masked)


def recv_frame(sock):
    b0, b1 = recv_exact(sock, 2)
    opcode = b0 & 0x0F
    masked = (b1 & 0x80) != 0
    size = b1 & 0x7F
    if size == 126:
        size = struct.unpack("!H", recv_exact(sock, 2))[0]
    elif size == 127:
        size = struct.unpack("!Q", recv_exact(sock, 8))[0]
    mask = recv_exact(sock, 4) if masked else b""
    payload = bytearray(recv_exact(sock, size))
    if masked:
        for i, byte in enumerate(payload):
            payload[i] = byte ^ mask[i & 3]
    return opcode, bytes(payload)


def request(sock, line):
    send_frame(sock, 0x1, line)
    while True:
        opcode, payload = recv_frame(sock)
        if opcode == 0x9:
            send_frame(sock, 0xA, payload)
            continue
        if opcode == 0x8:
            raise SystemExit("server closed websocket")
        return opcode, payload


def checked_xy(args):
    for value in (args.x, args.y):
        if value < 0:
            raise SystemExit("coordinates must be non-negative")
    return args.x, args.y


def decode_inline_journal(script):
    out = []
    escape = False
    for ch in script:
        if escape:
            if ch == "n":
                out.append("\n")
            else:
                out.append("\\")
                out.append(ch)
            escape = False
        elif ch == "\\":
            escape = True
        elif ch == ";":
            out.append("\n")
        else:
            out.append(ch)
    if escape:
        out.append("\\")
    return "".join(out)


def load_journal_script(value):
    if value.startswith("inline:"):
        return decode_inline_journal(value[len("inline:") :])
    if "\n" in value:
        return value
    with open(value, "r", encoding="utf-8") as f:
        return f.read()


def parse_wait_seconds(line_no, value):
    try:
        seconds = float(value)
    except ValueError as err:
        raise SystemExit(
            f"invalid journal wait seconds on line {line_no}: {value}: {err}"
        ) from err
    if not math.isfinite(seconds) or seconds < 0:
        raise SystemExit(f"invalid journal wait seconds on line {line_no}: {value}")
    return seconds


def parse_journal_line(line_no, raw):
    line = raw.split("#", 1)[0].strip()
    if not line:
        return []
    if line.startswith("text,"):
        return [("send", f"text {line[len('text,') :]}")]

    parts = [part.strip() for part in line.split(",") if part.strip()]
    if len(parts) == 3 and parts[0] in ("move", "down", "up", "click"):
        try:
            x = int(parts[1])
            y = int(parts[2])
        except ValueError as err:
            raise SystemExit(f"invalid journal coordinate on line {line_no}: {err}") from err
        if x < 0 or y < 0:
            raise SystemExit(
                f"invalid journal coordinate on line {line_no}: coordinates must be non-negative"
            )
        return [("send", f"{parts[0]} {x} {y}")]
    if len(parts) == 2 and parts[0] == "key":
        key = parts[1]
        return [
            ("send", f"keydown {key}"),
            ("delay", KEY_EVENT_PLAY_DELAY_SECONDS),
            ("send", f"keyup {key}"),
            ("delay", KEY_EVENT_PLAY_DELAY_SECONDS),
        ]
    if len(parts) == 2 and parts[0] in ("keydown", "keyup"):
        return [
            ("send", f"{parts[0]} {parts[1]}"),
            ("delay", KEY_EVENT_PLAY_DELAY_SECONDS),
        ]
    if len(parts) == 2 and parts[0] == "wait":
        return [("delay", parse_wait_seconds(line_no, parts[1]))]

    raise SystemExit(
        f"invalid journal line {line_no}: expected move,x,y, down,x,y, up,x,y, click,x,y, "
        "key,key, keydown,key, keyup,key, text,value, or wait,seconds"
    )


def play_journal(sock, script):
    count = 0
    for line_no, raw in enumerate(script.splitlines(), 1):
        for kind, value in parse_journal_line(line_no, raw):
            if kind == "delay":
                time.sleep(value)
                continue
            opcode, payload = request(sock, value)
            text = payload.decode("utf-8", "replace") if opcode == 0x1 else ""
            if opcode != 0x1 or text.startswith("err"):
                raise SystemExit(
                    text or f"unexpected binary websocket response ({len(payload)} bytes)"
                )
            count += 1
    return count


def main():
    parser = argparse.ArgumentParser(description="Control the wemu SDL2 WebSocket harness.")
    parser.add_argument("--url", default="ws://127.0.0.1:8765")
    parser.add_argument("--timeout", type=float, default=5.0)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("ping")

    screenshot = sub.add_parser("screenshot")
    screenshot.add_argument("out")

    for name in ("move", "down", "up", "click", "rightdown", "rightup", "rightclick"):
        cmd = sub.add_parser(name)
        cmd.add_argument("x", type=int)
        cmd.add_argument("y", type=int)

    text = sub.add_parser("text")
    text.add_argument("value")

    for name in ("keydown", "keyup", "key"):
        cmd = sub.add_parser(name)
        cmd.add_argument("value")

    raw = sub.add_parser("raw")
    raw.add_argument("line")

    play = sub.add_parser("play")
    play.add_argument(
        "journal", help="journal path, inline:script, or a literal multiline script"
    )

    args = parser.parse_args()
    sock = connect_ws(args.url, args.timeout)
    try:
        if args.command == "play":
            count = play_journal(sock, load_journal_script(args.journal))
            print(f"ok played {count} events")
            return 0
        elif args.command == "ping":
            line = "ping"
        elif args.command == "screenshot":
            line = "screenshot"
        elif args.command == "text":
            line = f"text {args.value}"
        elif args.command in ("keydown", "keyup", "key"):
            line = f"{args.command} {args.value}"
        elif args.command == "raw":
            line = args.line
        else:
            x, y = checked_xy(args)
            line = f"{args.command} {x} {y}"

        opcode, payload = request(sock, line)
        if args.command == "screenshot":
            if opcode != 0x2:
                sys.stderr.write(payload.decode("utf-8", "replace"))
                return 1
            with open(args.out, "wb") as f:
                f.write(payload)
            print(args.out)
        elif opcode == 0x1:
            sys.stdout.write(payload.decode("utf-8", "replace"))
        else:
            print(f"binary {len(payload)} bytes")
    finally:
        try:
            send_frame(sock, 0x8, b"")
        except OSError:
            pass
        sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
