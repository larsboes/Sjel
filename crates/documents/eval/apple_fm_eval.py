#!/usr/bin/env python3
"""Evaluate Apple Foundation Models on synthetic Documents samples only."""
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
from collections import Counter, defaultdict
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlparse
from urllib.request import ProxyHandler, Request, build_opener

MODEL_ID = "apple-foundationmodel"
BASE_URL = os.environ.get("SJEL_FOUNDATION_MODELS_URL", "http://127.0.0.1:8091/v1")
PROMPT_VERSION = "documents-category-v2-evidence-bounded"
CATEGORIES = ("Finance", "Events", "Reference")
CATEGORY_GUIDANCE = {
    "Finance": "invoices, bills, receipts, payments, account statements, or reimbursements",
    "Events": "invitations, schedules, programs, tickets, or registration for gatherings and activities",
    "Reference": "manuals, instructions, technical guides, API documentation, or procedures",
}
SYSTEM_PROMPT = (
    "Classify the supplied synthetic document by its primary purpose. Choose exactly one allowed category. "
    "Use the category descriptions. Return the shortest useful evidence as an exact, contiguous quote copied "
    "from the document text. The quote must have no more than 20 whitespace-separated words and no more than "
    "160 characters. Do not paraphrase or add words.\n"
    + "\n".join(f"{category}: {CATEGORY_GUIDANCE[category]}" for category in CATEGORIES)
)
JSON_SCHEMA = {
    "type": "object",
    "properties": {
        "category": {"type": "string", "enum": list(CATEGORIES)},
        "evidence": {"type": "string"},
    },
    "required": ["category", "evidence"],
    "additionalProperties": False,
}
OPENER = build_opener(ProxyHandler({}))


def endpoint() -> str:
    parsed = urlparse(BASE_URL)
    if parsed.scheme != "http" or not parsed.hostname:
        raise SystemExit("Evaluation permits only an HTTP loopback Foundation Models URL")
    try:
        loopback = parsed.hostname == "localhost" or ipaddress.ip_address(parsed.hostname).is_loopback
    except ValueError:
        loopback = False
    if not loopback:
        raise SystemExit("Evaluation permits only an HTTP loopback Foundation Models URL")
    return BASE_URL.rstrip("/") + "/chat/completions"


def classify(text: str, url: str) -> dict:
    payload = json.dumps({
        "model": MODEL_ID,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": f"Document text:\n{text}"},
        ],
        "max_tokens": 160,
        "temperature": 0,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "document_category",
                "strict": True,
                "schema": JSON_SCHEMA,
            },
        },
    }, ensure_ascii=False).encode("utf-8")
    if len(payload) > 16 * 1024:
        raise ValueError("synthetic classification request exceeds the 16 KiB evaluation limit")
    request = Request(url, data=payload, headers={"Content-Type": "application/json"})
    try:
        with OPENER.open(request, timeout=45) as response:
            result = json.load(response)
    except HTTPError as error:
        body = error.read(2048).decode("utf-8", "replace")
        try:
            detail = json.loads(body).get("error", {}).get("message", "")
        except (json.JSONDecodeError, AttributeError):
            detail = ""
        if "safety guardrails" in detail.lower():
            return {"category": None, "evidence": None, "parse_error": False, "failure": "safety_block"}
        raise RuntimeError(f"local Foundation Models returned HTTP {error.code}: {detail[:240]}") from error
    if not isinstance(result, dict) or result.get("model") != MODEL_ID:
        raise RuntimeError("Foundation Models returned an unexpected model identity")
    choices = result.get("choices")
    message = choices[0].get("message") if isinstance(choices, list) and choices else None
    content = message.get("content") if isinstance(message, dict) else None
    if not isinstance(content, str):
        raise RuntimeError("Foundation Models omitted message content")
    try:
        answer = json.loads(content)
    except json.JSONDecodeError:
        return {"category": None, "evidence": None, "parse_error": True}
    if not isinstance(answer, dict):
        return {"category": None, "evidence": None, "parse_error": True}
    category = answer.get("category")
    evidence = answer.get("evidence")
    return {
        "category": category if category in CATEGORIES else None,
        "evidence": evidence if isinstance(evidence, str) else None,
        "parse_error": False,
    }


def grouped_metrics(rows: list[dict], field: str) -> dict:
    groups: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        groups[row[field]].append(row)
    summary = {}
    for key, group in sorted(groups.items()):
        valid = [row for row in group if row["proposal_valid"]]
        summary[key] = {
            "samples": len(group),
            "category_predictions": sum(row["predicted"] is not None for row in group),
            "category_correct": sum(row["correct"] for row in group),
            "valid_proposals": len(valid),
            "valid_proposals_correct": sum(row["correct"] for row in valid),
        }
    return summary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", default="clm-corpus-hybrid-v1.json", help="synthetic JSON corpus in this directory")
    parser.add_argument("--preblocked-id", action="append", default=[], help="record a prior local safety block without retrying that sample")
    args = parser.parse_args()
    eval_dir = Path(__file__).resolve().parent
    corpus_path = (eval_dir / args.corpus).resolve()
    if corpus_path.parent != eval_dir:
        raise SystemExit("Corpus must be a file directly inside the evaluation directory")
    raw_corpus = corpus_path.read_bytes()
    samples = json.loads(raw_corpus)
    if not isinstance(samples, list) or not samples:
        raise SystemExit("Corpus must contain at least one sample")
    if corpus_path.name in {"clm-corpus-hybrid-v1.json", "clm-corpus-combined-holdout-v1.json"}:
        counts = Counter((sample.get("language"), sample.get("expected")) for sample in samples if isinstance(sample, dict))
        difficulty_counts = Counter(
            (sample.get("language"), sample.get("expected"), sample.get("difficulty"))
            for sample in samples if isinstance(sample, dict)
        )
        if any(counts[(language, category)] != 8 for language in ("en", "de") for category in CATEGORIES):
            raise SystemExit("balanced corpus must have eight samples per language and category")
        if any(
            difficulty_counts[(language, category, difficulty)] != expected
            for language in ("en", "de")
            for category in CATEGORIES
            for difficulty, expected in (("clear", 6), ("overlap", 2))
        ):
            raise SystemExit("balanced corpus must have six clear and two overlapping samples per cell")

    ids: set[str] = set()
    known_ids = {sample.get("id") for sample in samples if isinstance(sample, dict)}
    preblocked_ids = set(args.preblocked_id)
    if not preblocked_ids.issubset(known_ids):
        raise SystemExit("--preblocked-id must name an id in the synthetic corpus")
    url = endpoint()
    results = []
    confusion: Counter[tuple[str, str]] = Counter()
    for sample in samples:
        if (
            not isinstance(sample, dict)
            or sample.get("language") not in {"en", "de"}
            or sample.get("expected") not in CATEGORIES
            or sample.get("difficulty") not in {"clear", "overlap"}
            or not isinstance(sample.get("id"), str)
            or not isinstance(sample.get("text"), str)
            or not sample["text"].strip()
            or sample["id"] in ids
        ):
            raise SystemExit("Corpus contains an invalid or duplicate synthetic sample")
        ids.add(sample["id"])
        if sample["id"] in preblocked_ids:
            answer = {"category": None, "evidence": None, "parse_error": False, "failure": "safety_block"}
        else:
            try:
                answer = classify(sample["text"], url)
            except Exception as error:
                raise SystemExit(f"Synthetic sample {sample['id']} failed: {error}") from error
        predicted = answer["category"]
        evidence = answer["evidence"]
        start = sample["text"].find(evidence) if evidence else -1
        evidence_exact = bool(evidence) and start >= 0
        evidence_words = len(evidence.split()) if evidence else 0
        evidence_characters = len(evidence) if evidence else 0
        evidence_within_limits = evidence_words <= 20 and evidence_characters <= 160
        evidence_valid = evidence_exact and evidence_within_limits
        correct = predicted == sample["expected"]
        proposal_valid = predicted is not None and evidence_valid
        if predicted is not None:
            confusion[(sample["expected"], predicted)] += 1
        results.append({
            "id": sample["id"],
            "language": sample["language"],
            "expected": sample["expected"],
            "difficulty": sample["difficulty"],
            "predicted": predicted,
            "correct": correct,
            "confidence": None,
            "evidence_output": evidence,
            "evidence_exact": evidence_exact,
            "evidence_valid": evidence_valid,
            "evidence_words": evidence_words,
            "evidence_characters": evidence_characters,
            "evidence_span": {
                "start": start,
                "end": start + len(evidence),
                "text": evidence,
            } if evidence_exact else None,
            "proposal_valid": proposal_valid,
            "parse_error": answer["parse_error"],
            "failure": answer.get("failure"),
            "preblocked": sample["id"] in preblocked_ids,
        })

    correct = sum(row["correct"] for row in results)
    valid = [row for row in results if row["proposal_valid"]]
    report = {
        "model": MODEL_ID,
        "model_revision": None,
        "backend": "foundation-models",
        "prompt_version": PROMPT_VERSION,
        "prompt_sha256": hashlib.sha256(SYSTEM_PROMPT.encode("utf-8")).hexdigest(),
        "prompt": SYSTEM_PROMPT,
        "generation": {"temperature": 0, "max_tokens": 160},
        "response_format": "strict JSON Schema for category enum and evidence string; exactness and length limits are enforced by the evaluator",
        "corpus": corpus_path.name,
        "corpus_sha256": hashlib.sha256(raw_corpus).hexdigest(),
        "samples": len(results),
        "category_predictions": sum(row["predicted"] is not None for row in results),
        "category_correct": correct,
        "category_accuracy": round(correct / len(results), 4),
        "valid_proposals": len(valid),
        "valid_proposals_correct": sum(row["correct"] for row in valid),
        "valid_proposal_precision": round(sum(row["correct"] for row in valid) / len(valid), 4) if valid else None,
        "valid_proposal_coverage": round(len(valid) / len(results), 4),
        "invalid_evidence": sum(row["predicted"] is not None and not row["evidence_exact"] for row in results),
        "over_limit_evidence": sum(row["evidence_exact"] and not row["evidence_valid"] for row in results),
        "parse_errors": sum(row["parse_error"] for row in results),
        "safety_blocks": sum(row["failure"] == "safety_block" for row in results),
        "preblocked_ids": sorted(preblocked_ids),
        "by_language": grouped_metrics(results, "language"),
        "by_expected_category": grouped_metrics(results, "expected"),
        "confusion": {
            f"{expected} -> {predicted}": count
            for (expected, predicted), count in sorted(confusion.items())
        },
        "results": results,
        "warning": "Synthetic results only; this one-shot evaluation does not authorize document dispatch or automatic filing.",
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
