#!/usr/bin/env bash
set -euo pipefail

dist=${1:?usage: publish.sh <dist dir>}
: "${ENVIRONMENT:?}" "${VERSION:?}" "${BUILD_NUMBER:?}" "${TAG:?}" "${SHA:?}"
: "${DOWNLOADS_BASE:?set the DESKTOP_DOWNLOADS_BASE variable}"
: "${BUNNY_STORAGE_ZONE:?set the BUNNY_STORAGE_ZONE variable}" "${BUNNY_STORAGE_KEY:?set the BUNNY_STORAGE_KEY secret}"
host=${BUNNY_STORAGE_HOST:-storage.bunnycdn.com}
prefix=${STORAGE_PREFIX:-desktop}
folder="$ENVIRONMENT/$VERSION-$BUILD_NUMBER"
feed_url="${DOWNLOADS_BASE%/}/$ENVIRONMENT/feed.json"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mapfile -t files < <(find "$dist" -type f \( -name '*.zip' -o -name '*.dmg' -o -name '*.AppImage' \) | sort)
[ "${#files[@]}" -gt 0 ] || { echo "nothing to publish under $dist" >&2; exit 1; }
# why: platforms of one version publish one by one into the same folder; only a file name taken already moves us aside
kept=$(curl -fsS -H "AccessKey: $BUNNY_STORAGE_KEY" "https://$host/$BUNNY_STORAGE_ZONE/$prefix/$folder/" || true)
if printf '%s' "$kept" | python3 -c '
import json, os, sys
try:
    taken = {entry.get("ObjectName") for entry in json.load(sys.stdin)}
except ValueError:
    taken = set()
sys.exit(0 if taken & {os.path.basename(f) for f in sys.argv[1:]} else 1)
' "${files[@]}"; then
    folder="$folder.${GITHUB_RUN_NUMBER:-0}-${GITHUB_RUN_ATTEMPT:-1}"
fi

upload() {
    local file=$1 path=$2 type=$3 checksum
    checksum=$(sha256sum "$file" | awk '{ print toupper($1) }')
    curl -fsS --retry 3 --retry-all-errors -X PUT \
        -H "AccessKey: $BUNNY_STORAGE_KEY" -H "Checksum: $checksum" -H "Content-Type: $type" \
        --upload-file "$file" "https://$host/$BUNNY_STORAGE_ZONE/$prefix/$path" >/dev/null
    echo "uploaded $prefix/$path"
}

base_url="${DOWNLOADS_BASE%/}/$folder"
for file in "${files[@]}"; do
    upload "$file" "$folder/$(basename "$file")" application/octet-stream
done

stored() {
    curl -sS --retry 2 -o "$2" -w '%{http_code}' -H "AccessKey: $BUNNY_STORAGE_KEY" \
        "https://$host/$BUNNY_STORAGE_ZONE/$prefix/$1"
}
mine() {
    VERSION="$VERSION" python3 - "$1" "${files[@]}" <<'PY'
import json, os, sys
feed = json.load(open(sys.argv[1]))
release = next((r for r in feed.get("releases", []) if r.get("version") == os.environ["VERSION"]), {})
urls = {entry["url"].rsplit("/", 1)[-1] for kinds in release.get("downloads", {}).values() for entry in kinds.values()}
sys.exit(0 if {os.path.basename(f) for f in sys.argv[2:]} <= urls else 1)
PY
}

# why: other platforms of this version publish at the same time; storage has no compare-and-swap,
# so read the stored feed (not the cdn copy), write, then check our files survived and merge again if not
round=1
while :; do
code=$(stored "$ENVIRONMENT/feed.json" "$work/current.json")
case "$code" in
    200) echo "merging into the current feed" ;;
    404) : > "$work/current.json"; echo "no feed yet, starting one" ;;
    *) echo "reading $ENVIRONMENT/feed.json failed with http $code" >&2; exit 1 ;;
esac
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
sleep $((5 + RANDOM % 10))
[ "$(stored "$ENVIRONMENT/feed.json" "$work/check.json")" = 200 ] && mine "$work/check.json" && break
[ "$round" -lt 6 ] || { echo "::error::another publish keeps overwriting $ENVIRONMENT/feed.json" >&2; exit 1; }
echo "::warning::another platform wrote the feed at the same time, merging again"
round=$((round + 1))
done

latest_url="${DOWNLOADS_BASE%/}/$ENVIRONMENT/latest"
index_url="${DOWNLOADS_BASE%/}/$ENVIRONMENT/index.html"
code=$(curl -sS --retry 2 -o "$work/releases-current.json" -w '%{http_code}' -H "AccessKey: $BUNNY_STORAGE_KEY" \
    "https://$host/$BUNNY_STORAGE_ZONE/$prefix/$ENVIRONMENT/releases.json")
case "$code" in
    200) echo "merging into the release history" ;;
    404) : > "$work/releases-current.json"; echo "no release history yet, starting one" ;;
    *) echo "reading $ENVIRONMENT/releases.json failed with http $code" >&2; exit 1 ;;
esac
ENVIRONMENT="$ENVIRONMENT" SLUG="$SLUG" VERSION="$VERSION" LATEST_URL="$latest_url" \
    python3 "$here/releases.py" "$work/check.json" "$work/releases-current.json" \
    "$work/releases.json" "$work/index.html" "$work/latest-copies.tsv"
purge() {
    [ -n "${BUNNY_API_KEY:-}" ] || return 0
    curl -fsS -X POST -H "AccessKey: $BUNNY_API_KEY" \
        "https://api.bunny.net/purge?async=false&url=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1")" >/dev/null &&
        echo "purged $1" || echo "::warning::cdn purge failed for $1"
}

served=()
for file in "${files[@]}"; do served+=("$file"$'\t'"$base_url/$(basename "$file")"); done
while IFS=$'\t' read -r name latest; do
    source_file=$(printf '%s\n' "${files[@]}" | awk -F/ -v n="$name" '$NF == n' | head -n 1)
    [ -n "$source_file" ] || continue
    upload "$source_file" "$ENVIRONMENT/latest/$latest" application/octet-stream
    served+=("$source_file"$'\t'"$latest_url/$latest")
done < "$work/latest-copies.tsv"
upload "$work/releases.json" "$ENVIRONMENT/releases.json" application/json
upload "$work/index.html" "$ENVIRONMENT/index.html" "text/html; charset=utf-8"

for url in "$feed_url" "$feed_url.sig" "${DOWNLOADS_BASE%/}/$ENVIRONMENT/releases.json" "$index_url" \
    "$base_url/*" "$latest_url/*"; do
    purge "$url"
done

for pair in "${served[@]}"; do
    file=${pair%%$'\t'*}
    url=${pair#*$'\t'}
    want=$(sha256sum "$file" | awk '{ print $1 }')
    attempt=1
    until [ "$(curl -fsS --retry 2 "$url" | sha256sum | awk '{ print $1 }')" = "$want" ]; do
        [ "$attempt" -lt 5 ] || { echo "::error::the cdn serves another file at $url" >&2; exit 1; }
        echo "::warning::$url is not the uploaded file, purge $attempt of 4"
        attempt=$((attempt + 1))
        purge "$url"
        purge "${url%/*}/*"
        sleep 20
    done
    echo "the cdn serves $url"
done

if [ "$ENVIRONMENT" = production ]; then
    notes="$work/notes.md"
    echo "$SLUG $VERSION, build $BUILD_NUMBER, $ENVIRONMENT" > "$notes"
    gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1 ||
        gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --target "$SHA" \
            --title "$SLUG $VERSION" --notes-file "$notes" ||
        gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null
    gh release upload "$TAG" --repo "$GITHUB_REPOSITORY" --clobber "${files[@]}"
    {
        echo
        gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json assets \
            --jq '.assets | sort_by(.name) | .[] | "- [\(.name)](\(.url))"'
    } >> "$notes"
    gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" --notes-file "$notes" >/dev/null
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

if [ "$ENVIRONMENT" = production ]; then
    {
        echo "### $SLUG $VERSION ($BUILD_NUMBER), $ENVIRONMENT"
        echo
        echo "all versions: $index_url"
        for file in "${files[@]}"; do echo "- $base_url/$(basename "$file")"; done
    } >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
fi
