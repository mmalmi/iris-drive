import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from importlib.util import module_from_spec, spec_from_file_location

spec = spec_from_file_location('unpack', Path(__file__).with_name('unpack-hosted-product.py'))
module = module_from_spec(spec)
spec.loader.exec_module(module)


class UnpackTests(unittest.TestCase):
    def test_extracts_bytes_and_execute_mode(self):
        self.fixture('Iris Drive.app/Contents/MacOS/idrive', valid=True)

    def test_rejects_escape_and_links_before_writing_any_file(self):
        for name, kind in [('../escape', tarfile.REGTYPE), ('/escape', tarfile.REGTYPE),
                           ('link', tarfile.SYMTYPE), ('link', tarfile.LNKTYPE)]:
            with self.subTest(name=name, kind=kind):
                self.fixture(name, kind=kind)

    def fixture(self, name, kind=tarfile.REGTYPE, valid=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / 'products.tar'
            destination = root / 'output'
            destination.mkdir()
            with tarfile.open(archive, 'w') as stream:
                item = tarfile.TarInfo(name)
                item.type, item.mode = kind, 0o755
                item.linkname = '../escape'
                item.size = 3 if kind == tarfile.REGTYPE else 0
                stream.addfile(item, io.BytesIO(b'abc'))
            if valid:
                module.unpack(archive, destination)
                self.assertEqual((destination / name).read_bytes(), b'abc')
                self.assertTrue((destination / name).stat().st_mode & 0o111)
            else:
                with self.assertRaises(ValueError):
                    module.unpack(archive, destination)
                self.assertEqual(list(destination.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
