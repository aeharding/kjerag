#!/usr/bin/env bash
# Playback regression for the controls/header rebuilding the video away.
# Run after building kjerag and pointer in this checkout, or set
# KJERAG_FLATPAK to test the actual installed bundle. All input stays in a
# private Cage session; no keys or pointer events reach the owner's desktop.
# Usage: bash scripts/uitest-controls-wake.sh <file.insv> [time=... yaw=... ...]
# KJERAG_BIN and KJERAG_POINTER may name frozen same-checkout test binaries.
# KJERAG_FLATPAK=dev.harding.Kjerag selects the installed Flatpak instead.
set -euo pipefail
ulimit -c 0

task_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ${1:-} == --client ]]; then
    shift
    task_dir=${1:?private evidence directory}
    shift
    # This branch is internal to the isolated compositor launched below.
    test "${XDG_RUNTIME_DIR:-}" = "$KJERAG_WAKE_RUNTIME"
    test "${XDG_CONFIG_HOME:-}" = "$task_dir/config"
    test -n "${WAYLAND_DISPLAY:-}"
    if [[ -n ${KJERAG_FLATPAK:-} ]]; then
        task_media=${1:?media file}
        exec {task_instance_fd}>"$task_dir/flatpak-instance"
        task_flatpak=(env "FLATPAK_USER_DIR=$KJERAG_WAKE_FLATPAK_USER_DIR"
            flatpak run
            --die-with-parent
            "--instance-id-fd=$task_instance_fd"
            --env=KJERAG_NATIVE_LIFECYCLE_PROBE=1
            --env=KJERAG_NATIVE_CAPACITY_PROBE=1
            --env=PULSE_SINK=kjerag_quiet
            --env=PIPEWIRE_NODE=kjerag_quiet
            "--filesystem=$task_media:ro")
        task_mate=$(printf '%s' "$task_media" |
            sed -E 's/_00_([^_/]*)$/_10_\1/; t; s/_10_([^_/]*)$/_00_\1/')
        if [[ $task_mate != "$task_media" && -f $task_mate ]]; then
            task_flatpak+=("--filesystem=$task_mate:ro")
        fi
        # Keep the manifest's command and shipped permission set. Capture
        # paths are added read-only because the private HOME changes what the
        # manifest's xdg-videos grant resolves to.
        task_flatpak+=("$KJERAG_FLATPAK")
        "$task_root/scripts/quiet.sh" "${task_flatpak[@]}" "$@" \
            > "$task_dir/play.log" 2>&1 &
        exec {task_instance_fd}>&-
    else
        "$task_root/scripts/quiet.sh" env \
            KJERAG_NATIVE_LIFECYCLE_PROBE=1 KJERAG_NATIVE_CAPACITY_PROBE=1 \
            "$KJERAG_BIN" "$@" > "$task_dir/play.log" 2>&1 &
    fi
    task_player=$!
    cleanup() {
        if [[ -n ${KJERAG_FLATPAK:-} && -s $task_dir/flatpak-instance ]]; then
            task_cleanup_instance=$(<"$task_dir/flatpak-instance")
            task_cleanup_app=$(
                env "FLATPAK_USER_DIR=$KJERAG_WAKE_FLATPAK_USER_DIR" \
                    flatpak ps --columns=instance:f,application:f |
                    awk -v wanted="$task_cleanup_instance" \
                        '$1 == wanted { print $2 }'
            ) || task_cleanup_app=
            if [[ $task_cleanup_app == "$KJERAG_FLATPAK" ]]; then
                env "FLATPAK_USER_DIR=$KJERAG_WAKE_FLATPAK_USER_DIR" \
                    flatpak kill "$task_cleanup_instance" || true
            fi
        fi
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
    if [[ -n ${KJERAG_FLATPAK:-} ]]; then
        task_instance=$(<"$task_dir/flatpak-instance")
        env "FLATPAK_USER_DIR=$KJERAG_WAKE_FLATPAK_USER_DIR" \
            flatpak ps --columns=instance:f,child-pid:f,application:f,commit:f \
            > "$task_dir/running-flatpak.tsv"
        read -r task_sandbox_pid task_running_app task_running_commit <<<"$(
            awk -v wanted="$task_instance" \
                '$1 == wanted { print $2, $3, $4 }' "$task_dir/running-flatpak.tsv"
        )"
        test -n "$task_sandbox_pid"
        test "$task_running_app" = "$KJERAG_FLATPAK"
        # Flatpak's commit column contains a 12-character prefix even with
        # full column formatting. Bind that prefix to the full deployment
        # digest, then find the actual app beneath this sandbox only.
        [[ $task_running_commit =~ ^[0-9a-f]{12,64}$ ]]
        [[ $KJERAG_WAKE_COMMIT == "$task_running_commit"* ]]
        kill -0 "$task_sandbox_pid"

        # child-pid is Flatpak's wrapper on this host, not Kjerag. Walk only
        # its authenticated process tree and cap the walk so corrupt proc
        # state cannot turn identity checking into an unbounded host scan.
        task_process_limit=64
        task_processes=("$task_sandbox_pid")
        declare -A task_seen_processes=(["$task_sandbox_pid"]=1)
        for ((task_process_index = 0;
            task_process_index < ${#task_processes[@]};
            task_process_index++)); do
            task_process=${task_processes[task_process_index]}
            task_children=()
            read -r -a task_children \
                < "/proc/$task_process/task/$task_process/children" || true
            for task_child in "${task_children[@]}"; do
                [[ $task_child =~ ^[0-9]+$ ]] || continue
                [[ -z ${task_seen_processes[$task_child]:-} ]] || continue
                if (( ${#task_processes[@]} >= task_process_limit )); then
                    echo "Flatpak process tree exceeds $task_process_limit processes" >&2
                    exit 3
                fi
                task_seen_processes["$task_child"]=1
                task_processes+=("$task_child")
            done
        done

        task_app_processes=()
        for task_process in "${task_processes[@]:1}"; do
            read -r task_process_comm < "/proc/$task_process/comm" || continue
            [[ $task_process_comm == kjerag ]] || continue
            task_process_sha=$(sha256sum "/proc/$task_process/exe" 2>/dev/null |
                cut -d ' ' -f1) || continue
            if [[ $task_process_sha == "$KJERAG_WAKE_SHA256" ]]; then
                task_app_processes+=("$task_process")
            fi
        done
        if (( ${#task_app_processes[@]} != 1 )); then
            echo "Expected one authenticated Kjerag process, found ${#task_app_processes[@]}" >&2
            exit 3
        fi
        task_measure_pid=${task_app_processes[0]}
        sha256sum "/proc/$task_measure_pid/exe" \
            > "$task_dir/running-binary.sha256"
        test "$(cut -d ' ' -f1 "$task_dir/running-binary.sha256")" = \
            "$KJERAG_WAKE_SHA256"
        printf 'instance\tsandbox_pid\tapp_pid\tapplication\tcommit\texe_sha256\n' \
            > "$task_dir/process-identity.tsv"
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$task_instance" \
            "$task_sandbox_pid" "$task_measure_pid" "$task_running_app" \
            "$KJERAG_WAKE_COMMIT" "$KJERAG_WAKE_SHA256" \
            >> "$task_dir/process-identity.tsv"
    else
        task_measure_pid=$task_player
        sha256sum "/proc/$task_player/exe" > "$task_dir/running-binary.sha256"
        test "$(cut -d ' ' -f1 "$task_dir/running-binary.sha256")" = "$KJERAG_WAKE_SHA256"
    fi

    # Each wait exceeds the app's two-second autohide delay. Alternating
    # coordinates guarantees a new motion event even on an unchanged view.
    for task_x in 620 640 660; do
        sleep 3
        "$KJERAG_POINTER" 1280 720 "$task_x" 350 >> "$task_dir/pointer.log" 2>&1
    done
    sleep 2
    # Preserve memory accounting for this exact private player, not another
    # desktop process. DRM fdinfo may repeat a client; analysis must deduplicate
    # by drm-client-id before summing device allocations.
    cp "/proc/$task_measure_pid/status" "$task_dir/player-status.txt"
    for task_fdinfo in /proc/"$task_measure_pid"/fdinfo/*; do
        if rg -q '^drm-client-id:' "$task_fdinfo"; then
            cp "$task_fdinfo" "$task_dir/drm-fdinfo-${task_fdinfo##*/}.txt"
        fi
    done
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
if [[ -n ${KJERAG_FLATPAK:-} ]]; then
    command -v flatpak >/dev/null || { echo 'Missing test tool: flatpak' >&2; exit 2; }
    flatpak info "$KJERAG_FLATPAK" >/dev/null 2>&1 || {
        echo "No installed Flatpak for $KJERAG_FLATPAK" >&2
        exit 2
    }
fi
if pgrep -x kjerag >/dev/null; then
    echo 'Stop the running player before measuring controls wakeup' >&2
    exit 2
fi
export KJERAG_POINTER=${KJERAG_POINTER:-$task_root/target/release/pointer}
test -x "$KJERAG_POINTER"
task_args=("$@")
if [[ -n ${KJERAG_FLATPAK:-} ]]; then
    task_args[0]=$(realpath -- "${task_args[0]}")
    KJERAG_WAKE_FLATPAK_USER_DIR=${FLATPAK_USER_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/flatpak}
    export KJERAG_WAKE_FLATPAK_USER_DIR
else
    export KJERAG_BIN=${KJERAG_BIN:-$task_root/target/release/kjerag}
    test -x "$KJERAG_BIN"
    KJERAG_WAKE_SHA256=$(sha256sum "$KJERAG_BIN" | cut -d ' ' -f1)
fi
export KJERAG_WAKE_SHA256
mkdir -p "$task_root/scratch/controls-wake"
task_dir=$(mktemp -d "$task_root/scratch/controls-wake/run.XXXXXXXX")
mkdir -p "$task_dir/home/.local/state/cosmic" \
    "$task_dir/config/cosmic" "$task_dir/cache" "$task_dir/data"
task_audio_runtime=${XDG_RUNTIME_DIR:?sound server runtime directory}
task_runtime=$(mktemp -d "$task_audio_runtime/kjerag-controls-wake.XXXXXXXX")
chmod 700 "$task_runtime"
# The runtime contains compositor and proxy sockets only. Evidence, logs and
# private app state remain under task_dir; no important runtime file is kept.
cleanup_runtime() {
    rm -rf -- "$task_runtime"
}
trap cleanup_runtime EXIT
export KJERAG_WAKE_RUNTIME=$task_runtime
test -S "$task_audio_runtime/pipewire-0"
ln -s "$task_audio_runtime/pipewire-0" "$task_runtime/pipewire-0"
if [[ -n ${KJERAG_FLATPAK:-} ]]; then
    test -S "$task_audio_runtime/pulse/native"
    mkdir -p "$task_runtime/pulse"
    ln -s "$task_audio_runtime/pulse/native" "$task_runtime/pulse/native"
    flatpak info --show-commit "$KJERAG_FLATPAK" \
        > "$task_dir/installed-commit.before"
    KJERAG_WAKE_COMMIT=$(<"$task_dir/installed-commit.before")
    export KJERAG_WAKE_COMMIT
    flatpak info --show-location "$KJERAG_FLATPAK" \
        > "$task_dir/installed-location.before"
    task_flatpak_location=$(<"$task_dir/installed-location.before")
    task_flatpak_binary=$task_flatpak_location/files/bin/kjerag
    test -x "$task_flatpak_binary"
    sha256sum "$task_flatpak_binary" > "$task_dir/installed-binary.before.sha256"
    KJERAG_WAKE_SHA256=$(cut -d ' ' -f1 "$task_dir/installed-binary.before.sha256")
    export KJERAG_WAKE_SHA256
    sha256sum "$KJERAG_POINTER" > "$task_dir/binaries.sha256"
else
    sha256sum "$KJERAG_BIN" "$KJERAG_POINTER" > "$task_dir/binaries.sha256"
fi
printf 'Controls-wake evidence: %s\n' "$task_dir"
task_cage_status=0
env -u WAYLAND_DISPLAY -u DISPLAY \
    HOME="$task_dir/home" XDG_RUNTIME_DIR="$task_runtime" \
    XDG_CONFIG_HOME="$task_dir/config" XDG_STATE_HOME="$task_dir/home/.local/state" \
    XDG_CACHE_HOME="$task_dir/cache" XDG_DATA_HOME="$task_dir/data" \
    PULSE_SERVER="${PULSE_SERVER:-unix:$task_audio_runtime/pulse/native}" \
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
    timeout --signal=TERM --kill-after=3s 90s cage -- \
    bash "$task_root/scripts/uitest-controls-wake.sh" --client "$task_dir" \
    "${task_args[@]}" > "$task_dir/compositor.log" 2>&1 || task_cage_status=$?
if [[ -n ${KJERAG_FLATPAK:-} ]]; then
    task_flatpak_location_after=$(flatpak info --show-location "$KJERAG_FLATPAK")
    flatpak info --show-commit "$KJERAG_FLATPAK" \
        > "$task_dir/installed-commit.after"
    printf '%s\n' "$task_flatpak_location_after" \
        > "$task_dir/installed-location.after"
    sha256sum "$task_flatpak_location_after/files/bin/kjerag" \
        > "$task_dir/installed-binary.after.sha256"
    cmp -s "$task_dir/installed-commit.before" "$task_dir/installed-commit.after" || {
        echo 'Installed Flatpak commit changed during the controls-wake run' >&2
        exit 3
    }
    test "$(cut -d ' ' -f1 "$task_dir/installed-binary.before.sha256")" = \
        "$(cut -d ' ' -f1 "$task_dir/installed-binary.after.sha256")" || {
        echo 'Installed Flatpak executable changed during the controls-wake run' >&2
        exit 3
    }
fi
if (( task_cage_status != 0 )); then
    exit "$task_cage_status"
fi
task_status=0
python3 "$task_root/scripts/check-controls-wake.py" "$task_dir/play.log" \
    > "$task_dir/result.json" || task_status=$?
cat "$task_dir/result.json"
python3 - "$task_dir/result.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as source:
    result = json.load(source)
count = result["logs"][0]["wake_count"]
if count != 3:
    raise SystemExit(f"Expected three recorded pointer wakes, found {count}")
PY
exit "$task_status"
