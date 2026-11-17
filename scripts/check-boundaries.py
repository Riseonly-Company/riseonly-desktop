#!/usr/bin/env python3
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"
APP = ROOT / "app"
APP_CRATE = "app"
GENERATED_PREFIX = "// @generated"
BLOCK_BEGIN = "// @generated mod list begin"
BLOCK_END = "// @generated mod list end"
OS_NAMES = {"macos", "linux", "windows"}

PENDING_CFG = {
    "crates/rise-platform/src/reveal_in_file_manager/reveal_in_file_manager.rs": "P00",
}

_FORBIDDEN_CRATES = (
    "gpui|gpui_platform|objc2?|cocoa|core_foundation|windows|winapi|x11|wayland_client|keyring"
)
FORBIDDEN_IN_ENGINE = re.compile(
    rf"(?:^|[^.\w])(?:use\s+|extern\s+crate\s+)?({_FORBIDDEN_CRATES})\s*(?:::|;|$)"
)
FORBIDDEN_IN_MANIFEST = re.compile(rf"^\s*({_FORBIDDEN_CRATES})\s*[.=]")

OS_CFG_KEY = re.compile(r"\btarget_(?:os|family)\s*=")
OS_CFG_SHORTHAND = re.compile(
    r"\bcfg(?:_attr)?\s*!?\s*\((?:.*[(,\s])?(?:unix|windows)\s*[),]"
)


class Violations:
    def __init__(self) -> None:
        self.items: list[str] = []
        self.notes: list[str] = []

    def add(self, path: Path, lineno: int, rule: str, detail: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.items.append(f"{rel}:{lineno}  [{rule}] {detail}")

    def note(self, text: str) -> None:
        self.notes.append(text)

    def report(self) -> int:
        for text in self.notes:
            print(f"note: {text}")
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


def generated_lines(lines: list[str]) -> set[int]:
    if lines and lines[0].startswith(GENERATED_PREFIX) and not lines[0].startswith(BLOCK_BEGIN):
        return set(range(1, len(lines) + 1))
    out: set[int] = set()
    inside = False
    for lineno, raw in enumerate(lines, 1):
        stripped = raw.strip()
        if stripped.startswith(BLOCK_BEGIN):
            inside = True
        if inside:
            out.add(lineno)
        if stripped.startswith(BLOCK_END):
            inside = False
    return out


def has_os_cfg(line: str) -> bool:
    return bool(OS_CFG_KEY.search(line) or OS_CFG_SHORTHAND.search(line))


def check_os_layer_placement(violations: Violations, crate: str, src: Path) -> None:
    for path in sorted(src.rglob("*")):
        rel = path.relative_to(src)
        parts = rel.parts
        in_feature = crate == APP_CRATE and parts[0] == "modules"
        in_store = "stores" in parts[:-1]
        if not (in_feature or in_store):
            continue
        name = path.stem if path.is_file() else path.name
        if name in OS_NAMES and (path.is_dir() or path.suffix == ".rs"):
            violations.add(path, 1, "os-layer-placement",
                           "feature and store code has no OS layer: call a "
                           "rise-platform or rise-media seam instead")


def check() -> int:
    violations = Violations()

    engine_manifest = CRATES / "rise-engine" / "Cargo.toml"
    if engine_manifest.is_file():
        for lineno, line in enumerate(engine_manifest.read_text().splitlines(), 1):
            if FORBIDDEN_IN_MANIFEST.search(strip_comments(line)):
                violations.add(engine_manifest, lineno, "engine-purity",
                               f"rise-engine must not depend on: {line.strip()}")

    targets = [p for p in CRATES.iterdir() if p.is_dir()]
    if APP.is_dir():
        targets.append(APP)

    pending_hit: set[str] = set()

    for crate_dir in sorted(targets):
        crate = crate_dir.name
        src = crate_dir / "src"
        if not src.is_dir():
            continue
        check_os_layer_placement(violations, crate, src)

        for path in sorted(src.rglob("*.rs")):
            rel_root = path.relative_to(ROOT).as_posix()
            try:
                lines = path.read_text().splitlines()
            except UnicodeDecodeError:
                continue
            generated = generated_lines(lines)

            for lineno, raw in enumerate(lines, 1):
                line = strip_comments(raw)

                if crate == "rise-engine" and FORBIDDEN_IN_ENGINE.search(line):
                    violations.add(path, lineno, "engine-purity",
                                   "rise-engine must stay free of UI and OS crates")

                if lineno in generated or not has_os_cfg(line):
                    continue
                if rel_root in PENDING_CFG:
                    pending_hit.add(rel_root)
                    continue
                violations.add(path, lineno, "cfg-containment",
                               "OS cfg comes only from gen-mod-tree.py: move the "
                               "body into macos.rs, linux.rs or windows.rs")

    for rel_root, card in sorted(PENDING_CFG.items()):
        if rel_root in pending_hit:
            violations.note(f"{rel_root}: hand OS cfg allowed until {card} lands")
        else:
            violations.note(f"{rel_root}: stale PENDING_CFG entry ({card}), delete it")

    return violations.report()


if __name__ == "__main__":
    if not CRATES.is_dir():
        print(f"no crates directory at {CRATES}", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(check())
