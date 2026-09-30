import json
import os
from pathlib import Path
import shutil
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from g2d_ground_truth import private_session, write_private
from g2d_real_status import REQUIRED_CASES, status


def message(text="synthetic"):
    return {"text": text, "direction": "THEM", "sender_labeled": True}


class G2DRealStatusTests(unittest.TestCase):
    def setUp(self):
        self.session = f"st-{os.getpid()}-{abs(hash(self._testMethodName)) % 1_000_000}"
        self.directory = private_session(self.session)

    def tearDown(self):
        shutil.rmtree(self.directory, ignore_errors=True)

    def write_gt(self, tags, fill_expected=True):
        cases = []
        for index, tag in enumerate(tags):
            observed = [message(f"observed-{index}-{n}") for n in range(4)]
            expected = [message(f"observed-{index}-{n}") for n in range(4)] if fill_expected else []
            cases.append({"id": f"case-{index}", "tags": [tag], "expected": expected, "observed": observed})
        write_private(
            self.directory / "groundtruth.json",
            {
                "schema_version": "foxbot.g2c-ground-truth.v1",
                "strategy": "WECHAT_HEURISTIC_V0",
                "revision": "unit-v1",
                "cases": cases,
            },
        )

    def test_empty_session_guides_first_required_capture_without_raw_text(self):
        result = status(self.session)
        self.assertEqual(result["stage"], "CAPTURE_CASES")
        self.assertEqual(result["next_case"], "private-basic")
        encoded = json.dumps(result)
        self.assertNotIn("message_text", encoded)
        self.assertNotIn("observed", encoded)

    def test_all_cases_without_expected_requires_operator_labeling(self):
        self.write_gt([tag for _, tag in REQUIRED_CASES], fill_expected=False)
        result = status(self.session)
        self.assertEqual(result["stage"], "LABEL_EXPECTED")
        self.assertTrue(result["operator_action_required"])

    def test_accepted_session_without_baseline_guides_baseline(self):
        self.write_gt([tag for _, tag in REQUIRED_CASES], fill_expected=True)
        write_private(
            self.directory / "acceptance.json",
            {
                "schema_version": "foxbot.g2c-ground-truth-result.v1",
                "strategy": "WECHAT_HEURISTIC_V0",
                "revision": "unit-v1",
                "accepted": True,
                "cases": 6,
                "labeled_messages": 24,
                "covered_tags": sorted(tag for _, tag in REQUIRED_CASES),
                "direction_errors": 0,
                "sender_errors": 0,
                "message_count_errors": 0,
                "text_errors": 0,
            },
        )
        result = status(self.session)
        self.assertEqual(result["stage"], "READY_BASELINE")
        self.assertIn("g2d-real-baseline", result["next_command"])

    def test_baseline_guides_exactly_one_external_message(self):
        self.write_gt([tag for _, tag in REQUIRED_CASES], fill_expected=True)
        acceptance = {
            "schema_version": "foxbot.g2c-ground-truth-result.v1",
            "strategy": "WECHAT_HEURISTIC_V0",
            "revision": "unit-v1",
            "accepted": True,
            "cases": 6,
            "labeled_messages": 24,
            "covered_tags": sorted(tag for _, tag in REQUIRED_CASES),
            "direction_errors": 0,
            "sender_errors": 0,
            "message_count_errors": 0,
            "text_errors": 0,
        }
        write_private(self.directory / "acceptance.json", acceptance)
        write_private(self.directory / "bridge-config.json", {"placeholder": True})
        write_private(self.directory / "baseline-snapshot.json", {"placeholder": True})
        result = status(self.session)
        self.assertEqual(result["stage"], "WAIT_EXTERNAL_MESSAGE")
        self.assertIn("exactly one", result["operator_instruction"])
        self.assertIn("g2d-real-verify", result["next_command"])


if __name__ == "__main__":
    unittest.main()
