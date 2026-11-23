#!/usr/bin/env python3
"""Run the physical linking gate against two disposable loopback services over USB."""
import argparse
import asyncio
from contextlib import ExitStack
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlsplit


SCRIPTS = Path(__file__).resolve().parent


async def serve_fixture(stack, tasks, directory, script, *options):
    ready_file = directory / (script + ".ready")
    log = stack.enter_context((directory / (script + ".log")).open("wb"))
    process = await asyncio.create_subprocess_exec(
        sys.executable, SCRIPTS / script, "--ready-file", ready_file, *options,
        stdout=log, stderr=log,
    )
    tasks.append(process)
    for _ in range(120):
        if ready_file.exists():
            endpoint = urlsplit(ready_file.read_text().strip())
            if endpoint.hostname == "127.0.0.1" and endpoint.port:
                return endpoint
            break
        if process.returncode is not None:
            break
        await asyncio.sleep(0.05)
    raise RuntimeError(f"{script} did not publish a loopback endpoint")


def wired_identifier(identifier):
    result = subprocess.check_output([
        "xcrun", "devicectl", "list", "devices", "--json-output", "-", "--timeout", "15", "--quiet",
    ])
    devices = json.loads(result)["result"]["devices"]
    phone, = [device for device in devices if device["identifier"] == identifier]
    if phone["connectionProperties"]["transportType"] != "wired":
        raise RuntimeError("The selected iPhone needs a USB connection for this fixture")
    return phone["hardwareProperties"]["udid"]


async def copy_stream(source, destination):
    while True:
        chunk = await source.read(32768)
        if not chunk:
            if destination.can_write_eof():
                destination.write_eof()
            return
        destination.write(chunk)
        await destination.drain()


async def serve_connections(control_port, service_port, token, counts, label):
    # The port mapping is fixed at startup. Phone traffic cannot select another service.
    while True:
        writers = []
        pumps = []
        try:
            control, to_phone = await asyncio.open_connection("127.0.0.1", control_port)
            writers.append(to_phone)
            to_phone.write(token)
            await to_phone.drain()
            if await control.readexactly(1) != b"\x01":
                raise ConnectionError("Unexpected fixture handshake")
            service, to_service = await asyncio.open_connection("127.0.0.1", service_port)
            writers.append(to_service)
            counts[label] += 1
            pumps = [asyncio.create_task(copy_stream(control, to_service)),
                     asyncio.create_task(copy_stream(service, to_phone))]
            await asyncio.gather(*pumps)
        except (OSError, asyncio.IncompleteReadError):
            # Each XCTest case starts a fresh listener. Wait between cases.
            pass
        finally:
            for task in pumps:
                task.cancel()
            for writer in writers:
                writer.close()
            await asyncio.gather(*pumps, return_exceptions=True)
        await asyncio.sleep(0.35)


async def stop(process):
    if process.returncode is None:
        process.terminate()
        try:
            await asyncio.wait_for(process.wait(), 4)
        except asyncio.TimeoutError:
            process.kill()
            await process.wait()


async def run(args):
    if os.environ.get("IRIS_DRIVE_NATIVE_MOBILE_RESERVED") != "1":
        raise RuntimeError("The physical linking gate must first reserve both devices")
    hardware = wired_identifier(args.ios)
    android_reverse = [args.adb, "-s", args.android, "reverse"]
    processes, connections, mappings = [], [], []
    with ExitStack() as stack:
        directory = Path(stack.enter_context(TemporaryDirectory(prefix="iris-drive-link-")))
        try:
            events = directory / "events"
            relay = await serve_fixture(stack, processes, directory, "local-nostr-relay.py", "--event-log", events)
            store = await serve_fixture(stack, processes, directory, "local-blossom-server.py",
                                        "--storage-dir", directory / "blocks")
            service_ports = [relay.port, store.port]
            with ExitStack() as allocation:
                control_ports = []
                for _ in service_ports:
                    sock = allocation.enter_context(socket.socket())
                    sock.bind(("127.0.0.1", 0))
                    control_ports.append(sock.getsockname()[1])
            proxy_log = stack.enter_context((directory / "usbmux.log").open("wb"))
            proxy = await asyncio.create_subprocess_exec(
                "iproxy", "--local", "--source", "127.0.0.1", "--udid", hardware,
                *(f"{port}:{port}" for port in control_ports), stdout=proxy_log, stderr=proxy_log,
            )
            processes.append(proxy)
            token = secrets.token_hex(24)
            counts = {"relay": 0, "blossom": 0}
            for label, control, service in zip(counts, control_ports, service_ports):
                mapping = f"tcp:{service}"
                subprocess.run([*android_reverse, "--no-rebind", mapping, mapping], check=True)
                mappings.append(mapping)
                connections.extend(asyncio.create_task(serve_connections(control, service, token.encode(), counts, label))
                                   for _ in range(8))
            environment = {**os.environ, "IRIS_DRIVE_USB_FIXTURE_PORTS": ",".join(map(str, service_ports + control_ports)),
                           "IRIS_DRIVE_USB_FIXTURE_TOKEN": token}
            child = await asyncio.create_subprocess_exec(*args.command, env=environment)
            processes.append(child)
            status = await child.wait()
            with events.open() as stream:
                event_count = sum(1 for _ in stream)
            print("Physical USB fixture " + json.dumps({
                "connections": counts, "events": event_count,
                "storedObjects": sum(1 for _ in (directory / "blocks").iterdir()),
                "exit": status,
            }), flush=True)
            return status
        finally:
            for task in connections:
                task.cancel()
            await asyncio.gather(*connections, return_exceptions=True)
            for mapping in mappings:
                subprocess.run([*android_reverse, "--remove", mapping], check=False)
            for process in reversed(processes):
                await stop(process)


async def supervise(args):
    task = asyncio.create_task(run(args))
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, task.cancel)
    return await task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", required=True)
    parser.add_argument("--android", required=True)
    parser.add_argument("--ios", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if not options.command:
        parser.error("Missing linking gate command")
    sys.exit(asyncio.run(supervise(options)))
