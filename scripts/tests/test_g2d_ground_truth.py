import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from g2d_ground_truth import token


class G2DGroundTruthTests(unittest.TestCase):
    def test_session_token_rejects_path_escape(self):
        self.assertTrue(token("g2d-test_01"))
        self.assertFalse(token("../escape"))
        self.assertFalse(token(""))

    def test_public_result_contract_contains_no_raw_fields(self):
        result = {
            "schema_version": "foxbot.g2c-ground-truth-result.v1",
            "accepted": False,
            "raw_text_included": False,
        }
        encoded = json.dumps(result)
        self.assertNotIn("expected", encoded)
        self.assertNotIn("observed", encoded)
        self.assertNotIn("message_text", encoded)


if __name__ == "__main__":
    unittest.main()
