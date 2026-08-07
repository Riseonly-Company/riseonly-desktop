#!/usr/bin/env python3
"""Report how much of riseonly-ios has been ported, file by file.

The desktop tree mirrors the iOS tree: same folders, same file stems, with
kebab-case turned into snake_case and .swift turned into .rs. Anything the
reference has and we do not is unported; anything we have and the reference
does not is either a declared desktop-only addition or a mistake.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IOS = ROOT.parent / "riseonly-ios" / "Riseonly"
APP = ROOT / "app" / "src"

IOS_SKIP = {".claude", "build", ".build", "DerivedData", "Assets.xcassets", "ObjC"}
IOS_EXCLUDE_MODULES = {"share-extension"}
IOS_TOP_LOWERCASE = {"Config"}
# Core/, Theme/ and Utils/ became libraries under crates/, so they are not part
# of the app tree and are excluded from the app-tree parity score.
IOS_EXCLUDE_TOP = {"Core", "Theme", "Utils", "UI"}

# Files with no counterpart in riseonly-ios because the phone has no equivalent
# of the thing they do. Each one is a claim, not a waiver: if a reader can name
# the Swift file it should have been ported from, it does not belong here.
DESKTOP_ONLY = {
    "main.rs",
    "app/router/pane_layout.rs",
    # A phone has no window, so it has no rail, no column widths and no
    # storybook to stand a component in six ways at once.
    "app/shell/rail.rs",
    "app/storybook/storybook.rs",
    # Keyboard-first navigation. The phone has no modifier row, no Esc and no
    # back/forward chord, so MainTabs.swift has nothing these are a port of.
    "app/shell/shell_actions.rs",
    "app/shell/command_palette.rs",
    # A phone is launched once and shows one window; a desktop app is launched
    # constantly and hands the later launches to the first one.
    "app/shell/window_presenter.rs",
    "app/shell/handover.rs",
    # Universal Links reach the iOS app through the OS; here a link arrives as
    # an argv entry and something has to turn it into a screen.
    "app/router/deep_link_route.rs",
    # A phone has no pointer and no file manager: there is nothing to drop.
    "app/shell/file_drop.rs",
    # UIKit resolves bundle resources itself; here the asset root has to be
    # found, because a dev run and a .app lay it out differently.
    "core/assets.rs",
    # The reference expresses these through ~60 singletons and an
    # AccountStateResetCoordinator that tears them down in a specific order.
    # Here they are ownership, which has no file to be a port of.
    "core/account_scope.rs",
    "core/session_activity.rs",
    # SaiNavigationBridge.currentScreen() returns exactly one screen. A desktop
    # shows several at once, so this is a set — see PHASES.txt, Phase 6.
    "core/active_screens.rs",
    # RISE_API_URL / config/local.json / per-environment data directories. The
    # phone ships one backend per build and has no local override file.
    "core/config/app_environment.rs",
    "core/config/endpoints.rs",
    # The host half of the engine contract. rise-engine does not own the socket,
    # so reconnect, backoff, the handshake credential and the application-level
    # heartbeat live out here. The reference keeps the same split, but its half
    # is SaiWebSocketManager inside the Sai package rather than a file under
    # Riseonly/, so there is nothing in this tree to be a port of.
    "core/engine_bridge/",
    # Where the modules are assembled and the push router is wired. The
    # reference does this in RiseonlyApp.swift plus a registration list inside
    # event-interactions.swift; on desktop it has to happen before the first
    # window, so it is its own file.
    "app/composition.rs",
    # engine/ exists in these three modules and not in their Swift counterparts
    # for one reason: none of them migrated to RiseEngine in the reference. They
    # still call SaiFetch and SaiWs there, so porting them RiseEngine-only is a
    # redesign of their data layer — PHASES.txt Phase 7, "the budget nobody
    # expects" — and the domain, repository, presentation and RPC catalogue have
    # no Swift file to correspond to.
    "modules/auth/engine/",
    "modules/session/engine/",
    "modules/presence/engine/",
    # The reference splits a store into <domain>-actions / -services /
    # -interactions plus a -wire-types file. DTOs live beside their descriptors
    # here, because a store may not import rise_engine and a descriptor must, so
    # the wire-types file has no counterpart and the store files that remain do.
    "modules/auth/shared/",

    # Four Swift files that draw four hand-written animations become one value
    # per slide plus one renderer, because the animations did not come across.
    "modules/onboarding/pages/onboarding/slides/onboarding_slide.rs",
    # Rust keeps a unit test beside the code it tests. A file whose whole content
    # is `#[cfg(test)]` has no Swift counterpart by construction: the reference's
    # tests live outside Riseonly/ entirely.
    "modules/auth/engine/rise_auth_repository_tests.rs",
    "modules/auth/pages/sign/auth_step_flow_tests.rs",
    "modules/session/engine/rise_session_repository_tests.rs",
}


def to_rust_name(stem: str) -> str:
    stem = stem.replace("-", "_")
    stem = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", stem)
    stem = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", stem)
    return re.sub(r"_+", "_", stem).lower()


def ios_files() -> dict[str, Path]:
    out: dict[str, Path] = {}
    if not IOS.is_dir():
        return out
    for path in IOS.rglob("*.swift"):
        rel = path.relative_to(IOS)
        parts = list(rel.parts)
        if any(p in IOS_SKIP or p.endswith(".lproj") for p in parts):
            continue
        if parts[0] in IOS_EXCLUDE_TOP:
            continue
        if parts[0] == "modules" and len(parts) > 1 and parts[1] in IOS_EXCLUDE_MODULES:
            continue
        head = parts[0].lower() if parts[0] in IOS_TOP_LOWERCASE else parts[0]
        mapped = [head] + [to_rust_name(p) for p in parts[1:-1]] + [to_rust_name(rel.stem) + ".rs"]
        out["/".join(mapped)] = path
    return out


def desktop_files() -> set[str]:
    if not APP.is_dir():
        return set()
    return {
        p.relative_to(APP).as_posix()
        for p in APP.rglob("*.rs")
        if p.name not in ("lib.rs", "mod.rs")
    }


def area_of(key: str) -> str:
    parts = key.split("/")
    if parts[0] == "modules" and len(parts) > 1:
        return f"modules/{parts[1]}"
    return parts[0]


def main() -> int:
    reference = ios_files()
    if not reference:
        print(f"reference tree not found at {IOS}", file=sys.stderr)
        return 2

    ours = desktop_files()
    ported = {k for k in reference if k in ours}
    missing = sorted(set(reference) - ours)
    extra = sorted(
        f for f in ours - set(reference)
        if not any(f == d or f.startswith(d) for d in DESKTOP_ONLY)
    )

    by_area: dict[str, list[int]] = {}
    for key in reference:
        slot = by_area.setdefault(area_of(key), [0, 0])
        slot[1] += 1
        if key in ours:
            slot[0] += 1

    print(f"{'area':<30} {'ported':>7} {'total':>7}  progress")
    print("-" * 66)
    for area in sorted(by_area, key=lambda a: (-by_area[a][1], a)):
        done, total = by_area[area]
        pct = 100.0 * done / total if total else 0.0
        print(f"{area:<30} {done:>7} {total:>7}  {pct:5.1f}% {'#' * int(pct / 5)}")

    total = len(reference)
    pct = 100.0 * len(ported) / total if total else 0.0
    print("-" * 66)
    print(f"{'TOTAL':<30} {len(ported):>7} {total:>7}  {pct:5.1f}%")

    if "--list-missing" in sys.argv:
        print(f"\nunported ({len(missing)}), first 80:")
        for f in missing[:80]:
            print(f"  {f}")

    if extra:
        print(f"\n{len(extra)} file(s) with no iOS counterpart and not declared desktop-only:")
        for f in extra[:40]:
            print(f"  {f}")
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
