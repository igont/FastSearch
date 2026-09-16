import copy
import hashlib
import json
import unittest
from pathlib import Path

from calibrate_relevance import calibrate

ROOT = Path(__file__).resolve().parents[1] / "tests/fixtures/relevance"


class CalibrationTests(unittest.TestCase):
    def setUp(self):
        self.raw = (ROOT / "cases.json").read_bytes()
        self.cases = json.loads(self.raw)
        self.scores = json.loads((ROOT / "scores.json").read_bytes())

    def test_validation_labels_never_select_the_threshold(self):
        baseline = calibrate(self.cases, self.scores, self.raw)
        modified = copy.deepcopy(self.cases)
        for case in modified["pairs"]:
            if case["split"] == "validation":
                case["relevant"] = not case["relevant"]
        raw = json.dumps(modified).encode()
        self.scores["cases_sha256"] = hashlib.sha256(raw).hexdigest()
        failed = calibrate(modified, self.scores, raw)
        self.assertEqual(baseline["threshold"], failed["threshold"])
        self.assertFalse(failed["accepted"])

    def test_missing_scores_and_wrong_corpus_are_rejected(self):
        self.scores["scores"].pop()
        with self.assertRaises(ValueError):
            calibrate(self.cases, self.scores, self.raw)
        with self.assertRaises(ValueError):
            calibrate(self.cases, self.scores, self.raw + b" ")

    def test_training_and_validation_must_not_share_text(self):
        modified = copy.deepcopy(self.cases)
        next(p for p in modified["pairs"] if p["split"] == "validation")["document"] = modified["pairs"][0]["document"]
        raw = json.dumps(modified).encode()
        self.scores["cases_sha256"] = hashlib.sha256(raw).hexdigest()
        with self.assertRaisesRegex(ValueError, "leakage"):
            calibrate(modified, self.scores, raw)


if __name__ == "__main__":
    unittest.main()
