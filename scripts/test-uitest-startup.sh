#!/usr/bin/env bash
# Regression test for the command-line-view startup decision in uitest.sh.
#
# The production harness is intentionally not made sourceable for one check.
# Instead, this test extracts three complete, uniquely anchored function
# definitions from the requested source: said(), await(), and with_media().
# It then replaces every session/input boundary and stops at the startup
# verdict. This exercises the real decision while using no compositor, app,
# input device, GPU, or wall-clock sleep.
#
# Pass an archived uitest.sh to demonstrate the regression against an older
# source, for example:
#   scripts/test-uitest-startup.sh scratch/.../source/scripts/uitest.sh

set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source_script=${1:-$root/scripts/uitest.sh}

[ -f "$source_script" ] || {
	printf 'test-uitest-startup: no source at %s\n' "$source_script" >&2
	exit 2
}

mkdir -p "$root/scratch"
artifact=$(mktemp -d "$root/scratch/uitest-startup-test.XXXXXXXX")
extracted=$artifact/extracted.sh

extract_function() {
	local name=$1 count
	count=$(grep -Fxc "$name() {" "$source_script" || :)
	[ "$count" = 1 ] || {
		printf 'test-uitest-startup: expected one %s() anchor, found %s\n' \
			"$name" "$count" >&2
		exit 2
	}
	sed -n "/^${name}() {\$/,/^}\$/p" "$source_script"
}

{
	extract_function said
	extract_function await
	extract_function with_media
} >"$extracted"

# Fail closed if an anchored range ended early or ceased to be the harness
# route under test. The archived baseline is allowed not to contain the new
# goto wait: its delayed case must fail behaviorally below.
for marker in '^said() {$' '^await() {$' '^with_media() {$' "await '\^media:'" \
	'the command-line view reaches the real player'; do
	grep -q -- "$marker" "$extracted" || {
		printf 'test-uitest-startup: extracted route lacks marker %s\n' "$marker" >&2
		exit 2
	}
done

# shellcheck disable=SC1090
source "$extracted"
for function in said await with_media; do
	declare -F "$function" >/dev/null || {
		printf 'test-uitest-startup: extracted %s() is not a function\n' "$function" >&2
		exit 2
	}
done

READY=2
media=/fixture/paired.insv
play_args=(time=12.500 yaw=7.00 pitch=-3.00 fov=80.00 lock=1)
expected_view="$media ${play_args[*]}"
checks=0
failures=0

run_case() {
	local scenario=$1 expected_status=$2 expected_min_sleeps=$3 expected_max_sleeps=$4
	local case_dir=$artifact/$scenario
	mkdir "$case_dir"
	log=$case_dir/play.log
	printf '0\n' >"$case_dir/sleep-count"

	set +e
	(
		boot() {
			printf 'media: synthetic paired capture\n' >"$log"
			case $scenario in
			exact-immediate)
				printf 'goto:   %s\n' "$expected_view" >>"$log"
				;;
			media-then-delayed-exact | missing | dead-before-goto) ;;
			wrong-first-later-exact)
				printf '%s\n' \
					'goto:   /fixture/wrong.insv time=1.000 yaw=0.00 pitch=0.00 fov=80.00 lock=1' \
					"goto:   $expected_view" >>"$log"
				;;
			*) exit 98 ;;
			esac
		}
		alive() {
			[ "$scenario" != dead-before-goto ]
		}
		sleep() {
			local count
			count=$(<"$case_dir/sleep-count")
			count=$((count + 1))
			printf '%s\n' "$count" >"$case_dir/sleep-count"
			if [ "$scenario" = media-then-delayed-exact ] && [ "$count" = 1 ]; then
				printf 'goto:   %s\n' "$expected_view" >>"$log"
			fi
		}
		pass() {
			[ "$1" != 'the command-line view reaches the real player' ] || exit 10
		}
		fail() {
			[ "$1" != 'the command-line view reaches the real player' ] || exit 11
		}
		lost() {
			[ "$1" != 'the command-line view reaches the real player' ] || exit 42
		}
		teardown() { :; }
		await_paint() {
			printf 'test-uitest-startup: route passed the startup verdict\n' >&2
			exit 97
		}

		with_media
		exit 96
	) >"$case_dir/output.txt" 2>&1
	local status=$?
	set -e

	local sleeps
	sleeps=$(<"$case_dir/sleep-count")
	if [ "$status" != "$expected_status" ]; then
		printf 'not ok %s: status %s, expected %s (artifacts: %s)\n' \
			"$scenario" "$status" "$expected_status" "$case_dir" >&2
		failures=$((failures + 1))
	elif [ "$sleeps" -lt "$expected_min_sleeps" ] || \
		[ "$sleeps" -gt "$expected_max_sleeps" ]; then
		printf 'not ok %s: %s polls, expected %s..%s (artifacts: %s)\n' \
			"$scenario" "$sleeps" "$expected_min_sleeps" "$expected_max_sleeps" \
			"$case_dir" >&2
		failures=$((failures + 1))
	else
		printf 'ok %s\n' "$scenario"
	fi
	checks=$((checks + 1))
}

run_case exact-immediate 10 0 0
run_case media-then-delayed-exact 10 1 1
run_case wrong-first-later-exact 11 0 0
run_case missing 11 1 $((READY * 2 + 1))
run_case dead-before-goto 42 0 0

printf '%s startup checks, %s failed; artifacts: %s\n' "$checks" "$failures" "$artifact"
[ "$failures" = 0 ]
