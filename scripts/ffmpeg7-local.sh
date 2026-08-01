#!/usr/bin/env bash
#
# ffmpeg 7.1 into a directory of your own, with no sudo and nothing installed.
#
#   scripts/ffmpeg7-local.sh
#   eval "$(scripts/ffmpeg7-local.sh --env)"   # just print the exports
#
# The workspace pins ffmpeg-next 7.1 and Ubuntu 24.04 ships 6.1 (issue #65).
# The supported answer is the PPA in AGENTS.md, installed with apt. This is
# the other one: the same packages unpacked under $HOME, for a box that is
# not allowed to replace its system ffmpeg, or has not yet.
#
# Nothing here is a build of ffmpeg. The .debs are the PPA's own binaries,
# built for noble against noble's glibc, so they load on this distribution
# unchanged; a plucky or Debian deb would not. They are unpacked, not
# installed: dpkg's database is never touched and /usr is never written.
#
# The two runtimes coexist. Ubuntu's 6.1 is libavcodec60/libavutil58 and
# ffmpeg 7.1 is libavcodec61/libavutil59, different sonames in different
# files, so every program on the box keeps linking what it always did and
# only a build that is pointed here with PKG_CONFIG_PATH sees 7.1.
#
# Dependencies are resolved from the packages' own Depends fields rather
# than guessed from sonames: libaribb24.so.0 comes from `libaribb24-0t64`,
# which no rule that mangles a soname into a package name will produce.
# Anything dpkg reports as installed is left to the system copy.

set -uo pipefail

prefix=${KYERAG_FFMPEG7:-$HOME/.local/ffmpeg71-dev}
libdir=$prefix/usr/lib/x86_64-linux-gnu
ppa=https://ppa.launchpadcontent.net/ubuntuhandbook1/ffmpeg7/ubuntu
dist=noble

# The headers the pin needs. ffmpeg-next's default features bind all eight,
# which is what AGENTS.md's apt line installs; everything else this ends up
# fetching is something one of these eight asked for.
seed=(libavcodec-dev libavdevice-dev libavfilter-dev libavformat-dev
	libavutil-dev libpostproc-dev libswresample-dev libswscale-dev)

env_only=${1:-}
# RUSTFLAGS is not redundant with PKG_CONFIG_PATH. alsa-sys emits
# `-L /usr/lib/x86_64-linux-gnu` for libasound, and when that lands on the
# linker's command line before ffmpeg-sys-next's `-L` the `-lavcodec` there
# resolves to the system libavcodec-dev's 6.1 symlink. Nothing complains:
# ffmpeg 6.1 and 7.1 export the same names, so 7.1's bindgen struct layouts
# link cleanly against 6.1's libraries and the mismatch only exists at run
# time. A `-L` in RUSTFLAGS is searched first, which settles it. Changing
# RUSTFLAGS rebuilds the workspace, so export it once per shell.
#
# Whatever a build claims, `readelf -d <binary> | grep NEEDED` is the thing
# that knows: 7.1 is libavcodec.so.61 and libavutil.so.59.
exports() {
	printf 'export PKG_CONFIG_PATH=%s/pkgconfig\n' "$libdir"
	printf 'export RUSTFLAGS="-L native=%s"\n' "$libdir"
	printf 'export LD_LIBRARY_PATH=%s\n' "$libdir"
}
if [ "$env_only" = "--env" ]; then
	exports
	exit 0
fi

die() {
	printf 'ffmpeg7-local: %s\n' "$1" >&2
	exit 2
}

command -v dpkg-deb >/dev/null || die "needs dpkg-deb"
mkdir -p "$prefix/.debs" || die "cannot write $prefix"
cd "$prefix" || die "cannot enter $prefix"

# --------------------------------------------------------------- the index

index=$prefix/.debs/Packages
if [ ! -s "$index" ]; then
	printf 'fetching %s %s index\n' "$ppa" "$dist"
	curl -fsSL "$ppa/dists/$dist/main/binary-amd64/Packages.gz" |
		gunzip >"$index" || die "cannot read the PPA index"
fi

# Package name -> path within the PPA, for the names the PPA carries. Every
# other name is the system archive's, and `apt-get download` fetches those.
ppa_path() {
	awk -v want="$1" '
		/^Package: /  { pkg = $2 }
		/^Filename: / { if (pkg == want) { print $2; exit } }
	' "$index"
}

fetch() {
	local pkg=$1 path
	path=$(ppa_path "$pkg")
	if [ -n "$path" ]; then
		curl -fsSL -o ".debs/$pkg.deb" "$ppa/$path" && echo ".debs/$pkg.deb"
		return
	fi
	# Not the PPA's: the distribution's own copy, and only when the box does
	# not already have it unpacked in /usr.
	dpkg -s "$pkg" >/dev/null 2>&1 && return
	(cd .debs && apt-get download "$pkg" >/dev/null 2>&1) || return
	ls -t .debs/"${pkg}"_*.deb 2>/dev/null | head -1
}

# ------------------------------------------------------- fetch and unpack

declare -A seen=()
queue=("${seed[@]}")
while [ ${#queue[@]} -gt 0 ]; do
	pkg=${queue[0]}
	queue=("${queue[@]:1}")
	[ -n "${seen[$pkg]:-}" ] && continue
	seen[$pkg]=1

	deb=$(fetch "$pkg")
	[ -z "$deb" ] && continue
	printf '  %s\n' "$pkg"
	dpkg -x "$deb" . || die "cannot unpack $deb"

	# Alternatives (`a | b`) resolve to the first name, which is what apt
	# would pick too, and version constraints are dropped: the PPA pins its
	# own halves exactly and the rest is whatever noble has.
	while read -r dep; do
		[ -n "$dep" ] && queue+=("$dep")
	done < <(dpkg-deb -f "$deb" Depends 2>/dev/null |
		tr ',' '\n' | awk '{print $1}' | sed 's/|.*//; s/:any$//' | grep .)
done

# ------------------------------------------------------------ the .pc files

# The unpacked .pc files still say `prefix=/usr`, and pkg-config believes
# them: `--modversion` reads this tree and answers 61.19.101 while `--cflags`
# hands out `-I/usr/include/...`, which is the system's 6.1 headers, and
# `--libs` drops its `-L` entirely because /usr/lib is a default search path.
# A build takes 7.1's bindings against 6.1's headers and links 6.1, and the
# only thing that says so is `ldd` on the result. Repoint them at this tree.
for pc in "$libdir"/pkgconfig/*.pc; do
	sed -i \
		-e "s|^prefix=/usr$|prefix=$prefix/usr|" \
		-e "s|^libdir=/usr/lib|libdir=$prefix/usr/lib|" \
		-e "s|^includedir=/usr/include|includedir=$prefix/usr/include|" \
		"$pc"
done

# --------------------------------------------------------------- the check

PKG_CONFIG_PATH=$libdir/pkgconfig pkg-config --cflags libavcodec |
	grep -q "$prefix" || die "the .pc files still point outside $prefix"

missing=$(for lib in "$libdir"/lib*.so.*; do
	LD_LIBRARY_PATH=$libdir ldd "$lib" 2>/dev/null
done | awk '/not found/ {print $1}' | sort -u)

if [ -n "$missing" ]; then
	printf 'ffmpeg7-local: unresolved after unpacking:\n' >&2
	printf '  %s\n' $missing >&2
	exit 1
fi

printf '\nffmpeg %s in %s\n' \
	"$(awk '/^Version:/ {print $2; exit}' <(dpkg-deb -f .debs/libavcodec-dev.deb 2>/dev/null) 2>/dev/null)" \
	"$prefix"
printf 'point a build at it with:\n\n'
exports
