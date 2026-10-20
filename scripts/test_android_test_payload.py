"""The test signing step must preserve every application payload entry."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import zipfile
from verify_android_test_payload import verify_payload


class AndroidPayloadTests(unittest.TestCase):
    def check_archives(self, before, after):
        with TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ('original.apk', 'resigned.apk')]
            for path, entries in zip(paths, (before, after)):
                with zipfile.ZipFile(path, 'w') as archive:
                    for name, content in entries.items():
                        archive.writestr(name, content)
            return verify_payload(*paths)

    def test_allows_only_signature_metadata_to_change(self):
        payload = {'classes.dex': b'dex', 'lib/arm64-v8a/core.so': b'native', 'resources.arsc': b'ui'}
        digest = self.check_archives(dict(payload, **{'META-INF/CERT.RSA': b'old'}),
                                   dict(payload, **{'META-INF/CERT.RSA': b'new'}))
        self.assertEqual(len(digest), 64)

    def test_rejects_changed_native_code(self):
        with self.assertRaises(ValueError):
            self.check_archives({'lib/core.so': b'old'}, {'lib/core.so': b'new'})

    def test_rejects_added_payload(self):
        with self.assertRaises(ValueError):
            self.check_archives({'classes.dex': b'dex'}, {'classes.dex': b'dex', 'classes2.dex': b'new'})


if __name__ == '__main__':
    unittest.main()
