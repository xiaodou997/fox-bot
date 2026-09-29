#!/usr/bin/env python3
"""Evaluate a private G2d session and persist only a redacted acceptance result."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import tempfile

from g2c_ground_truth import evaluate

ROOT = Path(__file__).resolve().parents[1]
PRIVATE_ROOT = ROOT / "target" / "g2d-real"
MAX_BYTES = 2 * 1024 * 1024


def token(value: str) -> bool:
    return bool(value) and len(value) <= 64 and all(c.isalnum() or c in "._-" for c in value)


def private_session(session: str) -> Path:
    if not token(session):
        raise ValueError("session")
    PRIVATE_ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(PRIVATE_ROOT, 0o700)
    root = PRIVATE_ROOT.resolve()
    directory = root / session
    directory.mkdir(mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    resolved = directory.resolve()
    if resolved.parent != root or directory.is_symlink():
        raise ValueError("session path")
    return resolved


def read_private(path: Path) -> object:
    stat = os.lstat(path)
    if not path.is_file() or path.is_symlink() or stat.st_size > MAX_BYTES or stat.st_mode & 0o077:
        raise ValueError("private input")
    return json.loads(path.read_text(encoding="utf-8"))


def write_private(path: Path, value: object) -> None:
    if path.exists():
        stat = os.lstat(path)
        if not path.is_file() or path.is_symlink() or stat.st_mode & 0o077:
            raise ValueError("private output")
    payload = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    if len(payload) > MAX_BYTES:
        raise ValueError("output size")
    fd, temp_name = tempfile.mkstemp(prefix=".acceptance.", suffix=".tmp", dir=path.parent)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb", closefd=True) as file:
            file.write(payload)
            file.flush()
            os.fsync(file.fileno())
        os.replace(temp_name, path)
    except Exception:
        try:
            os.close(fd)
        except OSError:
            pass
        try:
            os.unlink(temp_name)
        except OSError:
            pass
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session")
    args = parser.parse_args()
    try:
        directory = private_session(args.session)
        result = evaluate(read_private(directory / "groundtruth.json"))
        write_private(directory / "acceptance.json", result)
    except (OSError, ValueError, json.JSONDecodeError, UnicodeError):
        print(json.dumps({"status": "INVALID_GROUND_TRUTH", "raw_text_included": False}))
        return 2
    print(json.dumps({**result, "raw_text_included": False}, ensure_ascii=False, indent=2))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
