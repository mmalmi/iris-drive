import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("ios_entitlements", pathlib.Path(__file__).with_name("ios-distribution-entitlements.py"))
entitlements = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entitlements)


class DistributionEntitlementsTest(unittest.TestCase):
    def test_rejects_a_signed_export_that_dropped_required_capabilities(self):
        required = {"com.apple.security.application-groups": ["group.example"],
                    "com.apple.developer.associated-domains": ["applinks:example.org"]}
        signature = {"application-identifier": "TEAM.org.example", "get-task-allow": False}
        valid = signature | required
        entitlements.check_entitlements(valid, required, "org.example")
        for key in required:
            with self.subTest(key=key), self.assertRaises(ValueError):
                entitlements.check_entitlements({k: v for k, v in valid.items() if k != key}, required, "org.example")
        with self.assertRaises(ValueError):
            entitlements.check_entitlements(valid | {"get-task-allow": True}, required, "org.example")


if __name__ == "__main__":
    unittest.main()
