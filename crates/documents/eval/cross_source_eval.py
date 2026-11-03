#!/usr/bin/env python3
"""Compare separate synthetic rule, CLM, and Apple FM outputs without merging them."""
from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


def load_json(path: Path) -> dict:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise SystemExit(f"Expected a JSON object in {path}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", default="clm-corpus-combined-holdout-v1.json")
    parser.add_argument("--rules", default="rule-baseline-v2-holdout-v1-2026-10-22.json")
    parser.add_argument("--clm", default="clm-eval-combined-holdout-v1-2026-10-22.json")
    parser.add_argument("--apple", default="apple-fm-eval-combined-holdout-v1-2026-10-22.json")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    paths = {key: (root / value).resolve() for key, value in vars(args).items()}
    if any(path.parent != root for path in paths.values()):
        raise SystemExit("All evaluation inputs must be files directly inside the evaluation directory")
    corpus_bytes = paths["corpus"].read_bytes()
    corpus = json.loads(corpus_bytes)
    if not isinstance(corpus, list) or not corpus:
        raise SystemExit("Corpus must contain at least one sample")
    corpus_hash = hashlib.sha256(corpus_bytes).hexdigest()
    corpus_name = paths["corpus"].name
    rules_report = load_json(paths["rules"])
    rules = rules_report.get("datasets", {}).get(corpus_name)
    if not isinstance(rules, dict):
        raise SystemExit(f"Rules report has no dataset entry for {corpus_name}")
    clm = load_json(paths["clm"])
    apple = load_json(paths["apple"])
    for name, report in (("rules", rules), ("CLM", clm), ("Apple FM", apple)):
        if report.get("corpus_sha256") != corpus_hash:
            raise SystemExit(f"{name} corpus hash does not match the selected corpus")
        if report.get("corpus") != corpus_name:
            raise SystemExit(f"{name} corpus name does not match the selected corpus")

    source_results = {
        "rules": {row["id"]: row for row in rules.get("predictions", [])},
        "clm": {row["id"]: row for row in clm.get("results", [])},
        "apple_foundation_models": {row["id"]: row for row in apple.get("results", [])},
    }
    expected_ids = {sample.get("id") for sample in corpus}
    if any(set(rows) != expected_ids for rows in source_results.values()):
        raise SystemExit("Source result ids do not exactly match the corpus")

    source_summary = {}
    for name, rows in source_results.items():
        predictions = [row for row in rows.values() if row.get("predicted") is not None]
        valid = [row for row in predictions if row.get("proposal_valid", False)]
        source_summary[name] = {
            "category_predictions": len(predictions),
            "category_correct": sum(bool(row.get("correct")) for row in predictions),
            "category_accuracy_over_corpus": round(
                sum(bool(row.get("correct")) for row in predictions) / len(corpus), 4
            ),
            "evidence_valid_proposals": len(valid),
            "evidence_valid_proposals_correct": sum(bool(row.get("correct")) for row in valid),
            "evidence_note": (
                "No source span is returned by the existing deterministic suggestion."
                if name == "rules" else
                "CLM returns category scores only; no source evidence span."
                if name == "clm" else
                "Exact source substring within the operator-selected 20-word/160-character limit."
            ),
        }

    pairs = {}
    names = list(source_results)
    for left_index, left in enumerate(names):
        for right in names[left_index + 1:]:
            both = [
                sample for sample in corpus
                if source_results[left][sample["id"]].get("predicted") is not None
                and source_results[right][sample["id"]].get("predicted") is not None
            ]
            agreed = [
                sample for sample in both
                if source_results[left][sample["id"]]["predicted"]
                == source_results[right][sample["id"]]["predicted"]
            ]
            pairs[f"{left}__{right}"] = {
                "both_predicted": len(both),
                "agree": len(agreed),
                "conflict": len(both) - len(agreed),
                "agreement_ids": [sample["id"] for sample in agreed],
                "conflict_ids": [sample["id"] for sample in both if sample not in agreed],
            }

    rows = []
    conflicts = Counter()
    any_expected = 0
    any_evidence_expected = 0
    for sample in corpus:
        predictions = {}
        non_null = []
        for name, source in source_results.items():
            item = source[sample["id"]]
            predicted = item.get("predicted")
            usable = bool(item.get("proposal_valid", False))
            predictions[name] = {
                "category": predicted,
                "correct": bool(item.get("correct")) if predicted is not None else None,
                "proposal_valid": usable,
                "confidence": item.get("confidence_margin", item.get("confidence")),
                "failure": item.get("failure"),
            }
            if predicted is not None:
                non_null.append(predicted)
        if any(predictions[name]["category"] == sample["expected"] for name in predictions):
            any_expected += 1
        if any(
            predictions[name]["proposal_valid"]
            and predictions[name]["category"] == sample["expected"]
            for name in predictions
        ):
            any_evidence_expected += 1
        distinct = set(non_null)
        if len(distinct) > 1:
            conflicts["samples_with_category_conflict"] += 1
        rows.append({
            "id": sample["id"],
            "expected": sample["expected"],
            "predictions": predictions,
            "category_conflict": len(distinct) > 1,
        })

    report = {
        "schema": "sjel.documents.cross-source-evaluation/v1",
        "purpose": "Compare separate source outputs for operator review; this report computes no merged category.",
        "corpus": corpus_name,
        "corpus_sha256": corpus_hash,
        "samples": len(corpus),
        "sources": source_summary,
        "pairwise": pairs,
        "any_correct_category_prediction": any_expected,
        "any_correct_evidence_proposal": any_evidence_expected,
        "samples_with_category_conflict": conflicts["samples_with_category_conflict"],
        "safety_blocks": apple.get("safety_blocks", 0),
        "preblocked_ids": apple.get("preblocked_ids", []),
        "results": rows,
        "policy": "The sources remain separate. Missing, malformed, safety-blocked, or evidence-invalid outputs are not turned into proposals; human review remains the filing decision.",
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
