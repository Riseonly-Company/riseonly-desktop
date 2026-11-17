#!/usr/bin/env bash
set -euo pipefail

label() {
    gh label create "$1" --repo "$GITHUB_REPOSITORY" --color "$2" --description "$3" >/dev/null 2>&1 || true
}

label build 0e8a16 "ci builds this pull request (added by ci into staging and main); with no platform label: Linux x64, Windows x64, both macOS"
label deploy d93f0b "after the merge the build is released"
label macos 5319e7 "build macOS for Apple silicon and Intel"
label macos-silicon 5319e7 "build macOS for Apple silicon (arm64)"
label macos-intel 5319e7 "build macOS for Intel (x86_64)"
label linux 1d76db "build Linux for x86_64 and arm64"
label linux-x64 1d76db "build Linux for x86_64 (AppImage)"
label linux-arm64 1d76db "build Linux for arm64 (AppImage, GitHub hosted arm runner)"
label win 0052cc "build Windows for x86_64 and arm64"
label win-x64 0052cc "build Windows for x86_64 (zip)"
label win-arm64 0052cc "build Windows for arm64 (zip)"
label env-dev c5def5 "build against the dev backend instead of the branch default"
label env-staging c5def5 "build against the staging backend instead of the branch default"
label env-prod c5def5 "build against the production backend instead of the branch default"
label release-profile f9d0c4 "build with the release profile (lto) instead of fast"
label tests fbca04 "also run cargo test and clippy"

case "$ACTION:$BASE" in
    opened:staging | opened:main | reopened:staging | reopened:main | ready_for_review:staging | ready_for_review:main)
        gh pr edit "$PR" --repo "$GITHUB_REPOSITORY" --add-label build >/dev/null
        echo "build label added"
        ;;
esac
