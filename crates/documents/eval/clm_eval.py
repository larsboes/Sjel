#!/usr/bin/env python3
"""Evaluate the local loopback CLM adapter on a frozen synthetic corpus."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from collections import Counter, defaultdict
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import ProxyHandler, Request, build_opener

MODEL_ID = "mlx-community/CLM-v0.1-8B-MLX-4bit"
MODEL_REVISION = "489b421bafc007e3ae409528d77006d26d362689"
BASE_URL = os.environ.get("SJEL_SYSTEMONE_URL", "http://127.0.0.1:8700/v1/")
MODEL_NAME = os.environ.get("SJEL_SYSTEMONE_MODEL", "clm-latest")
parsed_base = urlparse(BASE_URL)
if parsed_base.scheme != "http" or parsed_base.hostname not in {"127.0.0.1", "localhost"}:
    raise SystemExit("Evaluation only permits an HTTP loopback adapter URL")
OPENER = build_opener(ProxyHandler({}))
CATEGORY_LABELS = {"Finance", "Events", "Reference"}
CATEGORY_DESCRIPTIONS = {
    "Finance": "An invoice, bill, receipt, payment, or financial statement.",
    "Reference": "A manual, guide, API reference, or general technical documentation.",
}
EVENT_PROMPTS = {
    "baseline": "An invitation, conference program, event ticket, or event registration.",
    "scheduled-activity": "A document about a scheduled activity people attend or organize, such as an invitation, agenda, program, ticket, meeting notice, or registration confirmation.",
    "gathering-material": "Material for a gathering or public activity, such as a meeting, concert, workshop, talk, exhibition, or sports fixture; includes invitations, schedules, tickets, and registration details.",
}
INSTRUCTIONS = "Choose the best filing category for this document."


def classify(text: str, categories: dict[str, str]) -> dict:
    payload = json.dumps({
        "model": MODEL_NAME,
        "state": text,
        "questions": {
            "category": {
                "type": "choice",
                "instructions": INSTRUCTIONS,
                "criteria": categories,
            }
        },
    }).encode()
    headers = {"Content-Type": "application/json"}
    if token := os.environ.get("CLM_API_KEY"):
        headers["Authorization"] = f"Bearer {token}"
    request = Request(BASE_URL.rstrip("/") + "/systemone", data=payload, headers=headers)
    with OPENER.open(request, timeout=30) as response:
        result = json.load(response)
    if not isinstance(result, dict) or result.get("model") != MODEL_NAME or result.get("revision") != MODEL_REVISION:
        raise RuntimeError("adapter returned an unexpected model identity or revision")
    answers = result.get("answers")
    answer = answers.get("category") if isinstance(answers, dict) else None
    if not isinstance(answer, dict):
        raise RuntimeError("adapter omitted the category answer")
    probabilities = answer.get("probabilities")
    confidence = answer.get("confidence")
    if (
        answer.get("type") != "choice"
        or answer.get("choice") not in categories
        or not isinstance(probabilities, dict)
        or set(probabilities) != set(categories)
        or not isinstance(confidence, (int, float))
        or isinstance(confidence, bool)
        or not math.isfinite(confidence)
        or not 0 <= confidence <= 1
        or any(
            not isinstance(value, (int, float)) or isinstance(value, bool)
            or not math.isfinite(value) or not 0 <= value <= 1
            for value in probabilities.values()
        )
        or abs(sum(probabilities.values()) - 1) > 0.01
    ):
        raise RuntimeError("adapter returned an invalid category answer")
    return answer


def summarize(rows: list[dict], key: str) -> dict:
    groups: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        groups[row[key]].append(row)
    return {
        value: {
            "samples": len(group),
            "correct": sum(row["correct"] for row in group),
            "accuracy": round(sum(row["correct"] for row in group) / len(group), 4),
        }
        for value, group in sorted(groups.items())
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", default="clm-corpus-v2.json", help="synthetic JSON corpus in this directory")
    parser.add_argument("--event-prompt", choices=EVENT_PROMPTS, default="baseline")
    args = parser.parse_args()
    eval_dir = Path(__file__).resolve().parent
    corpus_path = (eval_dir / args.corpus).resolve()
    if corpus_path.parent != eval_dir:
        raise SystemExit("Corpus must be a file directly inside the evaluation directory")
    raw_corpus = corpus_path.read_bytes()
    samples = json.loads(raw_corpus)
    if not isinstance(samples, list) or not samples:
        raise SystemExit("Corpus must contain at least one sample")

    if corpus_path.name in {
        "clm-corpus-v2.json",
        "clm-corpus-holdout-v1.json",
        "clm-corpus-hybrid-v1.json",
        "clm-corpus-combined-holdout-v1.json",
    }:
        counts = Counter((sample.get("language"), sample.get("expected")) for sample in samples if isinstance(sample, dict))
        difficulty_counts = Counter(
            (sample.get("language"), sample.get("expected"), sample.get("difficulty"))
            for sample in samples if isinstance(sample, dict)
        )
        if any(counts[(language, category)] != 8 for language in ("en", "de") for category in ("Finance", "Events", "Reference")):
            raise SystemExit("balanced corpus must have eight samples per language and category")
        if any(
            difficulty_counts[(language, category, difficulty)] != expected
            for language in ("en", "de")
            for category in ("Finance", "Events", "Reference")
            for difficulty, expected in (("clear", 6), ("overlap", 2))
        ):
            raise SystemExit("balanced corpus must have six clear and two overlapping samples per cell")

    ids: set[str] = set()
    categories = {
        **CATEGORY_DESCRIPTIONS,
        "Events": EVENT_PROMPTS[args.event_prompt],
    }
    results = []
    confusion: Counter[tuple[str, str]] = Counter()
    for sample in samples:
        if (
            not isinstance(sample, dict)
            or sample.get("language") not in {"en", "de"}
            or sample.get("expected") not in CATEGORY_LABELS
            or sample.get("difficulty", "unspecified") not in {"clear", "overlap", "unspecified"}
            or not isinstance(sample.get("id"), str)
            or not isinstance(sample.get("text"), str)
            or not sample["text"].strip()
            or sample["id"] in ids
        ):
            raise SystemExit("Corpus contains an invalid or duplicate synthetic sample")
        ids.add(sample["id"])
        answer = classify(sample["text"], categories)
        predicted = answer["choice"]
        correct = predicted == sample["expected"]
        confusion[(sample["expected"], predicted)] += 1
        results.append({
            "id": sample["id"],
            "language": sample["language"],
            "expected": sample["expected"],
            "difficulty": sample.get("difficulty", "unspecified"),
            "predicted": predicted,
            "correct": correct,
            "confidence_margin": round(answer["confidence"], 4),
            "probabilities": {key: round(value, 4) for key, value in answer["probabilities"].items()},
        })

    correct = sum(row["correct"] for row in results)
    correct_margins = [row["confidence_margin"] for row in results if row["correct"]]
    error_margins = [row["confidence_margin"] for row in results if not row["correct"]]
    report = {
        "model": MODEL_ID,
        "revision": MODEL_REVISION,
        "adapter": "loopback System One (/v1/systemone)",
        "corpus": corpus_path.name,
        "corpus_sha256": hashlib.sha256(raw_corpus).hexdigest(),
        "event_prompt": args.event_prompt,
        "category_descriptions": {**CATEGORY_DESCRIPTIONS, "Events": EVENT_PROMPTS[args.event_prompt]},
        "samples": len(results),
        "correct": correct,
        "accuracy": round(correct / len(results), 4),
        "by_language": summarize(results, "language"),
        "by_category": summarize(results, "expected"),
        "by_difficulty": summarize(results, "difficulty"),
        "confidence_margin_summary": {
            "correct_mean": round(sum(correct_margins) / len(correct_margins), 4) if correct_margins else None,
            "incorrect_mean": round(sum(error_margins) / len(error_margins), 4) if error_margins else None,
            "wrong_margin_ge_05": sum(value >= 0.5 for value in error_margins),
            "note": "CLM margin is not calibrated; 0.5 is a descriptive bucket, not a filing threshold.",
        },
        "confusion": {
            f"{expected} -> {predicted}": count
            for (expected, predicted), count in sorted(confusion.items())
        },
        "misclassified": [row for row in results if not row["correct"]],
        "results": results,
        "warning": "Synthetic results only; this corpus is not representative of personal documents and does not authorize automatic filing.",
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
