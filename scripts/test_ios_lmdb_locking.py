"""Exercise the import policy against retained and dead-stripped lock backends."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "ios_lmdb_locking", Path(__file__).with_name("check-ios-lmdb-locking.py")
)
locking = importlib.util.module_from_spec(spec)
spec.loader.exec_module(locking)


class LockingPolicyTest(unittest.TestCase):
    def test_system_v_is_rejected_in_every_image(self):
        for extension in (False, True):
            for symbol in locking.FORBIDDEN:
                unsupported, _ = locking.locking_errors(
                    {symbol} | locking.REQUIRED, extension=extension
                )
                self.assertEqual(unsupported, {symbol})

    def test_main_app_requires_the_supported_backend(self):
        self.assertEqual(locking.locking_errors(set(), extension=False)[1], locking.REQUIRED)
        self.assertEqual(locking.locking_errors(locking.REQUIRED, extension=False), (set(), set()))

    def test_dead_stripped_extension_has_no_lock_backend(self):
        self.assertEqual(locking.locking_errors({"_malloc"}, extension=True), (set(), set()))

    def test_partial_extension_backend_is_rejected(self):
        for symbol in locking.REQUIRED:
            _, missing = locking.locking_errors(locking.REQUIRED - {symbol}, extension=True)
            self.assertEqual(missing, {symbol})


if __name__ == "__main__":
    unittest.main()
