"""Real relay shutdown must complete while clients keep sockets open."""
from pathlib import Path
import socket
import subprocess
import sys
from tempfile import TemporaryDirectory
import time
import unittest
from urllib.parse import urlsplit


class RelayShutdownTests(unittest.TestCase):
    def test_shutdown_with_open_websocket(self):
        self.check_shutdown(partial_handshake=False)

    def test_shutdown_with_incomplete_handshake(self):
        self.check_shutdown(partial_handshake=True)

    def check_shutdown(self, partial_handshake):
        with TemporaryDirectory() as temporary:
            ready = Path(temporary) / 'ready'
            process = subprocess.Popen([
                sys.executable, str(Path(__file__).with_name('local-nostr-relay.py')),
                '--ready-file', str(ready),
            ], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            clients = []
            try:
                deadline = time.monotonic() + 5
                while not ready.exists():
                    self.assertIsNone(process.poll(), 'relay exited before readiness')
                    self.assertLess(time.monotonic(), deadline, 'relay readiness timed out')
                    time.sleep(.01)
                url = urlsplit(ready.read_text().strip())
                if partial_handshake:
                    client = socket.create_connection((url.hostname, url.port), timeout=2)
                    clients.append(client)
                    client.sendall(b'GET / HTTP/1.1\r\n')
                # An answered upgrade proves the event loop has accepted work.
                client = socket.create_connection((url.hostname, url.port), timeout=2)
                clients.append(client)
                client.sendall(
                    b'GET / HTTP/1.1\r\nHost: localhost\r\n'
                    b'Upgrade: websocket\r\nConnection: Upgrade\r\n'
                    b'Sec-WebSocket-Version: 13\r\n'
                    b'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n')
                self.assertTrue(client.recv(4096).startswith(b'HTTP/1.1 101'))
                process.terminate()
                try:
                    stdout, stderr = process.communicate(timeout=2)
                except subprocess.TimeoutExpired:
                    self.fail('relay shutdown waited for clients to disconnect')
                self.assertEqual(process.returncode, 0, (stdout, stderr))
                self.assertFalse(ready.exists())
            finally:
                for client in clients:
                    client.close()
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=3)


if __name__ == '__main__':
    unittest.main()
