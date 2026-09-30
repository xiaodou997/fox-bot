#!/usr/bin/env python3
"""Guide the private G2d real-acceptance workflow without exposing chat content."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from g2d_ground_truth import private_session, read_private, token

REQUIRED_CASES = [
    ("private-basic", "private"),
    ("group-sender", "group"),
    ("duplicate-text", "duplicate_text"),
    ("numeric-money", "numeric"),
    ("multiline", "multiline"),
    ("reference", "reference"),
]
WORKER = "target/macos-probe/debug/foxbot-macos-ocr"


def case_tags(document: object) -> set[str]:
    if not isinstance(document, dict) or not isinstance(document.get("cases"), list):
        return set()
    tags: set[str] = set()
    for case in document["cases"]:
        if isinstance(case, dict) and isinstance(case.get("tags"), list):
            tags.update(tag for tag in case["tags"] if isinstance(tag, str))
    return tags


def labeled_count(document: object) -> int:
    if not isinstance(document, dict) or not isinstance(document.get("cases"), list):
        return 0
    total = 0
    for case in document["cases"]:
        expected = case.get("expected") if isinstance(case, dict) else None
        if isinstance(expected, list):
            total += len(expected)
    return total


def missing_expected_cases(document: object) -> list[str]:
    if not isinstance(document, dict) or not isinstance(document.get("cases"), list):
        return []
    missing = []
    for case in document["cases"]:
        if not isinstance(case, dict):
            continue
        tags = case.get("tags")
        expected = case.get("expected")
        if (
            isinstance(tags, list)
            and any(tag in {value for _, value in REQUIRED_CASES} for tag in tags)
            and isinstance(expected, list)
            and not expected
        ):
            case_id = case.get("id")
            if isinstance(case_id, str):
                missing.append(case_id)
    return missing


def command_capture(session: str, case_id: str, tag: str) -> str:
    return (
        "cargo run --quiet --locked -p foxbot-host -- "
        f"g2d-private-capture {WORKER} {session} {case_id} {tag} "
        "--allow-private-test-data"
    )


def status(session: str) -> dict:
    directory = private_session(session)
    groundtruth_path = directory / "groundtruth.json"
    acceptance_path = directory / "acceptance.json"
    bridge_path = directory / "bridge-config.json"
    baseline_path = directory / "baseline-snapshot.json"
    verified_path = directory / "verified-snapshot.json"

    groundtruth = None
    if groundtruth_path.exists():
        groundtruth = read_private(groundtruth_path)
    tags = case_tags(groundtruth)
    missing = [(case_id, tag) for case_id, tag in REQUIRED_CASES if tag not in tags]
    expected_missing = missing_expected_cases(groundtruth)
    labeled = labeled_count(groundtruth)

    acceptance = None
    if acceptance_path.exists():
        acceptance = read_private(acceptance_path)
    accepted = bool(isinstance(acceptance, dict) and acceptance.get("accepted") is True)

    result = {
        "schema_version": "foxbot.g2d-real-guide.v1",
        "session": session,
        "raw_text_included": False,
        "required_cases": len(REQUIRED_CASES),
        "captured_required_tags": len({tag for _, tag in REQUIRED_CASES} & tags),
        "labeled_messages": labeled,
        "accepted": accepted,
        "baseline_ready": bridge_path.is_file() and baseline_path.is_file(),
        "verified": verified_path.is_file(),
    }

    if verified_path.is_file():
        result.update(
            stage="COMPLETE",
            next_action="NONE",
            operator_action_required=False,
        )
        return result

    if missing:
        case_id, tag = missing[0]
        result.update(
            stage="CAPTURE_CASES",
            missing_tags=[tag for _, tag in missing],
            next_case=case_id,
            operator_action_required=True,
            operator_instruction=f"Place WeChat on the dedicated {tag} test scenario, then run next_command.",
            next_command=command_capture(session, case_id, tag),
        )
        return result

    if expected_missing or labeled < 24:
        result.update(
            stage="LABEL_EXPECTED",
            expected_missing_cases=expected_missing,
            operator_action_required=True,
            operator_instruction=(
                f"Edit target/g2d-real/{session}/groundtruth.json locally. "
                "Fill expected from what you can visually verify; do not copy observed blindly."
            ),
            next_command=f"python3 scripts/g2d_ground_truth.py {session}",
        )
        return result

    if acceptance is None:
        result.update(
            stage="RUN_ACCEPTANCE",
            operator_action_required=False,
            next_command=f"python3 scripts/g2d_ground_truth.py {session}",
        )
        return result

    if not accepted:
        result.update(
            stage="FIX_GROUND_TRUTH",
            operator_action_required=True,
            operator_instruction=(
                f"Review target/g2d-real/{session}/groundtruth.json against the dedicated test chats, "
                "fix the expected labels or parser discrepancies, then rerun next_command."
            ),
            next_command=f"python3 scripts/g2d_ground_truth.py {session}",
        )
        return result

    if not (bridge_path.is_file() and baseline_path.is_file()):
        result.update(
            stage="READY_BASELINE",
            operator_action_required=True,
            operator_instruction=(
                "Keep WeChat on the accepted dedicated test conversation. "
                "Choose stable opaque test IDs and run the baseline command."
            ),
            next_command=(
                "cargo run --quiet --locked -p foxbot-host -- "
                f"g2d-real-baseline {WORKER} {session} "
                "test-account test-conversation private --allow-private-test-data"
            ),
        )
        return result

    result.update(
        stage="WAIT_EXTERNAL_MESSAGE",
        operator_action_required=True,
        operator_instruction=(
            "From the other test account, send exactly one known incoming message to the same conversation. "
            "Do not scroll, switch conversations, or send anything from this Mac; then run next_command."
        ),
        next_command=(
            "cargo run --quiet --locked -p foxbot-host -- "
            f"g2d-real-verify {WORKER} {session} --allow-private-test-data"
        ),
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["init", "status"])
    parser.add_argument("session")
    args = parser.parse_args()
    if not token(args.session):
        print(json.dumps({"status": "INVALID_SESSION", "raw_text_included": False}))
        return 2
    try:
        directory = private_session(args.session)
        if args.command == "init":
            result = {
                "schema_version": "foxbot.g2d-real-guide.v1",
                "status": "INITIALIZED",
                "session": args.session,
                "private_directory": str(directory.relative_to(Path.cwd())),
                "raw_text_included": False,
                "next_command": f"python3 scripts/g2d_real_status.py status {args.session}",
            }
        else:
            result = status(args.session)
    except (OSError, ValueError, json.JSONDecodeError, UnicodeError):
        print(json.dumps({"status": "INVALID_PRIVATE_STATE", "raw_text_included": False}))
        return 2
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
