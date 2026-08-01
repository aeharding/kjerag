#!/usr/bin/env bash
#
# The project's old name is gone from this tree and stays gone (issue #75,
# owner: "doesn't exist in any files or filenames, folders, anything").
# Content and paths, case-insensitively, over everything git tracks.
#
#   scripts/name-check.sh
#
# Git history and the archived issues and pull requests keep their copies of
# it, which is what makes this a check on the tree rather than a rewrite of
# the record.
#
# The pattern is written `k[y]erag` so that this file does not match itself,
# which is the same trick AGENTS.md uses on `pkill -f [v]ite`. Written as the
# plain word it would be the one tracked file that always fails.
#
# Exit: 0 the name is gone, 1 it is back and the report says where.

set -uo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." || exit 1

pattern='k[y]erag'
found=0

# Paths first: a file named for the old name is a rename somebody did not
# finish, and it reads as a content hit on nothing.
paths=$(git ls-files | grep -i -- "$pattern")
if [ -n "$paths" ]; then
	found=1
	printf 'name-check: tracked paths carrying the old name:\n' >&2
	printf '  %s\n' $paths >&2
fi

# Content, binaries included: a PNG can carry it in a text chunk, and the
# rule the owner stated has no exception for file type.
content=$(git ls-files -z | xargs -0 grep -l -i -- "$pattern" 2>/dev/null)
if [ -n "$content" ]; then
	found=1
	printf 'name-check: tracked files carrying the old name:\n' >&2
	while IFS= read -r file; do
		printf '  %s (%s)\n' "$file" "$(grep -c -i -- "$pattern" "$file" 2>/dev/null)" >&2
	done <<<"$content"
fi

if [ "$found" = 1 ]; then
	printf 'name-check: the name is Kjerag; see docs/ROADMAP.md, issue #75\n' >&2
	exit 1
fi

printf 'name-check: the old name appears in no tracked path or file\n'
