#!/usr/bin/env python3
"""Interactively review private G2d OCR observations on the operator's local TTY."""
from __future__ import annotations

import argparse
import copy
import json
import sys
from typing import TextIO

from g2d_ground_truth import private_session, read_private, token, write_private

VALID_DIRECTIONS = ("ME", "THEM")
REQUIRED_TAGS = {
    "private",
    "group",
    "duplicate_text",
    "numeric",
    "multiline",
    "reference",
}


def yn(prompt: str, default: bool, inp: TextIO, out: TextIO) -> bool:
    suffix = " [Y/n] " if default else " [y/N] "
    while True:
        out.write(prompt + suffix)
        out.flush()
        value = inp.readline()
        if value == "":
            raise EOFError
        value = value.strip().lower()
        if not value:
            return default
        if value in {"y", "yes"}:
            return True
        if value in {"n", "no"}:
            return False
        out.write("请输入 y 或 n。\n")


def direction_prompt(current: str, inp: TextIO, out: TextIO) -> str:
    default = current if current in VALID_DIRECTIONS else "THEM"
    while True:
        out.write(f"方向 ME/THEM [{default}]: ")
        out.flush()
        value = inp.readline()
        if value == "":
            raise EOFError
        value = value.strip().upper() or default
        if value in VALID_DIRECTIONS:
            return value
        out.write("只能输入 ME 或 THEM。\n")


def corrected_message(observed: dict, inp: TextIO, out: TextIO) -> dict:
    out.write("正确文字（直接回车保留当前识别）: ")
    out.flush()
    raw = inp.readline()
    if raw == "":
        raise EOFError
    text = raw.rstrip("\n")
    if not text:
        text = observed["text"]
    direction = direction_prompt(str(observed.get("direction", "")), inp, out)
    sender = yn(
        "这条消息在界面上是否有明确 sender 标签？",
        bool(observed.get("sender_labeled")),
        inp,
        out,
    )
    return {"text": text, "direction": direction, "sender_labeled": sender}


def new_message(inp: TextIO, out: TextIO) -> dict:
    while True:
        out.write("漏识别消息的完整文字: ")
        out.flush()
        raw = inp.readline()
        if raw == "":
            raise EOFError
        text = raw.rstrip("\n")
        if text.strip():
            break
        out.write("文字不能为空。\n")
    direction = direction_prompt("THEM", inp, out)
    sender = yn("这条消息在界面上是否有明确 sender 标签？", False, inp, out)
    return {"text": text, "direction": direction, "sender_labeled": sender}


def review_case(case: dict, inp: TextIO, out: TextIO) -> list[dict]:
    case_id = case.get("id", "?")
    tags = ",".join(case.get("tags", []))
    observed = case.get("observed", [])
    out.write(f"\n=== Case: {case_id}  tags={tags}  observed={len(observed)} ===\n")
    expected: list[dict] = []
    for index, message in enumerate(observed, start=1):
        out.write(f"\n[{index}/{len(observed)}]\n")
        out.write(f"文字: {message['text']}\n")
        out.write(
            f"方向: {message['direction']}    sender_labeled: "
            f"{str(message['sender_labeled']).lower()}\n"
        )
        while True:
            out.write("处理：[Enter/Y]正确  [E]修改  [D]误识别删除: ")
            out.flush()
            action = inp.readline()
            if action == "":
                raise EOFError
            action = action.strip().lower()
            if action in {"", "y", "yes"}:
                expected.append(copy.deepcopy(message))
                break
            if action in {"e", "edit"}:
                expected.append(corrected_message(message, inp, out))
                break
            if action in {"d", "delete"}:
                out.write("已从 expected 排除这条 observed。\n")
                break
            out.write("请输入 Y、E 或 D。\n")

    while yn("是否有 OCR 完全漏掉、需要补进 expected 的消息？", False, inp, out):
        expected.append(new_message(inp, out))

    out.write(f"Case {case_id} expected={len(expected)}。\n")
    return expected


def review_document(document: object, inp: TextIO, out: TextIO, redo: bool = False) -> tuple[dict, int]:
    if not isinstance(document, dict) or not isinstance(document.get("cases"), list):
        raise ValueError("invalid groundtruth document")
    result = copy.deepcopy(document)
    reviewed = 0
    for case in result["cases"]:
        if not isinstance(case, dict):
            raise ValueError("invalid case")
        tags = case.get("tags")
        if not isinstance(tags, list) or not REQUIRED_TAGS.intersection(tags):
            continue
        existing = case.get("expected")
        if not redo and isinstance(existing, list) and existing:
            out.write(f"跳过已审核 case: {case.get('id', '?')}\n")
            continue
        case["expected"] = review_case(case, inp, out)
        reviewed += 1
    return result, reviewed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session")
    parser.add_argument("--redo", action="store_true", help="重新审核已有 expected 的 case")
    args = parser.parse_args()

    if not token(args.session):
        print("INVALID_SESSION", file=sys.stderr)
        return 2
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        print(
            "REFUSED_NON_INTERACTIVE: reviewer may display private chat text; run it directly in your local terminal.",
            file=sys.stderr,
        )
        return 3

    try:
        directory = private_session(args.session)
        path = directory / "groundtruth.json"
        document = read_private(path)
        updated, reviewed = review_document(document, sys.stdin, sys.stdout, redo=args.redo)
        if reviewed == 0:
            print("没有需要审核的 case。")
            return 0
        print("\n即将把人工确认结果写回本机私有 groundtruth.json。")
        if not yn("确认保存？", True, sys.stdin, sys.stdout):
            print("未保存。")
            return 1
        write_private(path, updated)
        print(f"已保存 {reviewed} 个 case。下一步：python3 scripts/g2d_ground_truth.py {args.session}")
        return 0
    except (OSError, ValueError, json.JSONDecodeError, UnicodeError, EOFError):
        print("REVIEW_FAILED", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
