#!/usr/bin/env python3
"""Select a score cutoff on training labels; evaluate once on held-out sources.

This does not calibrate a probability. Failed evaluation never produces an
accepted policy. Re-score with examples/score_relevance.rs after model changes.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path


def calibrate(cases, scored, cases_bytes):
    if scored["cases_sha256"] != hashlib.sha256(cases_bytes).hexdigest():
        raise ValueError("scores belong to another corpus")
    pairs = cases["pairs"]
    by_id = {pair["id"]: pair for pair in pairs}
    scores = {row["id"]: row["score"] for row in scored["scores"]}
    if len(by_id) != len(pairs) or len(scores) != len(scored["scores"]) or by_id.keys() != scores.keys():
        raise ValueError("duplicate or missing case IDs")
    if any(not math.isfinite(score) or not 0 <= score <= 1 for score in scores.values()):
        raise ValueError("invalid model score")
    if any(type(pair["relevant"]) is not bool or pair["split"] not in {"train", "validation"} for pair in pairs):
        raise ValueError("invalid label or split")
    train = [pair for pair in pairs if pair["split"] == "train"]
    validation = [pair for pair in pairs if pair["split"] == "validation"]
    for key in ("query", "query_id", "document", "source_path"):
        if {p[key] for p in train} & {p[key] for p in validation}:
            raise ValueError(f"train/validation leakage: {key}")
    positives = [scores[p["id"]] for p in train if p["relevant"]]
    negatives = [scores[p["id"]] for p in train if not p["relevant"]]
    if not positives or not negatives or not validation:
        raise ValueError("both training classes and a validation set are required")
    if max(negatives) >= min(positives):
        raise ValueError("training labels are not separable; do not publish a cutoff")
    # A margin inside the training separation; validation cannot tune this value.
    threshold = (max(negatives) + min(positives)) / 2

    def evaluate(rows):
        counts = dict(true_positive=0, false_positive=0, true_negative=0, false_negative=0)
        queries = {}
        for pair in rows:
            admitted = scores[pair["id"]] >= threshold
            key = ("true_" if admitted == pair["relevant"] else "false_") + ("positive" if admitted else "negative")
            counts[key] += 1
            queries.setdefault(pair["query_id"], []).append((pair["relevant"], admitted))
        no_answer = [rows for rows in queries.values() if not any(label for label, _ in rows)]
        counts.update(pairs=len(rows), queries=len(queries), no_answer_queries=len(no_answer),
                      correct_abstentions=sum(not any(admit for _, admit in rows) for rows in no_answer))
        return counts

    training = evaluate(train)
    held_out = evaluate(validation)
    relevant_count = held_out["true_positive"] + held_out["false_negative"]
    accepted = (held_out["false_positive"] == 0 and relevant_count > 0
                and held_out["true_positive"] / relevant_count >= 0.75
                and held_out["no_answer_queries"] >= 2
                and held_out["correct_abstentions"] == held_out["no_answer_queries"])
    return dict(schema=1, accepted=accepted, threshold=threshold,
                selection="midpoint between highest negative and lowest positive training score",
                model_revision=scored["model_revision"], instruction=scored["instruction"],
                max_tokens=scored["max_tokens"], cases_sha256=scored["cases_sha256"],
                training=training, validation=held_out,
                limitation="Small author-labelled passage benchmark. Not a probability calibration or a guarantee for arbitrary queries; retrieval recall is tested separately.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", type=Path)
    parser.add_argument("scores", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    cases_bytes = args.cases.read_bytes()
    report = calibrate(json.loads(cases_bytes), json.loads(args.scores.read_bytes()), cases_bytes)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    if not report["accepted"]:
        raise SystemExit("held-out evaluation failed; do not publish this policy")


if __name__ == "__main__":
    main()
