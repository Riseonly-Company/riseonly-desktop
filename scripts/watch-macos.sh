#!/usr/bin/env bash
set -uo pipefail

usage() {
    cat <<'EOF'
usage: scripts/watch-macos.sh [options] [-- app arguments]

Rebuilds the app on every saved change and restarts its windows, like a dev server.
A red build keeps the last green windows running and prints only the errors.

Windows
  (default)                      one visible window
  --drive NAME                   also a hidden window driven through files (drive.py --dir): HOME and
                                 drive dir are WATCH_DRIVE_ROOT/home-NAME and drive-NAME; repeat for more
  --headless                     no visible window, only the --drive ones
  --mute, --design               app arguments for the visible window, as in cargo make dev
  --no-run                       build and bundle, never start a window

Build
  --env dev|staging|production   backend compiled into the build (default dev: the local stand 127.0.0.1:8080)
  --check                        compiler check only: no build, no window
  --profile NAME                 cargo profile (default watch; devopt runs faster, builds slower)
  --linux, --windows             after each green build also check those branches (OrbStack, cargo-xwin)
  --warnings                     print warnings, not only their count
  --once                         one build and bundle, then exit (warms the cache)
  --adhoc                        ad-hoc signature (Keychain and camera prompts return after each build)

From another shell, scripts/watch-wait.py blocks until the change saved before it ran is built:
exit 0 when the windows run it, 1 with the errors when it is red.

Files in <target>/<profile>/: watch-status.json, watch-errors.txt, watch-app.log (visible window).
Environment: CARGO_TARGET_DIR (default <repo>/target), WATCH_QUIET seconds without changes before
a build (default 2), WATCH_DRIVE_ROOT (default <target>/watch-drive), RUST_LOG for the windows
(default RISE_DEV_LOG or info), RISE_SIGN_IDENTITY (default the first valid Apple Development
identity, so Keychain and privacy grants survive rebuilds).
EOF
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tools="$(cd "$root/.." && pwd)/riseonly-tools"
[ "$(uname -s)" = Darwin ] || { echo "watch-macos runs on macOS; Linux and Windows are checked with --linux and --windows" >&2; exit 2; }
command -v fswatch >/dev/null || { echo "fswatch is missing: brew install fswatch" >&2; exit 2; }

env_name=dev
profile=watch
mode=build
linux=0
windows=0
warnings=0
run=1
visible=1
once=0
app_args=()
drives=()
while [ $# -gt 0 ]; do
    case "$1" in
        --env) env_name=${2:?--env needs a value}; shift ;;
        --env=*) env_name=${1#--env=} ;;
        --check) mode=check ;;
        --profile) profile=${2:?--profile needs a value}; shift ;;
        --profile=*) profile=${1#--profile=} ;;
        --drive) drives+=("${2:?--drive needs a name}"); shift ;;
        --drive=*) drives+=("${1#--drive=}") ;;
        --headless) visible=0 ;;
        --linux) linux=1 ;;
        --windows) windows=1 ;;
        --warnings) warnings=1 ;;
        --no-run) run=0 ;;
        --once) once=1; run=0 ;;
        --adhoc) export RISE_SIGN_IDENTITY=- ;;
        --design | --mute) app_args+=("$1") ;;
        -h | --help) usage; exit 0 ;;
        --) shift; app_args+=("$@"); break ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done
for name in ${drives[@]+"${drives[@]}"}; do
    case "$name" in
        main | *[!A-Za-z0-9_-]*) echo "--drive name must be letters, digits, - or _ (and not main): $name" >&2; exit 2 ;;
    esac
done

if [ ${#app_args[@]} -gt 0 ]; then
    set -- "${app_args[@]}"
    . "$root/scripts/dev-args.sh"
    app_args=("$@")
fi

export RISE_ENV=$env_name
export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
export RUST_BACKTRACE=${RUST_BACKTRACE:-1}
export RUST_LOG=${RUST_LOG:-${RISE_DEV_LOG:-info}}
if [ -z "${RISE_SIGN_IDENTITY:-}" ]; then
    RISE_SIGN_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | grep -v CSSMERR \
        | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*$/\1/p' | head -n 1)"
fi
export RISE_SIGN_IDENTITY=${RISE_SIGN_IDENTITY:--}
quiet=${WATCH_QUIET:-2}
case "$profile" in
    dev) out_dir=debug ;;
    *) out_dir=$profile ;;
esac
state="$CARGO_TARGET_DIR/$out_dir"
droot=${WATCH_DRIVE_ROOT:-$CARGO_TARGET_DIR/watch-drive}
mkdir -p "$state"
status_file="$state/watch-status.json"
errors_file="$state/watch-errors.txt"
diag="$root/scripts/cargo-diagnostics.py"
diag_args=()
[ "$warnings" = 1 ] && diag_args+=(--warnings)

win_names=()
win_pids=()
win_logs=()
win_drives=()
[ "$visible" = 1 ] && { win_names+=(main); win_logs+=("$state/watch-app.log"); win_drives+=(""); win_pids+=(""); }
for name in ${drives[@]+"${drives[@]}"}; do
    win_names+=("$name")
    win_logs+=("$droot/$name.log")
    win_drives+=("$droot/drive-$name")
    win_pids+=("")
done

bundle=""
lin_pid=""
win_check_pid=""
fsw_pid=""
stop_requested=0
fifo=""
seq=0
phase=idle
started=0
finished=0
last_change=0

say() { printf '\033[1m[watch %s]\033[0m %s\n' "$(date +%H:%M:%S)" "$*"; }
now() { python3 -c 'import time; print("%.3f" % time.time())'; }
json() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }

write_status() {
    local tmp="$status_file.tmp" i pid sep=""
    {
        printf '{"watcher_pid": %s, "seq": %s, "state": "%s", "started": %s, "finished": %s, ' \
            "$$" "$seq" "$phase" "$started" "$finished"
        printf '"last_change": %s, "quiet": %s, "env": "%s", "profile": "%s", ' "$last_change" "$quiet" "$env_name" "$profile"
        printf '"bundle": "%s", "errors": "%s", "windows": [' "$(json "$bundle")" "$(json "$errors_file")"
        i=0
        while [ $i -lt ${#win_names[@]} ]; do
            pid=${win_pids[$i]}
            printf '%s{"name": "%s", "pid": %s, "drive_dir": "%s", "log": "%s"}' "$sep" "${win_names[$i]}" \
                "${pid:-null}" "$(json "${win_drives[$i]}")" "$(json "${win_logs[$i]}")"
            sep=", "
            i=$((i + 1))
        done
        printf ']}\n'
    } >"$tmp" && mv -f "$tmp" "$status_file"
}

kill_tree() {
    local child
    for child in $(pgrep -P "$1" 2>/dev/null); do
        kill_tree "$child"
    done
    kill -TERM "$1" 2>/dev/null
}

stop_os_checks() {
    [ -n "$lin_pid" ] && kill_tree "$lin_pid"
    [ -n "$win_check_pid" ] && kill_tree "$win_check_pid"
    lin_pid=""
    win_check_pid=""
}

os_check() {
    local label=$1 script=$2 dir=$3 start=$SECONDS codes
    CARGO_TARGET_DIR="$CARGO_TARGET_DIR/$dir" bash "$root/scripts/$script" "$root" ${4:+"$4"} \
        2>"$state/$dir.err" | python3 "$diag" --label="$label" ${diag_args[@]+"${diag_args[@]}"}
    codes=("${PIPESTATUS[@]}")
    case "${codes[0]}" in
        0) say "$label green in $((SECONDS - start))s" ;;
        2 | 3) say "$label check did not start: $(tail -n 1 "$state/$dir.err")" ;;
        *) say "$label RED in $((SECONDS - start))s" ;;
    esac
}

start_os_checks() {
    if [ "$linux" = 1 ]; then
        os_check Linux linux-check.sh watch-linux check &
        lin_pid=$!
    fi
    if [ "$windows" = 1 ]; then
        os_check Windows windows-check.sh watch-win &
        win_check_pid=$!
    fi
}

stop_windows() {
    local i=0 pid tries
    while [ $i -lt ${#win_names[@]} ]; do
        pid=${win_pids[$i]}
        if [ -n "$pid" ]; then
            kill -TERM "$pid" 2>/dev/null
            tries=0
            while kill -0 "$pid" 2>/dev/null && [ $tries -lt 10 ]; do
                sleep 0.1
                tries=$((tries + 1))
            done
            kill -KILL "$pid" 2>/dev/null
            wait "$pid" 2>/dev/null
            win_pids[$i]=""
        fi
        i=$((i + 1))
    done
}

start_windows() {
    local i=0 name exe="$bundle/Contents/MacOS/riseonly"
    [ ${#drives[@]} -gt 0 ] && mkdir -p "$droot"
    while [ $i -lt ${#win_names[@]} ]; do
        name=${win_names[$i]}
        : >"${win_logs[$i]}"
        if [ "$name" = main ]; then
            (cd "$state" && exec "$exe" ${app_args[@]+"${app_args[@]}"}) >>"${win_logs[$i]}" 2>&1 &
        else
            mkdir -p "$droot/home-$name" "$droot/drive-$name"
            rm -f "$droot/drive-$name/command.request" "$droot/drive-$name/command.done"
            (cd "$droot/home-$name" && exec env HOME="$droot/home-$name" RISE_DEV_DRIVE_DIR="$droot/drive-$name" \
                RISE_DEV_STEALTH=1 RISE_PERF_LOG="$droot/perf-$name.log" nice -n 10 "$exe" -riseMuteSound) \
                >>"${win_logs[$i]}" 2>&1 &
        fi
        win_pids[$i]=$!
        i=$((i + 1))
    done
}

report_exit() {
    local i=$1 code=$2
    if [ "$code" = 0 ]; then
        say "window ${win_names[$i]} closed; the next green build opens it again"
    else
        say "window ${win_names[$i]} exited with code $code; last lines of ${win_logs[$i]}:"
        tail -n 25 "${win_logs[$i]}"
    fi
}

watch_windows() {
    local i=0 pid code changed=0
    while [ $i -lt ${#win_names[@]} ]; do
        pid=${win_pids[$i]}
        if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" 2>/dev/null
            code=$?
            win_pids[$i]=""
            changed=1
            report_exit "$i" "$code"
        fi
        i=$((i + 1))
    done
    [ "$changed" = 1 ] && write_status
}

run_stoppable() {
    "$@" &
    local pid=$! rc
    while :; do
        wait "$pid"
        rc=$?
        if [ "$stop_requested" = 1 ]; then
            local kids kid
            kids=$(pgrep -P "$pid")
            { kill -KILL "$pid"; wait "$pid"; } 2>/dev/null
            for kid in $kids; do
                kill_tree "$kid"
            done
            return 130
        fi
        kill -0 "$pid" 2>/dev/null || return $rc
    done
}

cargo_run() {
    cargo "$@" --message-format=json-diagnostic-rendered-ansi 2>"$state/watch-cargo.err" \
        | python3 "$diag" ${diag_args[@]+"${diag_args[@]}"} | tee "$errors_file"
    local codes=("${PIPESTATUS[@]}")
    if [ "${codes[0]}" != 0 ] && [ "${codes[1]}" = 0 ]; then
        grep -v -E '^ +(Compiling|Checking|Building|Fresh|Blocking|Finished|Running)' "$state/watch-cargo.err" \
            | tail -n 30 | tee -a "$errors_file"
    fi
    return "${codes[0]}"
}

finish() {
    phase=$1
    finished=$(now)
    write_status
}

smoke_windows() {
    local i=0 pid code dead=0
    sleep 2
    while [ $i -lt ${#win_names[@]} ]; do
        pid=${win_pids[$i]}
        if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" 2>/dev/null
            code=$?
            win_pids[$i]=""
            report_exit "$i" "$code"
            {
                echo "window ${win_names[$i]} exited with code $code right after start; ${win_logs[$i]}:"
                tail -n 40 "${win_logs[$i]}"
            } >>"$errors_file"
            dead=1
        fi
        i=$((i + 1))
    done
    return $dead
}

cycle() {
    local start=$SECONDS built
    seq=$((seq + 1))
    started=$(now)
    phase=building
    write_status
    stop_os_checks
    if [ "$mode" = check ]; then
        say "checking..."
        if run_stoppable cargo_run check --profile "$profile" -p riseonly --features dev-drive; then
            say "GREEN check in $((SECONDS - start))s"
            finish green
            start_os_checks
            return 0
        fi
        say "RED check in $((SECONDS - start))s"
        finish red
        return 1
    fi
    say "building ($profile, $env_name)..."
    run_stoppable cargo_run build --profile "$profile" -p riseonly --features dev-drive
    local rc=$?
    [ "$stop_requested" = 1 ] && return 1
    if [ "$rc" != 0 ]; then
        if [ -n "$bundle" ] && [ "$run" = 1 ]; then
            say "RED in $((SECONDS - start))s; the windows keep the last green build"
        else
            say "RED in $((SECONDS - start))s"
        fi
        finish red
        return 1
    fi
    built=$((SECONDS - start))
    [ "$run" = 1 ] && stop_windows
    local line
    line="$(bash "$root/scripts/bundle-macos.sh" "$out_dir" 2>"$state/watch-bundle.err" | sed -n 's/^bundled: \(.*\)  (.*)$/\1/p')"
    if [ -z "$line" ]; then
        say "GREEN build in ${built}s, but the bundle failed (scripts/bundle-macos.sh $out_dir):"
        tail -n 20 "$state/watch-bundle.err" | tee -a "$errors_file"
        finish red
        return 1
    fi
    bundle=$line
    [ "$stop_requested" = 1 ] && return 1
    if [ "$run" = 1 ] && [ ${#win_names[@]} -gt 0 ]; then
        start_windows
        say "GREEN in ${built}s build + $((SECONDS - start - built))s bundle; windows ${win_names[*]} restarted"
        if ! smoke_windows; then
            finish crashed
            return 1
        fi
    else
        say "GREEN in ${built}s build + $((SECONDS - start - built))s bundle: $bundle"
    fi
    finish green
    start_os_checks
    return 0
}

cleanup() {
    trap - INT TERM EXIT
    stop_os_checks
    stop_windows
    [ -n "$fsw_pid" ] && kill "$fsw_pid" 2>/dev/null
    [ -n "$fifo" ] && rm -f "$fifo"
    phase=stopped
    write_status
    exit 0
}

if [ "$once" = 1 ]; then
    cycle
    code=$?
    wait
    exit $code
fi

# invariant: the INT and TERM handler only sets a flag; real work inside it can hang a pending read -t
trap 'stop_requested=1' INT TERM
trap cleanup EXIT

paths=("$root/app" "$root/crates" "$root/assets" "$root/Cargo.toml" "$root/Cargo.lock" "$root/vendor/gpui-6b2aa1c/crates")
[ -d "$tools/crates" ] && paths+=("$tools/crates")
fifo="$(mktemp -u "${TMPDIR:-/tmp}/rise-watch.XXXXXX")"
mkfifo "$fifo"
fswatch -r -o -l 0.3 -E -e '.*' -i '\.(rs|toml|lock|json|png|svg|ttf|otf|tgs|lottie|plist|in|diff)$' \
    "${paths[@]}" >"$fifo" &
fsw_pid=$!
exec 3<"$fifo"

say "watching app, crates, assets, gpui and riseonly-tools; Ctrl-C stops"
if [ "$run" = 1 ] && [ "$mode" = build ]; then
    [ "$visible" = 1 ] && say "visible window log: $state/watch-app.log"
    [ ${#drives[@]} -gt 0 ] && say "driven windows: ${drives[*]} (drive.py --dir $droot/drive-NAME)"
    [ "$RISE_SIGN_IDENTITY" = - ] && say "ad-hoc signature: Keychain and camera may ask again after each build"
    if [ "$env_name" = dev ]; then
        code="$(curl -s -o /dev/null -m 2 -w '%{http_code}' http://127.0.0.1:8080/ 2>/dev/null)"
        case "$code" in
            200 | 401 | 404) ;;
            *) say "the local stand does not answer on 127.0.0.1:8080; start it, or run with --env production" ;;
        esac
    fi
fi
cycle

pending=0
last=0
while :; do
    [ "$stop_requested" = 1 ] && cleanup
    if read -r -t 1 -u 3 _; then
        pending=1
        last=$SECONDS
        last_change=$(now)
        write_status
        continue
    elif [ $? -le 128 ]; then
        say "file watcher stopped"
        exit 1
    fi
    watch_windows
    [ "$stop_requested" = 1 ] && cleanup
    if [ "$pending" = 1 ] && [ $((SECONDS - last)) -ge "$quiet" ]; then
        pending=0
        cycle
    fi
done
