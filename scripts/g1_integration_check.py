#!/usr/bin/env python3
"""Run bounded integration gates with exact checkout fingerprints.

The optional macOS step compiles/tests native targets only. It never queries real chat apps,
performs draft writes, sends input, or mutates Keychain/TCC state.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def fingerprint():
    paths = sorted(set(git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")) - {b""})
    digest = hashlib.sha256()
    for raw in paths:
        path = ROOT / os.fsdecode(raw)
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("tracked input is not a regular file")
        digest.update(len(raw).to_bytes(8, "big")); digest.update(raw)
        data = path.read_bytes()
        digest.update(len(data).to_bytes(8, "big")); digest.update(data)
    return digest.hexdigest()


TEST_MINIMUMS = {"rust-tests": 151, "cipher-disabled": 1, "python-tests": 61, "swift-tests": 113}


def executed_tests(label, log):
    """Count terminal success summaries, not the number of test functions in source."""
    if label in {"rust-tests", "cipher-disabled"}:
        return sum(int(n) for n in re.findall(r"^test result: ok\. (\d+) passed; 0 failed;", log, re.MULTILINE))
    if label == "python-tests":
        return max([int(n) for n in re.findall(r"^Ran (\d+) tests? in ", log, re.MULTILINE)] or [0])
    if label == "swift-tests":
        # SwiftPM can launch multiple XCTest bundles. Count each bundle once, not its enclosing suites.
        bundles = {}
        for name, count in re.findall(r"Test Suite '([^'\n]+\.xctest)' passed[^\n]*\n\s*Executed (\d+) tests?, with 0 failures", log):
            bundles[name] = max(bundles.get(name, 0), int(count))
        return sum(bundles.values())
    return None


def run_step(label, command, directory, timeout=600):
    start = time.monotonic()
    with (directory / (label + ".log")).open("wb") as log:
        process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        timed_out = False
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True; code = None
        finally:
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            process.wait()
    count = executed_tests(label, (directory / (label + ".log")).read_text(encoding="utf-8", errors="replace"))
    minimum = TEST_MINIMUMS.get(label)
    proof = minimum is None or (count is not None and count >= minimum)
    result = {"step":label, "exit_code":code, "timeout":timed_out,
              "executed_tests":count, "minimum_tests":minimum,
              "seconds":round(time.monotonic()-start, 3), "passed":code == 0 and not timed_out and proof}
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--with-macos-probe", action="store_true",
                        help="build/test macOS native targets; tests never query or write real apps")
    args = parser.parse_args()
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    if not Path(cargo).is_file():
        raise RuntimeError("cargo is unavailable")
    directory = ROOT / "target/g1-integration" / uuid.uuid4().hex
    directory.mkdir(parents=True, mode=0o700)
    before = fingerprint()
    head = git("rev-parse", "HEAD").decode().strip()
    steps = [
        ("format", [cargo,"fmt","--all","--","--check"]),
        ("clippy", [cargo,"clippy","--workspace","--all-targets","--locked","--","-D","warnings"]),
        ("rust-tests", [cargo,"test","--workspace","--all-targets","--locked"]),
        ("cipher-disabled", [cargo,"test","--locked","-p","foxbot-host","--no-default-features","--lib",
                             "tests::disabled_cipher_feature_rejects_protected_entry_without_creating_plaintext","--","--exact"]),
        ("build-tools", [cargo,"build","--locked","-p","foxbot-host","-p","foxbot-http"]),
        ("g2d-bridge-smoke", [cargo,"run","--quiet","--locked","-p","foxbot-host","--",
                              "bridge-sim-probe",str(directory / "g2d-state"),"--allow-plaintext-synthetic"]),
        ("g3b-gate-smoke", [cargo,"run","--quiet","--locked","-p","foxbot-sim","--",
                            "gate-only",str(directory / "g3b-state")]),
        ("http-smoke", [sys.executable,"scripts/http_smoke.py"]),
        ("host-smoke", [sys.executable,"scripts/host_smoke.py"]),
        ("python-tests", [sys.executable,"-m","unittest","discover","-s","scripts/tests","-p","test_*.py"]),
        ("docs", [sys.executable,"scripts/check_docs.py"]),
        ("whitespace", ["git","diff","--check"]),
    ]
    if args.with_macos_probe:
        if sys.platform != "darwin":
            raise RuntimeError("macOS probe validation requires macOS")
        steps.append(("swift-tests", ["/usr/bin/xcrun","swift","test","--package-path","native/macos-probe",
                                      "--scratch-path","target/macos-probe","-Xswiftc","-warnings-as-errors"]))
    report = {"head":head, "source_before":before, "native_chat_operations":0, "keychain_operations":0,
              "checks":[], "external_model_requests":0}
    try:
        for label, command in steps:
            result = run_step(label, command, directory)
            report["checks"].append(result)
            if not result["passed"]:
                break
    finally:
        report["source_after"] = fingerprint()
        report["head_after"] = git("rev-parse", "HEAD").decode().strip()
        report["source_unchanged"] = before == report["source_after"] and head == report["head_after"]
        report["passed"] = (len(report["checks"]) == len(steps) and report["source_unchanged"]
                            and all(check["passed"] for check in report["checks"]))
        (directory / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        print(json.dumps({"passed":report["passed"], "source_unchanged":report["source_unchanged"],
                          "report":str(directory.relative_to(ROOT) / "report.json")}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
