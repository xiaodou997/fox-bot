import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from g2c_ground_truth import evaluate


def message(text, direction="THEM", sender=True):
    return {"text": text, "direction": direction, "sender_labeled": sender}


def document():
    cases = []
    tags = ["private", "group", "duplicate_text", "numeric", "multiline", "reference"]
    for index, tag in enumerate(tags):
        rows = [
            message(f"case-{index}-a"),
            message("好的"),
            message("好的"),
            message("订单 123.45", "ME", False),
        ]
        cases.append({"id": f"case-{index}", "tags": [tag], "expected": rows, "observed": [dict(v) for v in rows]})
    return {
        "schema_version": "foxbot.g2c-ground-truth.v1",
        "strategy": "WECHAT_HEURISTIC_V0",
        "revision": "synthetic-v1",
        "cases": cases,
    }


class GroundTruthTests(unittest.TestCase):
    def test_acceptance_result_matches_rust_bridge_contract(self):
        result = evaluate(document())
        self.assertTrue(result["accepted"])
        self.assertEqual(result["cases"], 6)
        self.assertEqual(result["labeled_messages"], 24)
        self.assertEqual(result["direction_errors"], 0)
        self.assertEqual(result["message_count_errors"], 0)

    def test_direction_or_sender_error_blocks_acceptance(self):
        source = document()
        source["cases"][0]["observed"][0]["direction"] = "ME"
        result = evaluate(source)
        self.assertFalse(result["accepted"])
        self.assertEqual(result["direction_errors"], 1)
        source = document()
        source["cases"][1]["observed"][0]["sender_labeled"] = False
        result = evaluate(source)
        self.assertFalse(result["accepted"])
        self.assertEqual(result["sender_errors"], 1)

    def test_missing_required_coverage_or_count_mismatch_blocks_acceptance(self):
        source = document()
        source["cases"][0]["tags"] = ["group"]
        self.assertFalse(evaluate(source)["accepted"])
        source = document()
        source["cases"][0]["observed"].pop()
        result = evaluate(source)
        self.assertFalse(result["accepted"])
        self.assertEqual(result["message_count_errors"], 1)

    def test_result_never_contains_raw_message_text(self):
        result = evaluate(document())
        encoded = json.dumps(result, ensure_ascii=False)
        self.assertNotIn("订单", encoded)
        self.assertNotIn("好的", encoded)
        self.assertNotIn("case-0-a", encoded)

    def test_text_tolerance_is_bounded_to_two_percent(self):
        source = document()
        source["cases"][0]["observed"][0]["text"] = "wrong"
        self.assertFalse(evaluate(source)["accepted"])


if __name__ == "__main__":
    unittest.main()
