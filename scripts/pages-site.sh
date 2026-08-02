#!/usr/bin/env bash
#
# Everything about the Flatpak repository that a person reads: the identity
# in its summary, the two descriptor files, the landing page and the icon.
#
#   scripts/pages-site.sh <repo-dir> <gpg-fingerprint>
#
# Two callers, one script, so a release and a republish say the same things:
# the pages job of .github/workflows/release.yml, against the repository
# flatter just built, and .github/workflows/site.yml, against the published
# one checked out of the Pages branch.
#
# There are two moments and they read different files. BEFORE the remote is
# trusted, a software centre has the `.flatpakref` and nothing else: no
# appstream branch, no metainfo, no icon. AFTER, it has the repository's
# summary for the source and the appstream branch for the app. The owner met
# both halves being empty on 2026-08-01: a placeholder icon, no summary and
# "Kjerag Developers" (COSMIC Store's own fallback string) on the ref, and
# `kjerag-origin` as the source once installed.
#
# So the display keys flatpakref(5) and flatpakrepo(5) document get filled in
# here, out of the metainfo, which is the file that already has to be right.
# The summary is written once and three files say it.
#
# The descriptors are generated rather than committed because they carry the
# public half of the signing key: rotate the secret and the next run rewrites
# them, rather than leaving a file in the tree naming a key nobody signs with.

set -eu

repo=${1:?usage: pages-site.sh <repo-dir> <gpg-fingerprint>}
fingerprint=${2:?usage: pages-site.sh <repo-dir> <gpg-fingerprint>}

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
metainfo="$root/resources/dev.harding.Kjerag.metainfo.xml"
manifest="$root/flatpak/dev.harding.Kjerag.yml"

# The metainfo is read with awk and not an XML parser because this runs inside
# flatter's build image, where a shell is what is promised. Both fields are
# one element each in a file this repository writes and CI validates
# (appstreamcli, ci.yml), and `exit` on the first match is what keeps the
# developer's <name> and the release notes' <p> out of it.
field() { awk "/<$1>/ { sub(/.*<$1>/, \"\"); sub(/<\\/$1>.*/, \"\"); print; exit }" "$metainfo"; }

name=$(field name)
summary=$(field summary)
description=$(awk '
	/<description>/ { in_description = 1 }
	in_description && /<p>/ { in_paragraph = 1 }
	in_paragraph { text = text " " $0 }
	in_paragraph && /<\/p>/ { exit }
	END {
		gsub(/<[^>]*>/, "", text)
		gsub(/[ \t]+/, " ", text)
		sub(/^ /, "", text)
		sub(/ $/, "", text)
		print text
	}
' "$metainfo")

# The app ID and the branch come from the manifest, which is what actually
# built the thing being described. The branch is also the ref file's name:
# it is the channel, the domain already says which app (owner, 2026-08-01).
app_id=$(awk '/^app-id:/ { print $2; exit }' "$manifest")
branch=$(awk '/^branch:/ { print $2; exit }' "$manifest")

url="https://$(cat "$root/flatpak/pages/CNAME")/"
icon="${url}icon.png"
key=$(gpg --export "$fingerprint" | base64 -w0)

# The repository's own identity, which is what a Store shows as the source of
# an installed app. Without it the source is the local remote's name, and a
# Store makes that `kjerag-origin`: `flatpak_transaction_add_install_flatpakref`
# names the remote after SuggestRemoteName plus `-origin` and gives it no
# title, where the CLI's `flatpak install --from` uses the suggested name as
# it stands (both measured against scratch installations, 2026-08-01). A
# remote takes these from the summary on its next metadata refresh, so an
# already-installed copy reads them too.
#
# flatter has already run build-update-repo once, without any of this, and
# running it again is composition rather than a fight: it adds these to the
# repository config, re-signs the summary, and leaves the appstream branch
# byte-identical (measured against the published repository, 2026-08-01).
flatpak build-update-repo \
	--title "$name ($branch)" \
	--comment "$summary" \
	--description "$description" \
	--homepage "$url" \
	--icon "$icon" \
	--default-branch "$branch" \
	--gpg-sign="$fingerprint" \
	"$repo"

# CNAME, .nojekyll and the landing page. .nojekyll is load-bearing: a
# branch-source Pages site is a Jekyll build by default, and Jekyll drops
# paths beginning with an underscore, which an OSTree static delta name can
# (its checksums are base64 with `_` in the alphabet).
cp -r "$root/flatpak/pages/." "$repo"/
cp "$root/resources/icons/hicolor/256x256/apps/$app_id.png" "$repo/icon.png"

# The ref was named after the application ID until 2026-08-01. A republish
# deploys the published tree as it stands, so the old name has to go here or
# it outlives the rename.
rm -f "$repo/$app_id.flatpakref"

cat >"$repo/kjerag.flatpakrepo" <<EOF
[Flatpak Repo]
Title=$name ($branch)
Url=$url
Homepage=$url
Icon=$icon
Comment=$summary
Description=$description
DefaultBranch=$branch
GPGKey=$key
EOF

# One click installs the app and adds the remote it came from, so the next
# release arrives on its own. RuntimeRepo is what lets that work on a machine
# that has never had Flathub configured: the app is ours, the runtime under
# it is not.
cat >"$repo/$branch.flatpakref" <<EOF
[Flatpak Ref]
Title=$name
Name=$app_id
Branch=$branch
Url=$url
Homepage=$url
Icon=$icon
Comment=$summary
Description=$description
SuggestRemoteName=kjerag
GPGKey=$key
RuntimeRepo=https://flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
EOF

printf 'pages-site: %s, "%s (%s)", %s\n' "$app_id" "$name" "$branch" "$url"
