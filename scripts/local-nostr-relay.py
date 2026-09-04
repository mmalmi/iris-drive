#!/usr/bin/env python3
"""Tiny dependency-free local Nostr relay for offline smoke tests."""

from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import json
from pathlib import Path
import signal
import struct
import time


WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def event_matches_filter(event: dict, relay_filter: dict) -> bool:
    kinds = relay_filter.get("kinds")
    if kinds is not None and event.get("kind") not in kinds:
        return False
    authors = relay_filter.get("authors")
    if authors is not None and event.get("pubkey") not in authors:
        return False
    ids = relay_filter.get("ids")
    if ids is not None and event.get("id") not in ids:
        return False
    since = relay_filter.get("since")
    if since is not None and event.get("created_at", 0) < since:
        return False
    for key, values in relay_filter.items():
        if not key.startswith("#"):
            continue
        tag_name = key[1:]
        event_tags = event.get("tags") or []
        if not any(
            len(tag) > 1 and tag[0] == tag_name and tag[1] in values
            for tag in event_tags
        ):
            return False
    return True


async def read_http_upgrade(reader: asyncio.StreamReader) -> dict[str, str]:
    request = await reader.readline()
    if not request.startswith(b"GET "):
        raise ValueError("websocket upgrade must use GET")
    headers: dict[str, str] = {}
    while True:
        line = await reader.readline()
        if line in {b"\r\n", b"\n", b""}:
            break
        name, separator, value = line.decode("latin-1").partition(":")
        if not separator:
            raise ValueError("invalid websocket upgrade header")
        headers[name.strip().lower()] = value.strip()
    return headers


async def accept_websocket(
    reader: asyncio.StreamReader, writer: asyncio.StreamWriter
) -> None:
    headers = await read_http_upgrade(reader)
    key = headers.get("sec-websocket-key")
    if not key or headers.get("sec-websocket-version") != "13":
        raise ValueError("unsupported websocket handshake")
    accept = base64.b64encode(
        hashlib.sha1(f"{key}{WEBSOCKET_GUID}".encode()).digest()
    ).decode()
    writer.write(
        (
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
        ).encode()
    )
    await writer.drain()


async def read_frame(reader: asyncio.StreamReader) -> tuple[bool, int, bytes]:
    first, second = await reader.readexactly(2)
    final = bool(first & 0x80)
    opcode = first & 0x0F
    if first & 0x70:
        raise ValueError("websocket extensions are unsupported")
    if not second & 0x80:
        raise ValueError("client websocket frames must be masked")
    length = second & 0x7F
    if length == 126:
        length = struct.unpack("!H", await reader.readexactly(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", await reader.readexactly(8))[0]
    mask = await reader.readexactly(4)
    payload = await reader.readexactly(length)
    payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    return final, opcode, payload


async def send_frame(writer: asyncio.StreamWriter, opcode: int, payload: bytes) -> None:
    header = bytearray([0x80 | opcode])
    if len(payload) < 126:
        header.append(len(payload))
    elif len(payload) <= 0xFFFF:
        header.append(126)
        header.extend(struct.pack("!H", len(payload)))
    else:
        header.append(127)
        header.extend(struct.pack("!Q", len(payload)))
    writer.write(header + payload)
    await writer.drain()


async def send_json(writer: asyncio.StreamWriter, value: object) -> None:
    payload = json.dumps(value, separators=(",", ":")).encode("utf-8")
    await send_frame(writer, 1, payload)


class LocalRelay:
    def __init__(self, event_log: Path | None) -> None:
        self.events: list[dict] = []
        self.event_log = event_log
        self.connections: set[RelayConnection] = set()

    def record_event(self, event: dict) -> None:
        self.events.append(event)
        if self.event_log is None:
            return
        tag_names = [
            tag[0]
            for tag in event.get("tags") or []
            if isinstance(tag, list) and tag and isinstance(tag[0], str)
        ]
        record = {
            "sequence": len(self.events),
            "id": event.get("id"),
            "pubkey": event.get("pubkey"),
            "kind": event.get("kind"),
            "created_at": event.get("created_at"),
            "received_at": time.time(),
            "tag_names": tag_names,
        }
        with self.event_log.open("a", encoding="utf-8") as output:
            output.write(json.dumps(record, separators=(",", ":")) + "\n")

    async def broadcast_event(self, event: dict) -> None:
        deliveries = [
            (connection, subscription_id)
            for connection in tuple(self.connections)
            for subscription_id, filters in connection.subscriptions.items()
            if any(event_matches_filter(event, relay_filter) for relay_filter in filters)
        ]
        for connection, subscription_id in deliveries:
            try:
                await connection.send(["EVENT", subscription_id, event])
            except (ConnectionError, asyncio.IncompleteReadError):
                self.connections.discard(connection)

    async def handle(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        fragments = bytearray()
        fragment_opcode: int | None = None
        connection: RelayConnection | None = None
        try:
            await accept_websocket(reader, writer)
            connection = RelayConnection(writer)
            self.connections.add(connection)
            while True:
                final, opcode, payload = await read_frame(reader)
                if opcode == 8:
                    await send_frame(writer, 8, payload[:125])
                    break
                if opcode == 9:
                    await send_frame(writer, 10, payload[:125])
                    continue
                if opcode == 10:
                    continue
                if opcode in {1, 2}:
                    fragments = bytearray(payload)
                    fragment_opcode = opcode
                elif opcode == 0 and fragment_opcode is not None:
                    fragments.extend(payload)
                else:
                    raise ValueError("unsupported websocket frame sequence")
                if not final:
                    continue
                if fragment_opcode != 1:
                    raise ValueError("Nostr messages must be websocket text frames")
                message = fragments.decode("utf-8")
                fragments.clear()
                fragment_opcode = None
                try:
                    command = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if not isinstance(command, list) or not command:
                    continue
                if (
                    command[0] == "EVENT"
                    and len(command) >= 2
                    and isinstance(command[1], dict)
                ):
                    event = command[1]
                    self.record_event(event)
                    await self.broadcast_event(event)
                    await connection.send(["OK", event.get("id", ""), True, ""])
                elif command[0] == "REQ" and len(command) >= 2:
                    subscription_id = command[1]
                    filters = [item for item in command[2:] if isinstance(item, dict)]
                    connection.subscriptions[subscription_id] = filters
                    replay = list(self.events)
                    async with connection.send_lock:
                        for event in replay:
                            if any(
                                event_matches_filter(event, relay_filter)
                                for relay_filter in filters
                            ):
                                await send_json(writer, ["EVENT", subscription_id, event])
                        await send_json(writer, ["EOSE", subscription_id])
                elif command[0] == "CLOSE":
                    if len(command) >= 2:
                        connection.subscriptions.pop(command[1], None)
        except (asyncio.IncompleteReadError, ConnectionError, ValueError):
            pass
        finally:
            if connection is not None:
                self.connections.discard(connection)
            writer.close()
            try:
                await writer.wait_closed()
            except ConnectionError:
                pass


class RelayConnection:
    def __init__(self, writer: asyncio.StreamWriter) -> None:
        self.writer = writer
        self.subscriptions: dict[object, list[dict]] = {}
        self.send_lock = asyncio.Lock()

    async def send(self, value: object) -> None:
        async with self.send_lock:
            await send_json(self.writer, value)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ready-file", type=Path, required=True)
    parser.add_argument("--event-log", type=Path)
    return parser.parse_args()


async def main() -> None:
    args = parse_args()
    args.ready_file.parent.mkdir(parents=True, exist_ok=True)
    if args.event_log is not None:
        args.event_log.parent.mkdir(parents=True, exist_ok=True)
        args.event_log.write_text("", encoding="utf-8")
    relay = LocalRelay(args.event_log)
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for stop_signal in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(stop_signal, stop.set)
    server = await asyncio.start_server(relay.handle, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    args.ready_file.write_text(f"ws://127.0.0.1:{port}\n", encoding="utf-8")
    try:
        async with server:
            await stop.wait()
    finally:
        args.ready_file.unlink(missing_ok=True)


if __name__ == "__main__":
    asyncio.run(main())
