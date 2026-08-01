#!/usr/bin/env bash
#
# The version is one number in three places and a release is the moment they
# have to agree (issue #106): the workspace manifest, which is what
# `kjerag --version` prints, the metainfo's newest <release>, which is the
# changelog a software centre shows, and the git tag the release workflow
# builds from.
#
#   scripts/version-check.sh            the workspace and the metainfo agree
#   scripts/version-check.sh 0.1.0      ... and the tag names that version
#   scripts/version-check.sh --notes    print the newest release's notes
#
# The workspace manifest is the source of truth. Nothing here edits anything:
# a mismatch is reported and the caller decides, which for the release
# workflow means the tag build stops before it produces an artifact claiming a
# version nothing else says.
#
# A tag is the plain version, `0.1.0`, with no `v` in front of it (owner,
# 2026-08-01).
#
# It may carry a prerelease suffix, `<version>-<label>`, and that is not a
# loophole in the check: the version in front of the dash still has to be the
# workspace's. It is how the pipeline itself is exercised without spending the
# version number a real release will use, and the workflow marks such a
# release as a prerelease on GitHub.
#
# --notes is what the workflow puts in the GitHub Release body, so the
# changelog is written once, in the file Flathub will read, rather than typed
# a second time into a release form.
#
# Exit: 0 they agree, 1 they do not or the file could not be read.

set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

notes=0
tag=

for arg in "$@"; do
	case $arg in
	--notes) notes=1 ;;
	-*)
		printf 'version-check: unknown option %s\n' "$arg" >&2
		exit 1
		;;
	*) tag=$arg ;;
	esac
done

python3 - "$root/Cargo.toml" "$root/resources/dev.harding.Kjerag.metainfo.xml" \
	"$notes" "$tag" <<-'PY'
	import sys, tomllib
	import xml.etree.ElementTree as ET

	cargo, metainfo, notes, tag = sys.argv[1], sys.argv[2], sys.argv[3] == "1", sys.argv[4]


	def die(msg):
	    print(f"version-check: {msg}", file=sys.stderr)
	    sys.exit(1)


	version = tomllib.load(open(cargo, "rb"))["workspace"]["package"]["version"]

	releases = ET.parse(metainfo).getroot().find("releases")
	if releases is None or len(releases) == 0:
	    die(f"{metainfo} has no <releases> entry; the changelog is not optional")

	# AppStream orders releases newest first, so the first element is the one
	# being cut. appstreamcli validate is what enforces that ordering.
	newest = releases[0].get("version")
	if newest != version:
	    die(
	        f"the workspace is {version} and the metainfo's newest release is "
	        f"{newest}: bump one of them"
	    )

	if tag:
	    named, label = tag, ""
	    if named.startswith(f"{version}-"):
	        named, label = version, named[len(version) + 1 :]
	    if named != version:
	        die(
	            f"the tag {tag} names {named} and the workspace is {version}: "
	            "tag the commit that carries the version, or bump the version"
	        )
	    if label:
	        print(f"version-check: {tag} is a prerelease of {version}")
	    else:
	        print(f"version-check: {tag} is {version}")
	elif not notes:
	    print(f"version-check: the workspace and the metainfo are both {version}")

	if not notes:
	    sys.exit(0)

	description = releases[0].find("description")
	if description is None:
	    die(f"the {version} release in {metainfo} has no <description>")

	# The subset AppStream allows in a release description, rendered as the
	# markdown a GitHub Release body is read as. Anything else would be
	# invalid metainfo, so appstreamcli validate is the guard on this list.
	out = []
	for node in description:
	    if node.tag == "p":
	        out.append(" ".join((node.text or "").split()))
	    elif node.tag in ("ul", "ol"):
	        for item in node:
	            out.append(f"- {' '.join((item.text or '').split())}")
	    else:
	        die(f"the {version} release notes carry a <{node.tag}>, which is not rendered")
	    out.append("")

	print("\n".join(out).strip())
PY
