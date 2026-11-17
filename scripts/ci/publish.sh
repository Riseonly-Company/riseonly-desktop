#!/usr/bin/env bash
set -euo pipefail

dist=${1:?usage: publish.sh <dist dir>}
: "${ENVIRONMENT:?}" "${VERSION:?}" "${BUILD_NUMBER:?}" "${TAG:?}" "${SHA:?}"
: "${DOWNLOADS_BASE:?set the DESKTOP_DOWNLOADS_BASE variable}"
: "${BUNNY_STORAGE_ZONE:?set the BUNNY_STORAGE_ZONE variable}" "${BUNNY_STORAGE_KEY:?set the BUNNY_STORAGE_KEY secret}"
host=${BUNNY_STORAGE_HOST:-storage.bunnycdn.com}
prefix=${STORAGE_PREFIX:-desktop}
folder="$ENVIRONMENT/$VERSION-$BUILD_NUMBER"
base_url="${DOWNLOADS_BASE%/}/$folder"
feed_url="${DOWNLOADS_BASE%/}/$ENVIRONMENT/feed.json"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mapfile -t files < <(find "$dist" -type f \( -name '*.zip' -o -name '*.dmg' -o -name '*.AppImage' \) | sort)
[ "${#files[@]}" -gt 0 ] || { echo "nothing to publish under $dist" >&2; exit 1; }

upload() {
    local file=$1 path=$2 type=$3 checksum
    checksum=$(sha256sum "$file" | awk '{ print toupper($1) }')
    curl -fsS --retry 3 --retry-all-errors -X PUT \
        -H "AccessKey: $BUNNY_STORAGE_KEY" -H "Checksum: $checksum" -H "Content-Type: $type" \
        --upload-file "$file" "https://$host/$BUNNY_STORAGE_ZONE/$prefix/$path" >/dev/null
    echo "uploaded $prefix/$path"
}

for file in "${files[@]}"; do
    upload "$file" "$folder/$(basename "$file")" application/octet-stream
done

if curl -fsS --retry 2 "$feed_url?build=$BUILD_NUMBER" -o "$work/current.json"; then
    echo "merging into the current feed"
else
    echo "no feed yet at $feed_url, starting one"
    : > "$work/current.json"
fi
VERSION="$VERSION" BUILD_NUMBER="$BUILD_NUMBER" DOWNLOADS_URL="$base_url" \
    python3 "$here/feed.py" "$dist" "$work/current.json" "$work/feed.json"

if [ -n "${FEED_SIGNING_KEY:-}" ]; then
    (umask 077 && printf '%s\n' "$FEED_SIGNING_KEY" > "$work/feed.key")
    openssl pkeyutl -sign -inkey "$work/feed.key" -rawin -in "$work/feed.json" -out "$work/feed.sig.bin"
    base64 -w0 "$work/feed.sig.bin" > "$work/feed.json.sig"
    echo "feed public key (goes into the app): $(openssl pkey -in "$work/feed.key" -pubout -outform DER | tail -c 32 | base64 -w0)"
    upload "$work/feed.json.sig" "$ENVIRONMENT/feed.json.sig" text/plain
else
    echo "::warning::FEED_SIGNING_KEY is not set, production builds of the app refuse an unsigned feed"
fi
upload "$work/feed.json" "$ENVIRONMENT/feed.json" application/json

if [ -n "${BUNNY_API_KEY:-}" ]; then
    for url in "$feed_url" "$feed_url.sig"; do
        curl -fsS -X POST -H "AccessKey: $BUNNY_API_KEY" \
            "https://api.bunny.net/purge?async=false&url=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$url")" >/dev/null &&
            echo "purged $url" || echo "::warning::cdn purge failed for $url"
    done
fi

notes="$work/notes.md"
{
    echo "$SLUG $VERSION, build $BUILD_NUMBER, $ENVIRONMENT"
    echo
    echo "feed: $feed_url"
    for file in "${files[@]}"; do echo "- $base_url/$(basename "$file")"; done
} > "$notes"
if [ "$ENVIRONMENT" = production ]; then
    gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --target "$SHA" \
        --title "$SLUG $VERSION" --notes-file "$notes" "${files[@]}"
else
    gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --target "$SHA" --draft --prerelease \
        --title "$SLUG $VERSION ($BUILD_NUMBER)" --notes-file "$notes" "${files[@]}"
fi

if [ -n "${BACKEND_RELEASE_HOOK:-}" ]; then
    FEED_URL="$feed_url" python3 - "$work/feed.json" "$VERSION" > "$work/hook.json" <<'PY'
import json, os, sys
feed = json.load(open(sys.argv[1]))
release = next(r for r in feed["releases"] if r["version"] == sys.argv[2])
print(json.dumps({
    "app": "desktop",
    "environment": os.environ["ENVIRONMENT"],
    "version": release["version"],
    "build": release["build"],
    "channel": release["channel"],
    "feed_url": os.environ["FEED_URL"],
    "artifacts": release["artifacts"],
    "downloads": release["downloads"],
}))
PY
    auth=()
    [ -n "${BACKEND_RELEASE_TOKEN:-}" ] && auth=(-H "Authorization: Bearer $BACKEND_RELEASE_TOKEN")
    if curl -fsS --retry 2 -X POST -H "Content-Type: application/json" "${auth[@]}" \
        --data @"$work/hook.json" "$BACKEND_RELEASE_HOOK" >/dev/null; then
        echo "backend notified"
    else
        echo "::warning::the backend release hook failed, the feed is published anyway"
    fi
fi

{
    echo "### $SLUG $VERSION ($BUILD_NUMBER), $ENVIRONMENT"
    echo
    echo "feed: $feed_url"
    for file in "${files[@]}"; do echo "- $base_url/$(basename "$file")"; done
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
