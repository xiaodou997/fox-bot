#!/usr/bin/env python3
"""Evaluate local-only MessageSnapshot annotations without emitting raw chat text."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import sys
import unicodedata

ROOT = Path(__file__).resolve().parents[1]
PRIVATE_ROOT = ROOT / "target" / "g2c-groundtruth"
REQUIRED_TAGS = {"private", "group", "duplicate_text", "numeric", "multiline", "reference"}
VALID_DIRECTIONS = {"ME", "THEM", "UNKNOWN"}
MAX_FILE_BYTES = 1_048_576


def normalize_text(value: str) -> str:
    return unicodedata.normalize("NFC", value.replace("\r\n", "\n").replace("\r", "\n")).strip()


def edit_distance(expected: str, observed: str) -> int:
    if expected == observed:
        return 0
    if not expected:
        return len(observed)
    if not observed:
        return len(expected)
    previous = list(range(len(observed) + 1))
    for row, left in enumerate(expected, start=1):
        current = [row]
        for column, right in enumerate(observed, start=1):
            current.append(min(
                current[-1] + 1,
                previous[column] + 1,
                previous[column - 1] + (left != right),
            ))
        previous = current
    return previous[-1]


def validate_message(value: object) -> dict:
    if not isinstance(value, dict) or set(value) != {"text", "direction", "sender_labeled"}:
        raise ValueError("message fields")
    if (not isinstance(value["text"], str) or not value["text"].strip()
            or len(value["text"].encode()) > 16_384
            or value["direction"] not in VALID_DIRECTIONS
            or type(value["sender_labeled"]) is not bool):
        raise ValueError("message value")
    return value


def evaluate(document: object) -> dict:
    if not isinstance(document, dict) or set(document) != {"schema_version", "strategy", "revision", "cases"}:
        raise ValueError("document fields")
    if document["schema_version"] != "foxbot.g2c-ground-truth.v1":
        raise ValueError("schema")
    if (not isinstance(document["strategy"], str) or not 1 <= len(document["strategy"]) <= 128
            or not isinstance(document["revision"], str) or not 1 <= len(document["revision"]) <= 128
            or not isinstance(document["cases"], list) or len(document["cases"]) > 1000):
        raise ValueError("document limits")

    covered = set()
    labeled = direction_errors = sender_errors = count_errors = text_errors = 0
    text_edit_distance = text_expected_characters = 0
    ids = set()
    for case in document["cases"]:
        if not isinstance(case, dict) or set(case) != {"id", "tags", "expected", "observed"}:
            raise ValueError("case fields")
        if (not isinstance(case["id"], str) or not 1 <= len(case["id"]) <= 128
                or case["id"] in ids
                or not isinstance(case["tags"], list) or not case["tags"]
                or any(not isinstance(tag, str) or not 1 <= len(tag) <= 64 for tag in case["tags"])
                or not isinstance(case["expected"], list) or not isinstance(case["observed"], list)
                or len(case["expected"]) > 64 or len(case["observed"]) > 64):
            raise ValueError("case limits")
        ids.add(case["id"])
        covered.update(case["tags"])
        expected = [validate_message(value) for value in case["expected"]]
        observed = [validate_message(value) for value in case["observed"]]
        labeled += len(expected)
        if len(expected) != len(observed):
            count_errors += 1
        for want, got in zip(expected, observed):
            if want["direction"] != got["direction"]:
                direction_errors += 1
            if want["sender_labeled"] != got["sender_labeled"]:
                sender_errors += 1
            want_text = normalize_text(want["text"])
            got_text = normalize_text(got["text"])
            text_expected_characters += len(want_text)
            distance = edit_distance(want_text, got_text)
            text_edit_distance += distance
            if distance:
                text_errors += 1

    text_error_rate_bp = (
        (text_edit_distance * 10_000 + text_expected_characters - 1) // text_expected_characters
        if text_expected_characters
        else 0
    )
    accepted = (
        len(document["cases"]) >= 6
        and labeled >= 24
        and REQUIRED_TAGS <= covered
        and direction_errors == 0
        and sender_errors == 0
        and count_errors == 0
        and text_edit_distance * 100 <= text_expected_characters * 2
    )
    return {
        "schema_version": "foxbot.g2c-ground-truth-result.v1",
        "strategy": document["strategy"],
        "revision": document["revision"],
        "accepted": accepted,
        "cases": len(document["cases"]),
        "labeled_messages": labeled,
        "covered_tags": sorted(covered),
        "direction_errors": direction_errors,
        "sender_errors": sender_errors,
        "message_count_errors": count_errors,
        "text_errors": text_errors,
        "text_edit_distance": text_edit_distance,
        "text_expected_characters": text_expected_characters,
        "text_error_rate_bp": text_error_rate_bp,
    }


def private_file(path: Path) -> Path:
    PRIVATE_ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    root = PRIVATE_ROOT.resolve()
    candidate = path.resolve(strict=True)
    if root not in candidate.parents:
        raise ValueError("ground-truth input must live under target/g2c-groundtruth")
    metadata = os.lstat(candidate)
    if not os.path.isfile(candidate) or os.path.islink(candidate) or metadata.st_size > MAX_FILE_BYTES:
        raise ValueError("unsafe ground-truth input")
    return candidate


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    args = parser.parse_args()
    try:
        path = private_file(args.input)
        document = json.loads(path.read_text(encoding="utf-8"))
        result = evaluate(document)
    except (OSError, ValueError, json.JSONDecodeError, UnicodeError):
        print(json.dumps({"status": "INVALID_GROUND_TRUTH", "raw_text_included": False}))
        return 2
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
