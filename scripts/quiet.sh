#!/usr/bin/env bash
# Run a sound-emitting command with its audio routed to a null sink, so
# instrument runs never reach the speakers. The stream still opens a real
# device path through pipewire, so timing and underrun accounting behave;
# only audibility changes. For a measurement whose PURPOSE is real-device
# latency, do not use this - zero the stream volume instead.
#
#   scripts/quiet.sh cargo run --release -p kjerag-spike --bin sync -- <file>
set -euo pipefail
if ! pactl list short sinks | grep -q '^[0-9]*\s*kjerag_quiet'; then
	pactl load-module module-null-sink sink_name=kjerag_quiet \
		sink_properties=device.description=kjerag-quiet >/dev/null
fi
export PULSE_SINK=kjerag_quiet
export PIPEWIRE_NODE=kjerag_quiet
exec "$@"
