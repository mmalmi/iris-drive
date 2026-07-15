#!/usr/bin/env python3
"""Tiny local Nostr relay for offline smoke tests."""

import argparse
import asyncio
import json
from pathlib import Path

import websockets


def event_matches_filter(event, relay_filter):
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


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ready-file", required=True)
    args = parser.parse_args()
    events = []

    async def handle(websocket):
        async for message in websocket:
            try:
                payload = json.loads(message)
            except json.JSONDecodeError:
                continue
            if not isinstance(payload, list) or not payload:
                continue
            command = payload[0]
            if command == "EVENT" and len(payload) >= 2 and isinstance(payload[1], dict):
                event = payload[1]
                events.append(event)
                await websocket.send(json.dumps(["OK", event.get("id", ""), True, ""]))
            elif command == "REQ" and len(payload) >= 2:
                subscription_id = payload[1]
                filters = [item for item in payload[2:] if isinstance(item, dict)]
                for event in events:
                    if any(event_matches_filter(event, relay_filter) for relay_filter in filters):
                        await websocket.send(json.dumps(["EVENT", subscription_id, event]))
                await websocket.send(json.dumps(["EOSE", subscription_id]))
            elif command == "CLOSE":
                continue

    async with websockets.serve(handle, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        Path(args.ready_file).write_text(f"ws://127.0.0.1:{port}\n", encoding="utf-8")
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main())
