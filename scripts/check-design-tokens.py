#!/usr/bin/env python3
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

SCOPE = (
    "crates/rise-ui",
    "crates/rise-widgets",
    "crates/rise-navigation",
    "app",
)

STORYBOOK_PATHS = (
    "app/src/app/storybook",
)

TOKEN_SOURCES = (
    "crates/rise-theme/src/tokens/spacing.rs",
    "crates/rise-theme/src/tokens/icon.rs",
)
TOKEN_VALUE = re.compile(r"\b(\w+)\s*:\s*(?:l|(?:\w+\.)?scale)\(\s*(-?\d+(?:\.\d*)?)\s*\)")

ALLOW_FILE = "scripts/design-tokens-allow.json"
_VISUAL = "visual|\u0432\u0438\u0437\u0443\u0430\u043b\u044c\u043d\u043e\u0435"
ALLOW_REASON = re.compile(rf"^(?:{_VISUAL}),\s*\S+\.swift:\d+")

ESCAPE_CEILING = 12
ESCAPE = re.compile(r"//\s*design-token-exempt\b\s*:?\s*(.*)$")

_COLOUR_FNS = (
    "transparent_black|transparent_white|opaque_grey|rgba|rgb|hsla"
    "|white|black|red|blue|green|yellow"
)
COLOUR_CALL = re.compile(rf"(?:^|[^.\w])(?:gpui::)?({_COLOUR_FNS})\s*\(")
COLOUR_STRUCT = re.compile(r"\b(Rgba|Hsla)\s*\{")
TYPE_POSITION = re.compile(r"(->|:|<|\bas|\bstruct|\benum|\bimpl|\bfor|\btype)\s*$")
COLOUR_HEX = re.compile(r"\"#[0-9a-fA-F]{3,8}\"")

LENGTH_CALL = re.compile(r"(?:^|[^.\w])(?:gpui::)?(px|rems|Pixels|DevicePixels)\s*\(\s*(-?[\d.]+)")

_BOX_PREFIXES = (
    "min_size|max_size|min_w|min_h|max_w|max_h|gap_x|gap_y|inset|bottom|right"
    "|left|size|gap|top|mt|mb|my|mx|ml|mr|pt|pb|py|px|pl|pr|w|h|m|p"
)
_CORNER_PREFIXES = "rounded_tl|rounded_tr|rounded_bl|rounded_br|rounded_t|rounded_b|rounded_l|rounded_r|rounded"
_BORDER_PREFIXES = "border_t|border_b|border_l|border_r|border_x|border_y|border"
_NAMED_STEPS = "3xl|2xl|xs|sm|md|lg|xl|base"

BOX_STEP = re.compile(rf"\.(?:{_BOX_PREFIXES})_(?:neg_)?(\d+|px)\s*\(\s*\)")
CORNER_STEP = re.compile(rf"\.(?:{_CORNER_PREFIXES})_({_NAMED_STEPS})\s*\(\s*\)")
TEXT_STEP = re.compile(rf"\.text_({_NAMED_STEPS})\s*\(\s*\)")
BORDER_STEP = re.compile(rf"\.(?:{_BORDER_PREFIXES})_(\d+)\s*\(\s*\)")

CFG_TEST = re.compile(r"#\[cfg\((?:[^)]*\b)?test\b")
CHAR_LITERAL = re.compile(r"'(?:\\.|[^\\'])'")


class Violations:
    def __init__(self) -> None:
        self.items: list[str] = []
        self.escapes: list[str] = []
        self.warnings: list[str] = []

    def add(self, path: Path, lineno: int, rule: str, detail: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.items.append(f"{rel}:{lineno}  [{rule}] {detail}")

    def escape(self, path: Path, lineno: int, reason: str) -> None:
        rel = path.relative_to(ROOT).as_posix()
        self.escapes.append(f"{rel}:{lineno}  {reason}")

    def report(self, allowed: int) -> int:
        over_ceiling = len(self.escapes) > ESCAPE_CEILING

        for item in self.warnings:
            print(f"warning: {item}")
        if self.warnings:
            print()

        if self.items:
            print(f"design-tokens: {len(self.items)} violation(s)\n")
            for item in self.items:
                print(f"  {item}")
        else:
            print("design-tokens: clean")
        if allowed:
            print(f"\n{allowed} length literal(s) covered by {ALLOW_FILE}")

        if self.escapes:
            print(f"\nescape hatches: {len(self.escapes)} of {ESCAPE_CEILING} allowed")
            for item in self.escapes:
                print(f"  {item}")
        if over_ceiling:
            print(
                f"\nthe escape hatch is meant to be rare and {len(self.escapes)} "
                f"is past the ceiling of {ESCAPE_CEILING}: move these into "
                f"crates/rise-theme, cite the iOS line in {ALLOW_FILE}, "
                "or raise ESCAPE_CEILING with a reason."
            )

        if self.items or over_ceiling:
            print("\nSee the docstring of scripts/check-design-tokens.py for the rule.")
            return 1
        return 0


def number(text: str) -> float:
    return float(text.rstrip("."))


def token_values() -> dict[float, list[str]]:
    out: dict[float, list[str]] = {}
    for source in TOKEN_SOURCES:
        path = ROOT / source
        if not path.is_file():
            continue
        lines = path.read_text().splitlines()
        skip = test_lines(lines)
        stem = path.stem
        for lineno, raw in enumerate(lines, 1):
            if lineno in skip:
                continue
            for match in TOKEN_VALUE.finditer(code_only(raw)):
                value = number(match.group(2))
                if value != 0.0:
                    out.setdefault(value, []).append(f"{stem}.{match.group(1)}")
    return out


class Allow:

    def __init__(self, violations: Violations) -> None:
        self.rows: list[dict] = []
        self.used: set[int] = set()
        path = ROOT / ALLOW_FILE
        if not path.is_file():
            return
        try:
            data = json.loads(path.read_text())
        except (OSError, ValueError) as error:
            violations.warnings.append(f"{ALLOW_FILE} is unreadable: {error}")
            return
        items = data.get("items", []) if isinstance(data, dict) else data
        if not isinstance(items, list):
            violations.warnings.append(f"{ALLOW_FILE}: expected a list under \"items\"")
            return
        for row in items:
            if not self._valid(row):
                violations.warnings.append(
                    f"{ALLOW_FILE}: row ignored, it needs file, value and a reason "
                    f"\"visual, <iOS path>.swift:<line>\": {json.dumps(row, ensure_ascii=True)[:200]}"
                )
                continue
            self.rows.append(row)

    @staticmethod
    def _valid(row: object) -> bool:
        if not isinstance(row, dict) or not isinstance(row.get("file"), str):
            return False
        try:
            number(str(row.get("value", "")))
        except ValueError:
            return False
        line = row.get("line")
        if line is not None and not isinstance(line, int):
            return False
        return bool(ALLOW_REASON.match(str(row.get("reason", "")).strip()))

    def covers(self, rel: str, lineno: int, value: float) -> bool:
        for index, row in enumerate(self.rows):
            if row["file"] != rel or number(str(row["value"])) != value:
                continue
            if row.get("line") is not None and row["line"] != lineno:
                continue
            self.used.add(index)
            return True
        return False

    def report_unused(self, violations: Violations) -> None:
        for index, row in enumerate(self.rows):
            if index not in self.used:
                violations.warnings.append(
                    f"{ALLOW_FILE}: row covers nothing any more, drop it: "
                    f"{row['file']} value {row['value']}"
                )


def strip_comments(line: str) -> str:
    return line.split("//", 1)[0]


def code_only(line: str) -> str:
    line = CHAR_LITERAL.sub("' '", line)
    out: list[str] = []
    i = 0
    n = len(line)
    in_string = False
    while i < n:
        ch = line[i]
        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            i += 1
            continue
        if ch == "/" and i + 1 < n and line[i + 1] == "/":
            break
        out.append(ch)
        i += 1
    return "".join(out)


def test_lines(lines: list[str]) -> set[int]:
    inside: set[int] = set()
    depth = 0
    floor = 0
    pending = False
    skipping = False

    for lineno, raw in enumerate(lines, 1):
        code = code_only(raw)
        opens = code.count("{")
        closes = code.count("}")

        if not skipping and CFG_TEST.search(code):
            pending = True

        if pending and opens:
            skipping = True
            floor = depth
            pending = False
        elif pending and ";" in code:
            pending = False

        if skipping:
            inside.add(lineno)

        depth += opens - closes

        if skipping and depth <= floor:
            skipping = False

    return inside


def is_exempt(rel: str) -> bool:
    if any(part == "tests" for part in rel.split("/")) or rel.endswith("/tests.rs"):
        return True
    if rel.endswith("_tests.rs"):
        return True
    return any(rel == p or rel.startswith(p + "/") for p in STORYBOOK_PATHS)


def sources() -> list[Path]:
    out: list[Path] = []
    for area in SCOPE:
        src = ROOT / area / "src"
        if src.is_dir():
            out.extend(sorted(src.rglob("*.rs")))
    return out


def scan(path: Path, violations: Violations, tokens: dict[float, list[str]], allow: Allow) -> int:
    try:
        lines = path.read_text().splitlines()
    except UnicodeDecodeError:
        return 0

    rel = path.relative_to(ROOT).as_posix()
    skip = test_lines(lines)
    allowed = 0

    for lineno, raw in enumerate(lines, 1):
        if lineno in skip:
            continue

        escape = ESCAPE.search(raw)
        if escape:
            reason = escape.group(1).strip()
            if reason:
                violations.escape(path, lineno, reason)
                continue
            violations.add(path, lineno, "escape-unexplained",
                           "design-token-exempt needs a reason after the colon")
            continue

        line = code_only(raw)

        match = COLOUR_CALL.search(line)
        struct = COLOUR_STRUCT.search(line)
        if match:
            violations.add(path, lineno, "colour-literal",
                           f"{match.group(1)}() names a colour the palette should own")
        elif struct and not TYPE_POSITION.search(line[: struct.start(1)]):
            violations.add(path, lineno, "colour-literal",
                           "an Rgba/Hsla struct literal is a colour the palette should own")
        elif COLOUR_HEX.search(strip_comments(raw)):
            violations.add(path, lineno, "colour-literal",
                           "hex colours belong in tokens/palette.rs")

        for match in LENGTH_CALL.finditer(line):
            try:
                value = number(match.group(2))
            except ValueError:
                continue
            names = tokens.get(value)
            if value == 0.0 or not names:
                continue
            if allow.covers(rel, lineno, value):
                allowed += 1
                continue
            violations.add(path, lineno, "length-token",
                           f"{match.group(1)}({match.group(2)}) equals {', '.join(names)}: "
                           f"use the token, or if iOS writes this literal add a row to {ALLOW_FILE}")

        match = BOX_STEP.search(line)
        if match and match.group(1) != "0":
            violations.add(path, lineno, "baked-scale",
                           f"the _{match.group(1)}() helper bakes tailwind's scale, not ours")
        match = CORNER_STEP.search(line) or TEXT_STEP.search(line)
        if match:
            violations.add(path, lineno, "baked-scale",
                           f"_{match.group(1)}() is a tailwind size step; ask the theme instead")
        match = BORDER_STEP.search(line)
        if match and match.group(1) not in ("0", "1"):
            violations.add(path, lineno, "baked-scale",
                           f"border width {match.group(1)} is a design decision; "
                           "only the 1px hairline is structural")

    return allowed


def check() -> int:
    violations = Violations()
    tokens = token_values()
    if not tokens:
        violations.warnings.append("no token values found in TOKEN_SOURCES; length literals are unchecked")
    allow = Allow(violations)
    allowed = 0
    for path in sources():
        rel = path.relative_to(ROOT).as_posix()
        if is_exempt(rel):
            continue
        allowed += scan(path, violations, tokens, allow)
    allow.report_unused(violations)
    return violations.report(allowed)


if __name__ == "__main__":
    if not (ROOT / "crates").is_dir():
        print(f"no crates directory at {ROOT / 'crates'}", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(check())
