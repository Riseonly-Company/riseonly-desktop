#!/usr/bin/env python3
"""Enforce the architectural boundaries that keep the port portable.

These are the rules that decay silently if only a human watches them, so CI
watches them instead. Each violation prints file:line and fails the build.

  1. rise-engine never sees the UI toolkit or an OS crate. It is the layer that
     must stay compilable headless on every target, including a future FFI or
     wasm consumer.
  2. #[cfg(target_os)] lives only in rise-platform and rise-media. The moment it
     appears in a feature module, cross-platform behaviour stops being a
     property of the design and becomes three diverging branches.
  3. stores/ never reaches the engine directly, and engine/ never reaches the UI
     toolkit. This is STORE_ARCHITECTURE.md section 2 expressed as a check.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"
APP = ROOT / "app"

# Anchored at a path or import position on purpose: a bare word match turns
# slice.windows(2) into a "depends on the windows crate" violation.
_FORBIDDEN_CRATES = (
    "gpui|gpui_platform|objc2?|cocoa|core_foundation|windows|winapi|x11|wayland_client|keyring"
)
FORBIDDEN_IN_ENGINE = re.compile(
    rf"(?:^|[^.\w])(?:use\s+|extern\s+crate\s+)?({_FORBIDDEN_CRATES})\s*(?:::|;|$)"
)
FORBIDDEN_IN_MANIFEST = re.compile(rf"^\s*({_FORBIDDEN_CRATES})\s*[.=]")
CFG_TARGET_OS = re.compile(r"#\[cfg\((?:[^)]*\b)?target_os\s*=")
ENGINE_IMPORT = re.compile(r"\buse\s+rise_engine\b|\brise_engine::")
GPUI_IMPORT = re.compile(r"\buse\s+gpui\b|\bgpui::")

CFG_ALLOWED_CRATES = {"rise-platform", "rise-media"}


class Violations:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, path: Path, lineno: int, rule: str, detail: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.items.append(f"{rel}:{lineno}  [{rule}] {detail}")

    def report(self) -> int:
        if not self.items:
            print("boundaries: clean")
            return 0
        print(f"boundaries: {len(self.items)} violation(s)\n")
        for item in self.items:
            print(f"  {item}")
        print("\nSee docs/ARCHITECTURE_BOUNDARIES.md for why each rule exists.")
        return 1


def strip_comments(line: str) -> str:
    return line.split("//", 1)[0]


def check() -> int:
    violations = Violations()

    engine_manifest = (CRATES / "rise-engine" / "Cargo.toml")
    if engine_manifest.is_file():
        for lineno, line in enumerate(engine_manifest.read_text().splitlines(), 1):
            if FORBIDDEN_IN_MANIFEST.search(strip_comments(line)):
                violations.add(engine_manifest, lineno, "engine-purity",
                               f"rise-engine must not depend on: {line.strip()}")

    targets = [p for p in CRATES.iterdir() if p.is_dir()]
    if APP.is_dir():
        targets.append(APP)

    for crate_dir in sorted(targets):
        crate = crate_dir.name
        for path in (crate_dir / "src").rglob("*.rs"):
            rel = path.relative_to(crate_dir / "src").as_posix()
            try:
                lines = path.read_text().splitlines()
            except UnicodeDecodeError:
                continue

            for lineno, raw in enumerate(lines, 1):
                line = strip_comments(raw)

                if crate == "rise-engine" and FORBIDDEN_IN_ENGINE.search(line):
                    violations.add(path, lineno, "engine-purity",
                                   "rise-engine must stay free of UI and OS crates")

                if CFG_TARGET_OS.search(line) and crate not in CFG_ALLOWED_CRATES:
                    violations.add(path, lineno, "cfg-containment",
                                   "target_os belongs in rise-platform or rise-media")

                if crate == "app":
                    if "/stores/" in rel and ENGINE_IMPORT.search(line):
                        violations.add(path, lineno, "store-layering",
                                       "stores/ must go through the module's engine/, "
                                       "never rise_engine directly")
                    if "/engine/" in rel and GPUI_IMPORT.search(line):
                        violations.add(path, lineno, "engine-layering",
                                       "engine/ owns data, not presentation; gpui is not allowed")

    return violations.report()


if __name__ == "__main__":
    if not CRATES.is_dir():
        print(f"no crates directory at {CRATES}", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(check())
