#!/usr/bin/env bash
# Native playback regression for the controls/header rebuilding the video away.
# Run after building kjerag and pointer in this checkout. All input stays in a
# private Cage session; no keys or pointer events reach the owner's desktop.
# Usage: bash scripts/uitest-controls-wake.sh <file.insv> [time=... yaw=... ...]
# KJERAG_BIN and KJERAG_POINTER may name frozen same-checkout test binaries.
set -euo pipefail
ulimit -c 0

task_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ${1:-} == --client ]]; then
    shift
    task_dir=${1:?private evidence directory}
    shift
    # This branch is internal to the isolated compositor launched below.
    test "${XDG_RUNTIME_DIR:-}" = "$task_dir/runtime"
    test "${XDG_CONFIG_HOME:-}" = "$task_dir/config"
    test -n "${WAYLAND_DISPLAY:-}"
    "$task_root/scripts/quiet.sh" env \
        KJERAG_NATIVE_LIFECYCLE_PROBE=1 KJERAG_NATIVE_CAPACITY_PROBE=1 \
        "$KJERAG_BIN" "$@" > "$task_dir/play.log" 2>&1 &
    task_player=$!
    cleanup() {
        if kill -0 "$task_player" 2>/dev/null; then kill -TERM "$task_player"; fi
        wait "$task_player" || true
    }
    trap cleanup EXIT
    for task_attempt in {1..300}; do
        kill -0 "$task_player" 2>/dev/null || exit 3
        if rg -q '^play:' "$task_dir/play.log"; then break; fi
        sleep 0.1
    done
    rg -q '^play:' "$task_dir/play.log"
    sha256sum "/proc/$task_player/exe" > "$task_dir/running-binary.sha256"
    test "$(cut -d ' ' -f1 "$task_dir/running-binary.sha256")" = "$KJERAG_WAKE_SHA256"

    # Each wait exceeds the app's two-second autohide delay. Alternating
    # coordinates guarantees a new motion event even on an unchanged view.
    for task_x in 620 640 660; do
        sleep 3
        "$KJERAG_POINTER" 1280 720 "$task_x" 350 >> "$task_dir/pointer.log" 2>&1
    done
    sleep 2
    grim -o HEADLESS-1 "$task_dir/after.png"
    wtype -M ctrl -k q -m ctrl
    for task_attempt in {1..50}; do
        if ! kill -0 "$task_player" 2>/dev/null; then break; fi
        if (( task_attempt % 10 == 0 )); then wtype -M ctrl -k q -m ctrl; fi
        sleep 0.1
    done
    if kill -0 "$task_player" 2>/dev/null; then
        echo 'Player did not quit within five seconds' >&2
        exit 4
    fi
    wait "$task_player"
    trap - EXIT
    exit 0
fi

test "$#" -gt 0 || { echo 'Usage: uitest-controls-wake.sh <file.insv> [view arguments]' >&2; exit 2; }
for task_tool in cage wtype grim pactl rg python3 sha256sum; do
    command -v "$task_tool" >/dev/null || { echo "Missing test tool: $task_tool" >&2; exit 2; }
done
if pgrep -x kjerag >/dev/null; then
    echo 'Stop the running player before measuring controls wakeup' >&2
    exit 2
fi
export KJERAG_BIN=${KJERAG_BIN:-$task_root/target/release/kjerag}
export KJERAG_POINTER=${KJERAG_POINTER:-$task_root/target/release/pointer}
test -x "$KJERAG_BIN" && test -x "$KJERAG_POINTER"
KJERAG_WAKE_SHA256=$(sha256sum "$KJERAG_BIN" | cut -d ' ' -f1)
export KJERAG_WAKE_SHA256
mkdir -p "$task_root/scratch/controls-wake"
task_dir=$(mktemp -d "$task_root/scratch/controls-wake/run.XXXXXXXX")
mkdir -p "$task_dir/runtime" "$task_dir/home/.local/state/cosmic" \
    "$task_dir/config/cosmic" "$task_dir/cache" "$task_dir/data"
chmod 700 "$task_dir/runtime"
task_audio_runtime=${XDG_RUNTIME_DIR:?sound server runtime directory}
test -S "$task_audio_runtime/pipewire-0"
ln -s "$task_audio_runtime/pipewire-0" "$task_dir/runtime/pipewire-0"
sha256sum "$KJERAG_BIN" "$KJERAG_POINTER" > "$task_dir/binaries.sha256"
printf 'Controls-wake evidence: %s\n' "$task_dir"
env -u WAYLAND_DISPLAY -u DISPLAY \
    HOME="$task_dir/home" XDG_RUNTIME_DIR="$task_dir/runtime" \
    XDG_CONFIG_HOME="$task_dir/config" XDG_STATE_HOME="$task_dir/home/.local/state" \
    XDG_CACHE_HOME="$task_dir/cache" XDG_DATA_HOME="$task_dir/data" \
    PULSE_SERVER="${PULSE_SERVER:-unix:$task_audio_runtime/pulse/native}" \
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
    timeout --signal=TERM --kill-after=3s 90s cage -- \
    bash "$task_root/scripts/uitest-controls-wake.sh" --client "$task_dir" "$@" \
    > "$task_dir/compositor.log" 2>&1
task_status=0
python3 "$task_root/scripts/check-controls-wake.py" "$task_dir/play.log" \
    > "$task_dir/result.json" || task_status=$?
cat "$task_dir/result.json"
exit "$task_status"
