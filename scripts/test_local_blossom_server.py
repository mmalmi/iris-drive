"""Exercise the real storage fixture with unavailable reverse DNS."""
import hashlib
import importlib.util
from pathlib import Path
import socket
from tempfile import TemporaryDirectory
from threading import Thread
import unittest
from unittest.mock import patch
from urllib.request import Request, urlopen

spec = importlib.util.spec_from_file_location('blossom_fixture', Path(__file__).with_name('local-blossom-server.py'))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class BlossomStartupTests(unittest.TestCase):
    def test_numeric_bind_does_not_require_reverse_dns(self):
        with TemporaryDirectory() as temporary, patch.object(
            socket, 'getfqdn', side_effect=AssertionError('reverse DNS unavailable')
        ):
            with fixture.BlossomFixtureServer(('127.0.0.1', 0), Path(temporary)) as server:
                worker = Thread(target=server.serve_forever, daemon=True)
                worker.start()
                try:
                    base = f'http://127.0.0.1:{server.server_port}'
                    with urlopen(base + '/health', timeout=2) as response:
                        self.assertEqual(response.read(), b'ok\n')
                    content = b'fixture works without reverse DNS'
                    digest = hashlib.sha256(content).hexdigest()
                    request = Request(base + '/upload', content, {'x-sha-256': digest}, method='PUT')
                    with urlopen(request, timeout=2) as response:
                        self.assertEqual(response.status, 201)
                    with urlopen(base + '/' + digest + '.bin', timeout=2) as response:
                        self.assertEqual(response.read(), content)
                    self.assertEqual((Path(temporary) / (digest + '.bin')).read_bytes(), content)
                finally:
                    server.shutdown()
                    worker.join(timeout=2)


if __name__ == '__main__':
    unittest.main()
