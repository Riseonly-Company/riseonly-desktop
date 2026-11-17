#!/usr/bin/env bash
set -euo pipefail

label() {
    gh label create "$1" --repo "$GITHUB_REPOSITORY" --color "$2" --description "$3" >/dev/null 2>&1 || true
}

label build 0e8a16 "ci builds this pull request (added by ci into staging and main)"
label deploy d93f0b "after the merge the build is released"
label macos 5319e7 "build macOS for Apple silicon and Intel, no platform label means all three"
label macos-silicon 5319e7 "build macOS for Apple silicon only (with macos-intel: both)"
label macos-intel 5319e7 "build macOS for Intel only (with macos-silicon: both)"
label linux 1d76db "build Linux, no platform label means all three"
label win 0052cc "build Windows, no platform label means all three"
label tests fbca04 "also run cargo test and clippy"

case "$ACTION:$BASE" in
    opened:staging | opened:main | reopened:staging | reopened:main | ready_for_review:staging | ready_for_review:main)
        gh pr edit "$PR" --repo "$GITHUB_REPOSITORY" --add-label build >/dev/null
        echo "build label added"
        ;;
esac
