#!/usr/bin/env bash
# One prebuilt libtest case per process, with host CPU/memory/time bounds.
# These bounds are NOT a GPU-memory limit or protection from driver failure.
# --gpu-approved acknowledges approval obtained separately from the owner; it
# does not grant it. Wrap sound-emitting tests in scripts/quiet.sh.
#
# bash scripts/run-gpu-test-isolated.sh --gpu-approved <test-binary> <exact-name>
#
# This deliberately accepts just one case. Inspect its result and system health
# before another invocation. Never wrap it in an unattended full-suite loop.
set -euo pipefail

if [[ $# != 3 || ${1:-} != --gpu-approved ]]; then
	printf '%s\n' 'GPU tests need owner approval, a prebuilt test binary and one exact test name.' >&2
	exit 2
fi
binary=$(realpath -- "$2")
test_name=$3
if [[ ! -f $binary || ! -x $binary || ! $test_name =~ ^[a-zA-Z_][a-zA-Z_0-9:]*$ ]]; then
	printf '%s\n' 'Expected an executable test binary and one Rust test name.' >&2
	exit 2
fi

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$root/scratch"
artifact=$(mktemp -d "$root/scratch/isolated-gpu-test.XXXXXXXX")
printf 'Test evidence: %s\n' "$artifact"
printf '%s\n' "$binary" "$test_name" >"$artifact/selection.txt"
sha256sum -- "$binary" >"$artifact/binary.sha256"

# No fallback to an unbounded process if the user manager or controls fail.
# Listing uses libtest's no-test-execution route under the same bounds.
bounded() {
	systemd-run --user --scope \
		--property=MemoryMax=4G --property=MemorySwapMax=0 \
		--property=CPUQuota=100% --property=TasksMax=128 \
		--property=RuntimeMaxSec=125s --property=TimeoutStopSec=5s \
		--property=KillMode=control-group --property=OOMPolicy=kill \
		prlimit --fsize=67108864:67108864 -- \
		nice -n 10 timeout --signal=TERM --kill-after=5s 120s \
		env KJERAG_REQUIRE_GPU=1 RUST_TEST_THREADS=1 "$binary" "$@"
}
bounded --list --format terse --exact "$test_name" \
	>"$artifact/list.log" 2>"$artifact/list.stderr"
if [[ $(<"$artifact/list.log") != "$test_name: test" ]]; then
	printf '%s\n' 'The binary did not list exactly the requested test. Nothing was run.' >&2
	exit 2
fi
sha256sum --check "$artifact/binary.sha256" >"$artifact/binary-before.log" 2>&1

status=0
bounded --exact "$test_name" --nocapture --test-threads=1 \
	>"$artifact/test.log" 2>&1 || status=$?
printf '%s\n' "$status" >"$artifact/test.exit"
sha256sum --check "$artifact/binary.sha256" >"$artifact/binary-after.log" 2>&1
if (( status != 0 )); then
	printf 'Test process failed with exit %s. Stop and inspect %s/test.log\n' "$status" "$artifact" >&2
	exit "$status"
fi
# A typo, ignored case or harness mismatch must not produce a false pass.
if ! LC_ALL=C grep -Eq \
	'^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured;' "$artifact/test.log"; then
	printf 'No one-test pass was reported. Inspect %s/test.log\n' "$artifact" >&2
	exit 2
fi
printf 'One test passed in its own process. Evidence: %s\n' "$artifact"
