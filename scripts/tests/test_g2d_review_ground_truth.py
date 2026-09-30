import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from g2d_review_ground_truth import review_case, review_document


def msg(text, direction="THEM", sender=False):
    return {"text": text, "direction": direction, "sender_labeled": sender}


class G2DReviewGroundTruthTests(unittest.TestCase):
    def test_accept_edit_delete_and_add(self):
        case = {
            "id": "numeric-money",
            "tags": ["numeric"],
            "expected": [],
            "observed": [
                msg("A"),
                msg("B", "ME"),
                msg("false-positive"),
            ],
        }
        # A accept; B edit text+direction+sender; false positive delete;
        # add one missing message; then stop adding.
        inp = io.StringIO("\ne\nB-fixed\nTHEM\ny\nd\ny\nMISSING\nME\nn\nn\n")
        out = io.StringIO()
        expected = review_case(case, inp, out)
        self.assertEqual(expected[0], msg("A"))
        self.assertEqual(expected[1], msg("B-fixed", "THEM", True))
        self.assertEqual(expected[2], msg("MISSING", "ME", False))
        self.assertNotIn("false-positive", [m["text"] for m in expected])

    def test_existing_expected_is_skipped_unless_redo(self):
        doc = {
            "schema_version": "foxbot.g2c-ground-truth.v1",
            "strategy": "WECHAT_HEURISTIC_V0",
            "revision": "test",
            "cases": [
                {
                    "id": "private-basic",
                    "tags": ["private"],
                    "expected": [msg("verified")],
                    "observed": [msg("observed")],
                }
            ],
        }
        updated, reviewed = review_document(doc, io.StringIO(""), io.StringIO(), redo=False)
        self.assertEqual(reviewed, 0)
        self.assertEqual(updated["cases"][0]["expected"][0]["text"], "verified")

    def test_review_output_is_operator_tty_content_not_redacted_contract(self):
        case = {
            "id": "private-basic",
            "tags": ["private"],
            "expected": [],
            "observed": [msg("LOCAL-PRIVATE-TEXT")],
        }
        inp = io.StringIO("\nn\n")
        out = io.StringIO()
        review_case(case, inp, out)
        self.assertIn("LOCAL-PRIVATE-TEXT", out.getvalue())


if __name__ == "__main__":
    unittest.main()
