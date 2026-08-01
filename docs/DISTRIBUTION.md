# Distribution: opening a `.insv`, and shipping a Flatpak

Two questions, one document, because they share every file: what has to
exist for a double click on a `.insv` to start Kjerag, and what has to exist
for someone who does not build Rust to have Kjerag at all.

Everything called **measured** below was run on 2026-07-31 on the
development box (AMD Radeon 760M, Phoenix, `radeonsi`, kernel
7.0.11-76070011-generic, Pop!\_OS 24.04, `flatpak` 1.16.6). Nothing here is
quoted from documentation where a command could answer instead.

**Every run below predates the rename** (issue #75), and the application ID
and binary name in the transcripts have been rewritten to the current ones,
which is the whole of what was edited in them. Nothing else in a transcript
was touched and nothing was re-run to produce one, so where a name is the
point rather than the setting, §2.4 and §3.6 say which run has and has not
been repeated since. The ID is `dev.harding.Kjerag` and the binary is
`kjerag`, in the tree and in every line below.

The prototypes are in the tree: `resources/` (what gets installed onto a
desktop, which is the desktop entry, the metainfo, the MIME package and the
icon theme tree), `flatpak/dev.harding.Kjerag.yml` (the manifest) and
`justfile` (the install recipe). `crates/app/res/` is a different thing and
stays where it is: the two jump-button icons are `include_bytes!`d into the
binary.

**Status.** The Flatpak builds, installs, and registers the type. A double
click resolves to Kjerag, verified end to end (§3.8). The blocker this
document was written around is gone: the workspace pins ffmpeg 7.1 and the
runtime ships 7.1.3 (§3.4). The app has an icon (§2.4) and the crate sources
are committed rather than tarred (§3.5).

**And it builds from this tree with nothing applied to it**, which the
Flatpak had never done before: no scratch patch, no 950 MB tarball, no
network. Getting there turned up one defect on `main` and §3.8 is the record
of it.

**The channel is Kjerag's own signed repository**, `https://kjerag.harding.dev/`
(owner, 2026-08-01; issue #137), published by the same version tag that builds
the bundles. §4 is the decision and the machinery under it. It reverses the
ruling of 2026-07-31, which was Flathub and nothing else.

---

## 1. The MIME type for `.insv`

### 1.1 Nothing claims it, and something already claims its sibling

Measured, with shared-mime-info as installed:

```
$ xdg-mime query filetype ~/Videos/VID_…_00_004.insv
application/octet-stream
$ grep -i 'lrv\|insv' /usr/share/mime/globs2
50:video/mp4:*.lrv
```

So `.insv` is unknown to every application on a stock desktop, and `.lrv`
is already spoken for. Both facts drive decisions below.

### 1.2 The name: `video/x-insta360-insv`

Nothing is registered: the IANA media-types registry has no `insv` and no
`insta360`, and neither does shared-mime-info. There is no prior art to
collide with either: no other project appears to have minted a name for
this format.

The shared-mime-info **specification says nothing** about naming an
unregistered format; the only guidance upstream gives is in its
CONTRIBUTING.md, "Mime-types used should be IANA registered mime-types when
possible" and "When old mime-types become registered, the new definition
should include an alias for the old mime-type". RFC 6838 §3.4 discourages
the unregistered tree, and RFC 6648 discourages `X-` prefixes generally;
§3.2's vendor tree wants a registration by the vendor. Against that,
freedesktop's practice is unambiguous: **530** `x-` subtypes against **154**
`vnd.` ones in the installed database, and upstream was still minting new
`x-` names in July 2026 (`image/x-portable-arbitrarymap`,
`image/x-aseprite`, `application/x-hwpx`). Where a registration does appear,
upstream migrates and keeps the old name as an `<alias>`. `video/vnd.avi`
carries `video/x-msvideo`, `video/vnd.rn-realvideo` carries
`video/x-real-video`. That is the road out if Insta360 ever registers one.

So an `x-` name it is, and the remaining question is whether to qualify it
by vendor. freedesktop does both: `video/x-flv` and `video/x-matroska` do
not, `video/x-ms-wmv` and `video/x-sgi-movie` do. The tie is broken by the
closest analogue there is, a dozen file formats belonging to one camera
maker each and registered nowhere:

```
image/x-canon-cr2   image/x-canon-cr3   image/x-nikon-nef
image/x-sony-arw    image/x-fuji-raf    image/x-olympus-orf
image/x-panasonic-rw2  image/x-pentax-pef  image/x-minolta-mrw
```

`<media>/x-<vendor>-<extension>`, one per camera vendor, for a dozen raw
formats none of which is IANA registered. The video tree uses the same shape
where it needs to (`video/x-ms-wmv`, `video/x-sgi-movie`).

`video/x-insta360-insv` is that pattern applied to us, and the three
alternatives lose on specific grounds:

- **`application/x-insta360-insv`** is wrong about what the file is. It is
  video, it holds two HEVC streams and an AAC track, and the top level
  exists to say so.
- **`video/vnd.insta360.insv`** spends Insta360's namespace. The vendor tree
  is for names a vendor registers or at least acknowledges; Insta360 has
  done neither, and inventing inside someone else's tree is a claim we are
  not entitled to make. (freedesktop does carry unregistered `vnd.` names,
  so this is a judgement call, not a rule violation.)
- **`video/x-insv`** is the genuinely close call, and would not be wrong:
  plenty of freedesktop `x-` names carry no vendor. It loses on the
  camera-format precedent above, which is the most similar set of entries in
  the database and is unanimous the other way, and on the fact that four
  letters with no vendor in them are a worse thing to squat than four
  letters with one.

### 1.3 There is no magic rule, and there cannot be a good one

The bytes that identify an `.insv` beyond doubt are the last thirty-two:

```
$ tail -c 32 ~/Videos/VID_…_00_004.insv
8db42d694ccc418790edff439fe026bf
```

which is `crates/meta/src/trailer.rs:48`'s `MAGIC`, and it sits at EOF-32
because the trailer is appended after `moov`/`mdat`
(docs/research/insv-format.md §2). shared-mime-info's `<match offset=…>` is
"the byte offset(s) in the file to check… a single number or a range in the
form `start:end`": a non-negative start and an optional *forward* range.
There is no end-relative offset and no way to express one.

Worth knowing before someone tries: `update-mime-database` **accepts**
`offset="-32"` without a word, writes it into the compiled `magic` file, and
nothing ever matches it. Worse, on this box a rule with a negative offset
makes `gio info -a standard::content-type` segfault, reproducibly, on any
binary file, for as long as the rule is installed anywhere. Forward ranges
do not save it either: GLib sniffs about 16 KB and stops. The good magic
rule is unreachable, and the near miss is a trap.

What is reachable is the `ftyp` brand at offset 4, and it is generic:

```
$ head -c 32 ~/Videos/VID_…_00_004.insv | xxd
00000000: 0000 001c 6674 7970 6176 6331 2014 0200  ....ftypavc1 ...
00000010: 6176 6331 6973 6f6d 0000 0000 0000 0001  avc1isom........
```

`ftypavc1`, compatible brands `avc1isom`. An X4's `.insv` announces itself
as a plain MP4 and nothing else. shared-mime-info's own `video/mp4` magic
lists brands one by one, six in the version installed here and eleven
upstream, and `avc1` is in neither list. That is precisely why an `.insv`
reads as `application/octet-stream` today. The whole database has exactly
one `avc*` rule, `ftypavci`, for AVC-Intra imagery.

Adding `ftypavc1` would claim files that are not ours. MP4RA registers
`avc1` as a generic ISO brand ("Advanced Video Coding extensions"), the same
status as `isom`; any H.264 MP4 may carry it. A rule on it takes other
vendors' files.

So: **glob only**, at the default weight of 50, and no `<magic>` element at
all. Measured with the prototype installed:

```
$ xdg-mime query filetype sample.insv
video/x-insta360-insv
```

The cost is honest and small: a `.insv` renamed to something else is not
recognised. The camera names them, and nothing renames them afterwards.

### 1.4 `sub-class-of video/mp4` is kept, and it earns its place

An `.insv` really is a valid ISO-BMFF file, so the declaration is true. It
is also load-bearing. Measured, with the type and the entry installed and
the binary on `PATH`:

```
$ gio mime video/x-insta360-insv
Default application for “video/x-insta360-insv”: dev.harding.Kjerag.desktop
Registered applications:
	dev.harding.Kjerag.desktop
	fr.handbrake.ghb.desktop
	com.system76.CosmicPlayer.desktop
	mpv.desktop
Recommended applications:
	dev.harding.Kjerag.desktop
```

Kjerag is the default and the only *recommended* handler, and the three
generic players are still offered under Open With, inherited through the
subclass. Drop the subclass and they disappear: a pilot who wants to check
a file in mpv would have nothing to click. cosmic-files reaches them the
same way, through `mime_icon::parent_mime_types()`.

One trap comes with choosing a `video/…` name, and it is worth knowing
before a bug report arrives. cosmic-settings' default-applications page
handles its **Video** row by collecting every MIME type whose name starts
with `video` and setting the chosen application as the default for all of
them. A pilot who picks a video player in Settings silently takes `.insv`
away from Kjerag, and nothing tells them. That is COSMIC's behaviour, not
something this end can fix, and the answer is to know it rather than to pick
a dishonest top-level type to dodge it.

### 1.5 `.lrv` is left alone, deliberately

`.lrv` is already `video/mp4` in shared-mime-info at weight 50 (measured,
§1.1), and it has been since 2014: commit `acbec109`, "Add glob for
low-resolution videos from GoPro". So the extension names two vendors' proxy
formats, and the name alone cannot tell them apart. Handing every GoPro
proxy to a player that only understands Insta360 dual fisheye is a worse
failure than not offering to open the file.

Claiming it also does not work reliably, which settles it. Installing a
competing `*.lrv` glob and typing the same file two ways on one machine:

| our glob weight | `gio` | `mimetype` (File::MimeInfo) |
| --------------- | ----- | --------------------------- |
| 50 (a tie)      | ours  | `video/mp4`                 |
| 60 (higher)     | ours  | `video/mp4`                 |

Two shared-mime-info consumers, one database, different answers, even when
our weight is strictly higher. The recommended lookup order has no
tie-breaker both implement the same way. An `<alias>` would be worse still:
that asserts the two names mean the same thing, which would make every GoPro
proxy an Insta360 video by definition.

This also matches the code: "The camera's LRV proxy may not exist"
(AGENTS.md); proxies are generated, never assumed. Nothing in the app needs
`.lrv` to be a type.

---

## 2. The desktop entry, and installing without root

### 2.1 The entry

`resources/dev.harding.Kjerag.desktop`. Named for the app ID rather than the
binary because `flatpak build-export` only exports files under
`share/applications`, `share/mime/packages` and `share/metainfo` whose names
start with the app ID, and because cosmic-files, cosmic-player and
cosmic-edit all do the same (`res/com.system76.Cosmic*.desktop`).

`resources/` and not `res/`, which is where these three files started, because
the icon tree landed at `resources/icons/` (issue #67) following the official
`cosmic-app-template`, and two resource roots in one tree is a thing to trip
over rather than a distinction anybody wants. cosmic-player uses `res/`; the
template it is a template of uses `resources/`; one root matters more than
which of the two it is.

### 2.2 `Exec=kjerag %f`, not `%U`

The Desktop Entry Specification's field codes: `%f` is a single local file
path, `%F` a list of them, `%u` a single URL, `%U` a list. A launcher given
`%f` and several selected files spawns the program once per file.

`crates/app/src/args.rs` takes exactly one positional argument, treats it as
a `PathBuf`, and refuses two ("only one file can be opened at a time"). It
does not parse URLs, and says why: cosmic-player parses its arguments as
URLs because GStreamer streams from the network, and we decode local files
only. `%f` is the field code that matches that grammar, and it is also the
one the spec describes for our exact case: a program that "cannot handle
multiple file arguments", with remote files copied local first so the
program never sees a URL.

Both launchers that matter would in fact survive `%u`: cosmic-files
substitutes a plain local path for every field code
(`src/mime_app.rs`, `exec_to_command`), and GLib expands `%u` to a local
path too when the file is local. But `%u` is specified as "either a file:
URL or a file path", so taking it would mean handling both forms for no
gain. Two further reasons to leave `Exec` exactly as it is: `%f` is what
makes flatpak's file forwarding kick in (§3.7), and a `@@` written by hand
into `Exec` is a hard flatpak export failure.

### 2.3 `Categories=COSMIC;` fails validation, and stays

```
$ desktop-file-validate resources/dev.harding.Kjerag.desktop
error: value "COSMIC;AudioVideo;Player;Video;" for key "Categories" …
contains an unregistered value "COSMIC"; values extending the format should
start with "X-"
```

The same command against cosmic-files' and cosmic-player's own entries
produces the identical error (measured). System76 ships files that fail this
check, on purpose, because `COSMIC` is how a COSMIC app marks itself.
AGENTS.md says to do what a first-party COSMIC app would do, so we keep it
and write the finding down instead of quietly diverging. Flathub's linter
did not object to it (§3.6).

### 2.4 The icon exists, under a name the binary does not have yet

The icon landed with issue #67: `resources/icons/hicolor/`, an icon theme
tree laid out the way the Icon Theme Specification wants one, so an installer
copies rather than converts. A scalable SVG, PNGs from 256 down to 16, and a
drawing of its own for 32, 24 and 16. `resources/icons/README.md` says what
each file is and `docs/icon.md` says how it got there. Both the `justfile`
install recipe and the Flatpak manifest copy the tree whole rather than
listing sizes, because the tree is generated and a list goes stale.

Every basename is `dev.harding.Kjerag`, the application ID issue #66 settled,
and since issue #75 the desktop entry's `Icon=` key, the binary's `APP_ID`
and these file names are one string. **The gap this section used to describe
is closed.** While the entry named an ID the icons did not carry, the Flatpak
build said this every time, with the ID of the day where the current one now
stands:

```
WARNING: Icon referenced in desktop file but not exported: dev.harding.Kjerag
```

flatpak exports an icon only when its basename starts with the app ID, so a
launcher showed a generic placeholder. **Re-run on 2026-08-01** (§3.8, the
third build): the warning is gone and the eleven icons are exported by name.

The About page and the welcome view read
`hicolor/scalable/apps/dev.harding.Kjerag.svg` as bytes rather than asking
the icon theme for a name (`crates/app/src/app.rs`, `APP_ICON`), which is
what issue #93 did about this gap and what issue #75 kept: measured at the
rename, the name resolves for an installed build and draws nothing at all
for a `cargo run` out of the source tree, which installs no theme.

Nothing about the sizes is left to decide. The Icon Theme Specification's
stated minimum is 48×48, Flathub's floor is "preferably a SVG icon or at
least a 256x256 PNG", Flatpak requires the basename to equal the app ID, and
the tree satisfies all three.

### 2.5 How a desktop actually finds Kjerag

Standard XDG, with one COSMIC wrinkle. The MIME database maps `*.insv` to
our type, `mimeinfo.cache` maps our type to our desktop file, and
`mimeapps.list` records a user's explicit choice if they make one. Measured
in an isolated `XDG_DATA_HOME` with only the two prototype files in it:

```
$ grep insta360 …/applications/mimeinfo.cache
video/x-insta360-insv=dev.harding.Kjerag.desktop;
$ xdg-mime query default video/x-insta360-insv
dev.harding.Kjerag.desktop
```

The wrinkle: **cosmic-files does not read `mimeinfo.cache` at all.** It
enumerates desktop entries itself and reads each one's `MimeType=` key
(`src/mime_app.rs`, `MimeAppCache::reload`), watching the directories for
changes. It does honour `mimeapps.list`, including the
`cosmic-mimeapps.list` variant, for defaults and added/removed
associations. So `update-desktop-database` is not what makes Kjerag appear
in COSMIC's own file manager, but it is still required, because every
GLib/GTK application on the machine does read that cache.

A second COSMIC-specific gap, for the `prefix=$HOME/.local` case:
cosmic-settings enumerates MIME types from `XDG_DATA_DIRS` only, not
`XDG_DATA_HOME`, so a type installed into `~/.local/share/mime` is invisible
to its default-application page. cosmic-files itself is unaffected: its
MIME database loads the user data directory first.

One more trap, measured and worth the line: **GIO hides a desktop entry
whose `Exec` program is not on `PATH`.** With `Exec=kjerag` and no `kjerag`
installed, `gio mime` listed cosmic-player and mpv and not Kjerag; changing
`Exec` to a program that exists made Kjerag both the default and the only
recommended application, with nothing else altered. That is the failure mode
of a `prefix=$HOME/.local` install on a session whose `PATH` lacks
`~/.local/bin`: not an error message, just an app that is silently not
offered.

### 2.6 The install recipe

`justfile`, modelled on cosmic-player's (rev `23d5944`), which is the shape
a first-party COSMIC app uses.

```sh
just build-release
sudo just install                       # /usr/local
just prefix=$HOME/.local install        # no root; see the PATH trap above
```

What lands: the binary, three files (desktop entry, metainfo, MIME package),
and the icon theme tree copied whole out of `resources/icons/`. Then two
caches are refreshed, in this order and not the other:

```sh
update-mime-database  $prefix/share/mime
update-desktop-database $prefix/share/applications
```

`update-mime-database` is what compiles `resources/dev.harding.Kjerag.xml`
into the `globs2`/`subclasses` tables, so it is what makes `*.insv` mean
anything at all. `update-desktop-database` then records which application
handles the type it now knows about. Run them the other way round and the
second one writes a cache entry for a type the first has not created yet.

There is no `vendor` recipe any more, and §3.5 is why: the Flatpak build's
crate sources are committed, so nothing here has a step that needs the
network.

---

## 3. Flatpak

### 3.1 Runtime: `org.freedesktop.Platform` 25.08, and the choice is forced

libcosmic needs rustc 1.93. The Rust SDK extension's version is pinned to
the runtime branch, and measured on Flathub:

| branch | `org.freedesktop.Sdk.Extension.rust-stable` |
| ------ | ------------------------------------------- |
| 23.08  | 1.81.0                                      |
| 24.08  | 1.89.0                                      |
| 25.08  | **1.97.1**                                  |

24.08 is short by four minors, so 25.08 it is; there is no version to
choose. The freedesktop SDK has no `clang`, which `ffmpeg-sys-next`'s
bindgen needs, so `org.freedesktop.Sdk.Extension.llvm21` (clang 21.1.8)
comes along and sets `LIBCLANG_PATH`.

### 3.2 VA-API: it works, and here is the proof

This was the make-or-break. The pipeline is VA-API decode into a dmabuf that
wgpu imports with no copy, so the sandbox has to contain a working
`radeonsi` VA driver and `/dev/dri`. Both do.

The VA driver is not in the runtime. It arrives with
`org.freedesktop.Platform.GL.default`, the Mesa extension that flatpak
installs and mounts automatically for whatever GPU is present
(`download-if = active-gl-driver` in the runtime's metadata), and whose
`merge-dirs` list includes `lib/dri`. Measured, inside the runtime, with
nothing but `--device=dri`:

```
$ flatpak run --command=sh --device=dri org.freedesktop.Platform//25.08 \
    -c 'ls /usr/lib/x86_64-linux-gnu/GL/default/lib/dri | grep drv_video'
nouveau_drv_video.so
r600_drv_video.so
radeonsi_drv_video.so
virtio_gpu_drv_video.so
```

and libva finds it without being told where to look:

```
libva: VA-API version 1.22.0
libva: Trying to open /usr/lib/x86_64-linux-gnu/dri/radeonsi_drv_video.so
libva: Trying to open …/GL/lib/dri/radeonsi_drv_video.so
libva: Found init function __vaDriverInit_1_22
libva: va_openDriver() returns 0
Initialised VAAPI connection: version 1.22
VAAPI driver: Mesa Gallium driver 26.1.4 for AMD Radeon 760M Graphics
              (radeonsi, phoenix, ACO, DRM 3.64, 7.0.11-76070011-generic)
```

That is device creation. Real decode of real footage, same sandbox:

```
$ flatpak run --command=ffmpeg --device=dri --filesystem=home:ro \
    org.freedesktop.Platform//25.08 -hwaccel vaapi \
    -hwaccel_device /dev/dri/renderD128 -hwaccel_output_format vaapi \
    -i ~/Videos/VID_…_00_004.insv -frames:v 30 -f null -
Stream mapping:
  Stream #0:0 -> #0:0 (hevc (native) -> wrapped_avframe (native))
  … Video: wrapped_avframe, vaapi(pc, bt709, progressive), 3840x3840 …
frame=   30 … speed=2.67x
```

3840×3840 HEVC, hardware surfaces out, 2.67× realtime, inside the sandbox.
**`--device=dri` is the whole permission.** No `ffmpeg-full`, no
`LIBVA_DRIVERS_PATH`, no `--device=all`.

The second half of the pipeline is wgpu's Vulkan import of that dmabuf,
which has to work against the *extension's* Mesa (26.1.4) rather than the
host's, and needs the `VK_EXT_image_drm_format_modifier` enable that the
`[patch.crates-io]` fork exists for. Kjerag reports on that itself. Running
the installed Flatpak on real footage inside a headless `cage` session (the
same trick `scripts/uitest.sh` uses):

```
lens:   Insta360 X4 Air v1.2.7_build1, sampling 2 of 2 calibrated
media:  2 lens streams, 3840x3840, 29.970 fps, 53940 frames, 1799.8 s
device: dmabuf import: all extensions enabled
play:  14.82 s, 30.00 fps presented in 30.2 redraws/s, 0 dropped, 0 starved,
       worst 6.0 ms late, sound -4.3 ms, 0 underruns, 0 dropped
```

Full rate, nothing dropped, nothing starved, for as long as it was left
running, and the frame `grim` took out of that session is the reframed view
with a level horizon. The whole pipeline runs inside the sandbox: hardware
decode, zero-copy import, projection pass, and the sound.

### 3.3 HEVC comes from an extension, and it is already there

The base runtime's ffmpeg is built `--disable-decoder='h264,hevc,vc1,vvc'`
(read out of `ffmpeg -version` inside the sandbox), and forcing the base
library over the extension with `LD_PRELOAD` leaves `ffmpeg -decoders` with
no `hevc` at all. **The base runtime cannot decode our footage.**

The decoders arrive from `org.freedesktop.Platform.codecs-extra`, which is
what 25.08 calls what 24.08 called `org.freedesktop.Platform.ffmpeg-full`
(the rename is a documented breaking change in the freedesktop-sdk 25.08.0
release notes). It ships a complete `libavcodec.so.61.19.101`, same soname
and same version, built `--enable-decoders`, which shadows the runtime's
through `add-ld-path = lib`. The part that matters for the manifest: the
runtime's own metadata declares it with **no `no-autodownload`**, so it is
installed and mounted as a related ref along with the runtime and **the app
manifest declares nothing**. On 24.08 it was the other way round: an app had
to name `ffmpeg-full` in `add-extensions` itself.

The catch worth writing down: "installed by default" is not "installed".
`flatpak install --no-related`, or a user pruning extensions, removes the
only `hevc` decoder in the sandbox, and `hevc_vaapi` hangs off it. The
failure is `avcodec_find_decoder` returning null on a file the app just
opened.

**The app now says so, at open** (issue #69, shipped). `open_decoder` asks
`avcodec_find_decoder` for the stream's own codec one line before
ffmpeg-next asks for the same one, and refuses the file with a typed
`MissingDecoder` carrying the codec name; the shell turns that into

> Kjerag has no HEVC decoder here, so that file cannot be played. In a
> Flatpak, the decoder comes from the codecs-extra runtime extension.

and the terminal line reads
`kjerag: <path> not shown: no hevc decoder in this libavcodec`.

This is one of the three lines the shell is still allowed to write over an
error (AGENTS.md, "Errors are the error"), and it is allowed for a reason
that is this section: which package carries the decoder is a packaging fact,
and `kjerag-media` has no way of knowing it. Its own message is true and
leaves the pilot nowhere.

At open rather than at startup, which is what this section originally asked
for and is not what shipped. A startup probe needs a surface to say it on,
says it on a box that never opens a file it cannot play, and can only ever
ask about HEVC. Asking at open costs one lookup on a path that already
exists, asks about the codec the file actually carries, and replaces a line
that is wrong at the moment the pilot is reading it. It is not
Flatpak-specific either: a stripped system ffmpeg produces the same null.

### 3.4 The ffmpeg pin was the blocker, and it is 7.1 now

**Settled** (owner: "bump to 7"; issue #65, closed). The workspace pins
`ffmpeg-next = "7.1"`, the 25.08 runtime ships **ffmpeg 7.1.3**, and the
Flatpak's cargo build no longer has anything to argue with. The rest of this
section is the measurement that produced the decision, kept because it is
also the record of what ffmpeg 7 costs the development box.

The pin used to read `6.1.1`, because Pop!\_OS 24.04 ships ffmpeg 6.1 and the
crate major must match the headers it binds (AGENTS.md). 24.08 ships 7.0.3.
There is no freedesktop branch with ffmpeg 6.

Measured, building `-p kjerag-media` inside `org.freedesktop.Sdk//25.08`:

```
error: could not compile `ffmpeg-next` (lib) due to 30 previous errors
```

thirty errors inside the crate itself: non-exhaustive matches over
`AVPacketSideDataType` and `AVFrameSideDataType` variants that ffmpeg 7
added, plus missing fields. This is not something a flag works around.

Bumping to `ffmpeg-next = "7.1"` makes the crate compile and leaves
**two errors, both ours, both the same cause**: ffmpeg 7 replaced the old
bitmask channel layout with `AVChannelLayout`, which holds raw pointers and
so is not `Send`; `Track` stores one (`crates/media/src/track.rs:40`), which
makes `Reader` not `Send`, which breaks the decode thread's `spawn`
(`crates/media/src/player.rs:176`).

The fix is three hunks in one file: drop the stored field, compute it from
the channel count on demand (`ChannelLayout::default(i32)` still exists in
7.1), and hoist one call out of a `&mut self.resampler` borrow. That is what
landed.

The alternative it was weighed against was building ffmpeg 6.1 as a manifest
module: no source change ever, and the Flatpak stops caring what the runtime
ships, which would also survive the 26.08 runtime moving to ffmpeg 8. It lost
on the price, a large extra module with our own `--enable-vaapi
--enable-decoder=hevc` build to maintain and a Flathub reviewer asking why we
bundle what the runtime already has.

**The bill went to the development box**, which is the part of this decision
that is still live. Ubuntu 24.04 ships ffmpeg 6.1 and will not get a 7, so
the dev box takes its ffmpeg 7.1 from a PPA, and CI takes it from the same
one. Without root, `scripts/ffmpeg7-local.sh` unpacks the same .debs under
`~/.local`. AGENTS.md carries both, including the trap that decides whether
a build is right: 6.1 and 7.1 export the same symbol names, so a link
against the wrong one is silent and `readelf -d <binary> | grep NEEDED` is
what settles it (`libavcodec.so.61` is 7.1, `.so.60` is 6.1).

### 3.5 Sourcing cargo offline: a committed `cargo-sources.json`

flatpak-builder builds with no network, so every dependency has to be a
declared source. The workspace makes that harder than usual: eleven git
dependencies (libcosmic and the pop-os forks it drags along) plus a
`[patch.crates-io]` entry pointing at our own wgpu fork.

**`flatpak/cargo-sources.json` is what ships** (issue #72, closed). It is
generated from `Cargo.lock` by `flatpak-cargo-generator.py` through
`scripts/cargo-sources.sh`, and it is committed: 1404 sources, 500 KB, one
per crate. The manifest lists it beside the `dir` source and builds
`--offline --locked`, so the Flatpak build has no step that touches the
network at all.

It needs no `[patch]` support to cover the fork, because cargo has already
resolved the patch into an ordinary git source in the lock file
(`Cargo.lock:5713`, and seven more crates including `naga`). The stanza it
writes, read out of the committed file:

```toml
[source."https://github.com/aeharding/wgpu"]
git = "https://github.com/aeharding/wgpu"
replace-with = "vendored-sources"
rev = "fb66f36c5cf1135c11523767652ea7a809b3e598"
```

**A `rev`, not a branch, and that closed the one real fragility.** This
section used to end by pointing at it: the patch entry named a branch and
every offline recipe pinned whatever commit that branch resolved to, so a
force-push on the fork would break every recorded build. Issue #68 pinned the
rev in the root manifest, `Cargo.lock` did not re-resolve (the hash after the
`#` was already this commit), and the generated file inherits it.

Two traps come with this route and both are load-bearing:

- **The config lands at `$CARGO_HOME/config`**, so `CARGO_HOME` must be
  exactly `/run/build/<module-name>/cargo`. Anywhere else the file is
  written and never read and the build dies with "you are in the offline
  mode". The manifest's module is named `kjerag` and its `CARGO_HOME` says
  `/run/build/kjerag/cargo`; renaming one without the other breaks the
  build.
- **A change to `Cargo.lock` is a change to `cargo-sources.json`**, in the
  same commit (AGENTS.md, `scripts/cargo-sources.sh`). A stale one is not a
  build that fetches what it is missing; it is a build that fails.

The route not taken was `cargo vendor`, which is what built the first
Flatpak. It works, and measured on this tree it emitted a source replacement
per git remote including the patch, with the forked `wgpu-hal` rather than
crates.io's: the `VK_EXT_image_drm_format_modifier` hunk was present in
`vendor/wgpu-hal/src/vulkan/adapter.rs`. It lost on size. 668 crates, 988 MB,
~950 MB tarred, which cannot live in the repository, so any published build
would have to fetch the tarball as a release asset by URL and sha256. The
generated JSON is also what every COSMIC app on Flathub ships
(`dev.edfloreshz.Tasks`, `dev.edfloreshz.CosmicTweaks`,
`io.github.pixeldoted.cosmic-ext-color-picker`) and what Flathub's own
requirements ask for.

### 3.6 The app ID is `dev.harding.Kjerag`, and the whole tree says so

**Settled** (owner, issue #66): `dev.harding.Kjerag`. `harding.dev` resolves
and answers 200, which is what a reverse-DNS app ID has to assert and what
Flathub checks, so the ID is verifiable today by a `.well-known` file or a
DNS TXT record and costs nothing to hold.

**Issue #75 put it in the tree**, all of it in one mechanical PR: the crate
names, the binary, `App::APP_ID`, the cosmic-config identifiers, the four
`resources/` and `flatpak/` file names and the docs. Half a rename is the
state worth avoiding here, where the entry names one ID, the binary
registers another, and cosmic-config writes to a third. The icons were named
`dev.harding.Kjerag` from issue #67 and everything else now agrees with them
(§2.4).

That rename was also the last thing between the manifest and a linter with
nothing to say about the ID. What the linter said before it, run against the
manifest as it then stood, whose ID was built on a project-named `.app`
domain:

```json
"errors": [
  "finish-args-unnecessary-xdg-config-cosmic-rw-access",
  "finish-args-only-wayland",
  "appid-url-not-reachable"
],
"info": [
  "appid-url-not-reachable: Tried https://<project>.app | … Failed to
   resolve '<project>.app'"
]
```

**That domain did not exist**, which is what made the ID a decision rather
than a detail: a reverse-DNS ID asserts a domain and Flathub checks it.
Measured at the time: the project-named `.app` domain had no DNS record;
`harding.dev` resolves and answers 200; `github.com/aeharding` exists. The
three candidates were that one (buy the domain and serve real HTTPS off it,
since `.app` is HSTS-preloaded and a parking page will not do),
`io.github.aeharding.Kjerag` (verified by the GitHub account, the convention
for a project with no domain), and the one that won, which needs nothing. The
linter has not been re-run since; the ID it now reads is `dev.harding.Kjerag`
and the domain behind it is the one that answers.

Why it was worth settling before publishing rather than after. The app ID is
the cosmic-config path (`~/.config/cosmic/<id>/` and
`~/.local/state/cosmic/<id>/`, so every stored setting), the icon name, the
desktop-entry and MIME-package file names, the metainfo `<id>`, the D-Bus
name and the Wayland `app_id`. cosmic-config has no name-migration path, only
version fallback, and Flatpak's end-of-life rebase does not touch host paths,
so a rename orphans stored settings silently even when the rebase is done
right. The project is pre-release, which is exactly why #75 could do it as a
mechanical sweep with no migration: the settings, the recent files and the
seam pool this box had under the old ID were discarded, and the pool refills
itself by watching.

The other two linter errors are smaller, and both survived the rename:

- `finish-args-only-wayland`: Flathub wants `--socket=fallback-x11` and
  `--share=ipc` alongside Wayland. The manifest deliberately omits them,
  because the frame path is Wayland dmabuf and has never been run under
  Xwayland; claiming X11 support would be a guess. If Flathub is the target
  this has to be either fixed or argued, and "fixed" means someone runs the
  app under Xwayland first.
- `finish-args-unnecessary-xdg-config-cosmic-rw-access`: the linter wants
  `xdg-config/cosmic:ro`. But cosmic-config writes the app's own settings
  under that path, and the two COSMIC apps installed on this box take it
  read-write (`io.github.TopiCsarno.YapCap`: `~/.config/cosmic`;
  `io.github.cosmic_utils.minimon-applet`: `xdg-config/cosmic`). Whether
  `:ro` costs us persisted settings is untested.

### 3.7 Permissions, and why there is no `--filesystem=home`

```yaml
--socket=wayland
--device=dri
--socket=pulseaudio
--filesystem=xdg-config/cosmic
--filesystem=~/.local/state/cosmic
--talk-name=com.system76.CosmicSettingsDaemon
--talk-name=com.system76.CosmicSettingsDaemon.Config.*
--filesystem=xdg-pictures
```

- **`--device=dri`** is §3.2. It is the GPU for both decode and render.
- **`--socket=pulseaudio`** is the sound, and it is enough on its own: the
  flag bind-mounts the PulseAudio socket *and* `/dev/snd`, and the runtime
  ships `/etc/alsa/conf.d/99-pulseaudio-default.conf`, which makes ALSA's
  `default` device the pulse plugin. cpal opens `default` and lands there.
  (The runtime's `ALSA_CONFIG_PATH=/usr/share/alsa/alsa-flatpak.conf` is
  often credited with this and does not do it: its only difference from the
  stock `alsa.conf` is pointing `~/.asoundrc` at a writable path.) PipeWire
  is covered because `pipewire-pulse` provides the socket; there is no
  `--socket=pipewire` and flatpak rejects it. Measured: 0 underruns over a
  25 s playback in the sandbox.
- **The two cosmic paths + the talk-names.** cosmic-config deliberately
  escapes the sandbox: under `FLATPAK_ID` it resolves the **host's** config
  and state directories, not the app's private ones. Granting only
  `xdg-config/cosmic` is what produced this, every run:

  ```
  kjerag: saved state not saved: Read-only file system (os error 30)
          at path "/home/aeharding/.local/state/cosmic/dev.harding.Kjerag/v1/…"
  ```

  so the recent-files list and the window state were silently discarded.
  `--filesystem=~/.local/state/cosmic` fixes it (verified writable after).
  There is no `xdg-state` token in flatpak, hence the literal path.
- **`xdg-pictures`** is where a saved still goes.
  `crates/app/src/shot.rs` resolves `XDG_SCREENSHOTS_DIR` or the pictures
  directory. Without this the still lands in the sandbox's private home and
  the pilot never finds it.
- **No general filesystem access at all**, and it is not needed. Measured
  inside the installed app: `ls ~` shows exactly one entry, `Pictures`.
- **Nothing for icons.** The UI asks the icon theme for
  `camera-photo-symbolic`, `view-fullscreen-symbolic` and friends, and
  `/usr/share/icons` in the sandbox holds an empty `hicolor` before our own
  tree is installed into it. They resolve anyway: flatpak puts the host's
  icon themes on `XDG_DATA_DIRS` as `/run/host/share`, and all four names the
  app uses were found there, `video-x-generic-symbolic` from the host's own
  `Cosmic` theme. A host with no COSMIC icons installed is the untested case;
  `com.system76.Cosmic.BaseApp`, which the COSMIC apps on Flathub build
  against, exists to cover it. Whether to take that base is the one manifest
  question issue #72 left open and this branch is still leaving open: on this
  box nothing needs it, and "untested elsewhere" is not a reason to add a
  dependency, only a reason to know where to look when a report arrives.

Files reach the app two ways and both are portal-shaped:

1. **The file chooser** is already the XDG portal, so the chosen file
   arrives through the document portal and needs no permission.
2. **A double click in a file manager** works because `flatpak
   build-export` rewrites the exported entry. Measured, straight out of
   `~/.local/share/flatpak/exports/share/applications/`:

   ```
   Exec=/usr/bin/flatpak run --branch=master --arch=x86_64 \
        --command=kjerag --file-forwarding dev.harding.Kjerag @@ %f @@
   ```

   The `@@ … @@` markers are flatpak's file-forwarding: the launcher hands
   flatpak a host path, flatpak exports it through the document portal and
   substitutes the sandbox path before the program sees it. Our `%f`
   survives inside the markers. **A double click needs no filesystem
   permission**, and this is the reason the `%f`/`%U` choice in §2.2 had to
   be right.

Drag and drop is the one that does not work, and the reason is not ours to
fix. The app reads `text/uri-list` only (`crates/app/src/dnd.rs`), which is
a host `file://` path: readable outside a sandbox, `ENOENT` inside one.
`application/vnd.portal.filetransfer` is the mime that carries files across
a sandbox boundary, and `dnd.rs` says in its own header that it skips it
deliberately because "nothing about this app is sandboxed yet", a sentence
that expires the day the Flatpak ships.

Except that adding it would buy nothing today. The FileTransfer portal needs
**both** sides: the source starts the transfer and offers the mime, the
target retrieves by key. **cosmic-files does not offer it**: as a drag
source it advertises exactly `text/plain`, `text/plain;charset=utf-8`,
`UTF8_STRING`, `text/uri-list` and `x-special/gnome-copied-files`
(`src/clipboard.rs`). GTK4 apps like Nautilus do offer it, so handling it
would make drops from *those* work. libcosmic supports it opt-in
(`on_file_transfer`), so the change is small when it is worth making.

Until then a drag from cosmic-files into the Flatpak fails. The narrowest
permission that would paper over it is `--filesystem=xdg-videos:ro`, which
Clapper takes and the Flathub linter allows. Not taken here: it is a
permanent grant bought to work around one launcher's missing feature, and
double click and the file chooser both already work.

### 3.8 What the build actually produced

Two builds, and the difference between them is the point.

**The first one, with `vendor.tar` and §3.4's ffmpeg port applied in a
scratch tree.** Release build in 2m44s inside the sandbox, a 7.6 MB
single-file bundle, installed and checked:

```
$ flatpak run dev.harding.Kjerag --version
kjerag 0.1.0
$ xdg-mime query default video/x-insta360-insv
dev.harding.Kjerag.desktop
$ flatpak run --command=sh dev.harding.Kjerag -c 'ls /dev/dri'
by-path  card1  renderD128
```

and then run on real footage under a headless compositor, which is where
§3.2's `dmabuf import: all extensions enabled` and its 30 fps came from.

Two defects that run found, both fixed in the manifest and neither visible
without running it: saved state was silently discarded (§3.7), and the first
version of the manifest had no `~/.local/state/cosmic` grant to discard it
into.

**The second one is this tree, unpatched**, after the ffmpeg pin and
`cargo-sources.json` landed on `main`:

```sh
flatpak run org.flatpak.Builder --user --force-clean \
    --state-dir=scratch/flatpak-builder \
    --repo=scratch/fp/repo scratch/fp/build flatpak/dev.harding.Kjerag.yml
```

and it failed, which is the useful part:

```
error: failed to select a version for the requirement `ffmpeg-next = "^7.1"`
       (locked to 7.1.0)
candidate versions found which didn't match: 6.1.1
location searched: directory source `/run/build/kjerag/cargo/vendor`
```

**`flatpak/cargo-sources.json` on `main` was generated from a lock file that
still said ffmpeg 6.1**, because issue #90 regenerated it on a branch cut
before issue #95 bumped the pin and the two merged clean. Nothing reads that
file except a Flatpak build, so nothing had noticed. It is regenerated here,
and `scripts/cargo-sources.sh --check` now compares the lock file's packages
against the sources with no network and no generator, in CI and by hand.

With that fixed the build runs through:

```
Finished `release` profile [optimized] target(s) in 3m 21s
Exporting share/applications/dev.harding.Kjerag.desktop
Exporting share/mime/packages/dev.harding.Kjerag.xml
Exporting share/metainfo/dev.harding.Kjerag.metainfo.xml
WARNING: Icon referenced in desktop file but not exported: dev.harding.Kjerag
```

No network, no patch, no tarball. The eleven icon files are in the app
(`files/share/icons/hicolor/*/apps/dev.harding.Kjerag.*`), and the warning is
§2.4's: at the time of this run the entry named an ID the icons did not
carry, and flatpak exports only what starts with the app ID. The
binary links the runtime's ffmpeg and says which one, which is the check
AGENTS.md asks for: `readelf -d` reports `libavcodec.so.61`, so ffmpeg 7.1.

Three things about that second build to be exact about.

- **It was not installed and not run.** §3.2's playback numbers stand on the
  first build and have not been re-measured against this one, nor against the
  third build below, which was installed and asked its version and nothing
  more.
- **`--state-dir` is not decoration.** flatpak-builder's cache defaults to
  `.flatpak-builder` in the working directory, which for this manifest is the
  repository root, and the `dir` source copies the whole repository into the
  build. So the default puts 1.7 GB of build cache next to the source and
  then copies it into the next build of itself. `scratch/` is skipped and
  gitignored, which is why the cache belongs there.
- **Cargo warns about the file name it is given.**
  `/run/build/kjerag/cargo/config is deprecated in favor of config.toml`.
  That name is the generator's, cargo still reads it, and the day it stops is
  the day this breaks; it is a thing to watch rather than a thing to patch
  around here.

**The third build is the release pipeline's** (issue #106, 2026-08-01), the
same command from the branch that added the workflow, and it is the first
repeat of any of this since the rename. **The icon warning is gone**: the
export names the icons instead of complaining about them
(`Exporting share/icons/hicolor/32x32/apps/dev.harding.Kjerag.png` and ten
more) and the log carries no `WARNING:` line at all, which is what §2.4 said
was untested. The cargo build inside the sandbox took 1m58s and the bundle is
8.1 MB. That one was installed: `flatpak install --user
./kjerag-0.1.0-x86_64.flatpak` and then `flatpak run dev.harding.Kjerag
--version` prints `kjerag 0.1.0`.

The MIME package rides along: the first build's flatpak exported it to
`~/.local/share/flatpak/exports/share/mime/packages/dev.harding.Kjerag.xml`,
so installing the Flatpak teaches the whole desktop what a `.insv` is. No
separate step.

Tooling used to get here, all `--user`, no root and no system packages:
`org.flatpak.Builder`, `org.freedesktop.Sdk.Extension.rust-stable//25.08`,
`org.freedesktop.Sdk.Extension.llvm21//25.08`.

### 3.9 The 0.1.1 bundle, measured after the fact

Every build above was checked by starting it. **None of them was ever asked
to play anything**, and 0.1.1 shipped on the strength of a window and a
`--version` line. This is the measurement that was missing, taken on
2026-08-01 against the published `kjerag-0.1.1-x86_64.flatpak` downloaded
from its own release, on the §1 test bench of docs/research/gpu-pipeline.md.

**It plays**, and **the mode catches what it was built to catch.**
`KJERAG_FLATPAK=dev.harding.Kjerag scripts/uitest.sh <file>` (the mode
docs/RELEASING.md now calls the release check) against real X4 Air footage,
with the harness as it stands today: **32 checks, 6 failed**, against the
native path's **32 checks, 2 failed** in the same session. Two of the six are
the harness's standing flakes, the toast placement and `ctrl+v`. **The other
four are real, and all four are the bundle being four weeks old**:

```
FAIL  an open with no frame yet draws the backdrop
      the pane drew the test pattern: its mirrored halves read
      41 98 212 and 170 98 211, which no picture is
FAIL  a GoPro file is refused by name
      ... not shown: file has no video stream
FAIL  an .osv with nothing in it is still named
FAIL  escape takes the alert away and leaves the window as it was
```

The first is the owner's own report of 0.1.1 read back to us by a machine:
the test pattern in the picture area with no frame yet, which issue #100's
backdrop replaced. The other three are the format refusal of issue #107,
which 0.1.1 also predates. All four are fixed on `main` and all four pass on
the native path, which is the control: the checks are not broken, the bundle
is old. **That is the whole argument for this mode.** Every one of these was
reachable by a machine on release day and none of them was asked.

Directly under `cage`, the same bundle on a 3840x3840 X4 Air file and a
2880x2880 ONE X2 file:

```
device: dmabuf import: all extensions enabled
play:      9.90 s, 30.00 fps presented in 30.2 redraws/s, 0 dropped,
           0 starved, sound +3.1 ms, 0 underruns
```

Zero-copy, not the copy fallback: that line is the import's own, and it says
`all extensions enabled` rather than naming one that is missing. Sustained
45 s, both cameras, and the same numbers whether the path arrived on the
command line or as a document-portal path.

**What the sandbox brings its own of**, all measured inside it and all
different from the host:

| | host | sandbox (runtime 25.08) |
|---|---|---|
| Mesa (radeonsi + RADV) | 25.2.8 | **26.1.5** |
| ffmpeg | 6.1.1 system, 7.1 from the PPA | **7.1.3**, the runtime's |
| libva | 2.20.0 | **2.22.0** |
| HEVC decoder | present | **present**, in the base runtime |

The frame path in the sandbox therefore runs on a different Mesa and a
different libva from the ones every number in docs/research was taken on, and
it behaves the same. Two details worth keeping:

- **The VA driver is found through the GL extension, not `/usr/lib/.../dri`.**
  In the sandbox that directory holds only `intel-vaapi-driver` and
  `nvidia-vaapi-driver` subdirectories; libva's fourth candidate,
  `/usr/lib/x86_64-linux-gnu/GL/lib/dri/radeonsi_drv_video.so`, is the one
  that opens. A sandbox without `org.freedesktop.Platform.GL.default` has no
  VA-API at all.
- **HEVC needs no `codecs-extra` extension here.** The base runtime's
  libavcodec 61.19.101 carries the `hevc` decoder, which is what §3.3 found
  and what `strings::missing_decoder` names as the thing to install if it
  ever is not.

**A by-name grant is resolved against the caller's environment, which is what
lets the harness test the shipped sandbox without writing to the desktop**
[measured]. `--filesystem=xdg-config/cosmic` follows the launching process's
`XDG_CONFIG_HOME`, `--filesystem=~/.local/state/cosmic` follows its `HOME`
(and not `XDG_STATE_HOME`, which is a different directory and is ignored for
this), and `~/.var/app/<id>` follows `HOME` as well, flatpak's own `.ld.so`
cache included. Point those two variables at a scratch directory and the real
`~/.config/cosmic` is **not bound into the sandbox at all**: the app still
holds the grant, and there is nothing behind it. Proven by running the bundle
on real footage with the redirect in place and comparing the developer's own
directories byte for byte:

```
real Kjerag settings before: 38559f9c3810d988
play lines: 4
real Kjerag settings after:  38559f9c3810d988
scratch: .../cosmic/dev.harding.Kjerag/v1/{seam_pool,recent_files}
```

One trap in that: flatpak **skips a by-name bind whose source does not
exist**, so a scratch `~/.local/state/cosmic` that has not been created is not
an empty grant, it is an absent one, and the app quietly has no state
directory. `scripts/uitest.sh` makes both before it boots anything.

**One real defect the run turned up, and it is not about frames.** Footage
whose two lenses are two files (ONE X2: `..._00_001.insv` beside
`..._10_001.insv`) opened through the file chooser gets only the file the
pilot picked, because the document portal exports one document, and the app
says nothing about it:

```
lens:   Insta360 ONE X2 ..., sampling 1 of 2 calibrated
media:  1 lens stream, 2880x2880, ...
```

Half the sphere, silently, in the sandbox only. The same path passed on the
command line with `--filesystem` gives `2 lens streams from 2 files`.

---

## 4. Publishing

### 4.1 The channel is Kjerag's own signed repository

**`https://kjerag.harding.dev/`, published by the version tag** (owner,
2026-08-01; issue #137). A flatpak remote is a static OSTree repository over
HTTP: `flatpak-builder --repo=<dir>` writes one, a GPG key signs it, GitHub
Pages serves it, and a `.flatpakref` beside it makes an install one click.

**This reverses 2026-07-31**, which was Flathub and nothing else, and it is
not a change of mind about the price. Issue #71 costed self-hosting correctly
and every line of that costing still holds: nobody browses a one-app remote,
so there is no discovery in it, and update delivery and key management are
ours permanently. What changed is §4.2, the availability of the thing those
costs were being paid to avoid.

**One cost issue #71 named is not one.** It recorded that a repository with no
AppStream data leaves the app installable from the remote and invisible in
COSMIC Store, GNOME Software and Discover. `flatpak build-update-repo`
composes that data itself, from the metainfo the app already installs, and
writes `appstream2/<arch>` into the repository; the tool flatter runs on every
build is that command. Measured on the dry run, §4.3.

### 4.2 Flathub is not the channel

Flathub's contribution policy of 2026-05-29 rules out this project's
development process, so that route is not open here.

What the tree keeps from the plan that was: the shape of a submission, because
it is also the shape of a good app anywhere. The app ID's domain is one the
owner controls and it answers (§3.6), the permission set is small and every
line of it is argued (§3.7, docs/PERMISSIONS.md), and the metainfo is the file
every software centre reads hardest, ours included now. The two
`flatpak-builder-lint` complaints §3.6 records are still open questions rather
than settled answers, and §5 still lists them.

The way back, if the policy ever changes, is one origin. A Flathub build of
this manifest would be the same app on the same `stable` branch, so switching
is `flatpak install flathub dev.harding.Kjerag` and deleting our remote, with
no reinstall and no lost settings. **That is why the branch is `stable` and
not `master`.**

### 4.3 How a tag publishes it

`.github/workflows/release.yml`, the `pages` job, next to the `bundle` job
that issue #106 built. Three published actions and one step of shell; the
model is andyholmes/valent's `cd.yml`, which does the same job for a GNOME app
and proves the aarch64 half.

1. **`crazy-max/ghaction-import-gpg`** imports the signing key from the
   `GPG_PRIVATE_KEY` and `GPG_PASSPHRASE` repository secrets. No key, no
   repository: the job fails rather than publishing something a client would
   have to be told to trust anyway.
2. **`andyholmes/flatter`** runs `flatpak-builder --repo` against the same
   committed manifest, signs the result, runs `flatpak build-update-repo`
   (which is what writes the AppStream refs and signs the summary), and caches
   the repository so the next build adds to it rather than replacing it. It
   runs in flatter's own `rust:25.08` image, which carries the freedesktop
   Platform, Sdk and rust-stable for **both** arches already; `llvm21` is the
   one thing the manifest names that the image lacks, and
   `--install-deps-from=flathub` fetches it.
3. **`JamesIves/github-pages-deploy-action`** pushes the repository directory
   to the Pages branch as a single commit, at the root of the site, so the
   remote URL is the bare domain.

**The two arches share one repository, so they run one at a time**
(`max-parallel: 1`). The second job restores the first's cache and adds to it,
which is what makes one deploy carry both. Running them together would have
each deploy a repository holding only its own arch, and the loser's would win.

**Three files ride along**, copied out of `flatpak/pages/` and written beside
the OSTree objects:

- `CNAME`, which is what makes the custom domain stick, and which flatter also
  reads out of the working directory to write the `Url` into the
  `index.flatpakrepo` it generates.
- `.nojekyll`, which is load-bearing rather than cargo cult: a branch-source
  Pages site is a Jekyll build by default, Jekyll drops paths beginning with
  an underscore, and OSTree names static deltas in a base64 alphabet that
  contains one.
- `index.html`, because the root of the domain is otherwise a 404 and the
  domain is the thing people are given.

**And two are generated**, because they carry the public half of the signing
key and a committed copy would go stale the day it rotates:
`kjerag.flatpakrepo` (`Title`, `Url`, `GPGKey`) adds the remote, and
`dev.harding.Kjerag.flatpakref` (the same plus `Name`, `Branch=stable`,
`SuggestRemoteName=kjerag`, `RuntimeRepo=flathub`, `IsRuntime=false`) installs
the app and adds the remote in one click. `RuntimeRepo` is what lets that work
on a machine that has never had Flathub configured: the app is ours, the
runtime under it is not.

Three things to know before touching this.

- **The repository does not survive from tag to tag.** flatter's cache is a
  GitHub Actions cache, and Actions caches are scoped to the ref that wrote
  them: the two jobs of one tag share theirs, and the next tag starts from
  nothing. So each release publishes a repository holding that release alone.
  Updates work exactly as they should, because a client resolves the ref to
  whatever commit the summary now names; what is given up is static deltas, so
  an update downloads the app whole. The app is 8 MB.
- **GitHub Pages is a soft-limit host**: 1 GB per site, 100 GB of bandwidth a
  month, and it says out loud that a heavily subscribed Flatpak repository may
  be throttled. One release of one 8 MB app on two arches is nowhere near it;
  the day it is, the answer is a real object store, and only the `Url` fields
  above change.
- **`flatpak build-update-repo` runs inside flatter's cache save**, so
  disabling the cache (`cache-key: ''`) would leave the repository with no
  updated summary and no AppStream refs, which reads as a remote that has
  nothing in it. Do not disable the cache.

**Measured**, on the `0.1.1-pipelinetest1` dry run of 2026-08-01 (workflow run
30720758898), signed with a throwaway key generated for it:

```
Dependency Extension: org.freedesktop.Sdk.Extension.llvm21 25.08
Installing org.freedesktop.Sdk.Extension.llvm21/x86_64/25.08 from flathub
    Finished `release` profile [optimized] target(s) in 6m 21s
Running appstreamcli compose
Exporting dev.harding.Kjerag to repo
flatpak build-update-repo --gpg-sign=… /__w/kjerag/kjerag/repo
Updating appstream branch
```

Sixteen minutes for the x86_64 job, no `WARNING:` line of any kind, and eight
refs in the deployed repository afterwards: `app` and `.Debug` for each arch,
`appstream` and `appstream2` for each arch. **15 MB for one arch, 30 MB for
both**, which is the number to hold against the 1 GB cap. GitHub built the
Pages site from the branch in 22 s and its certificate for the domain came
back approved.

Then, from this box, against `https://kjerag.harding.dev/` and nothing local:

```
$ flatpak remote-add --user --from kjerag …/kjerag.flatpakrepo
$ flatpak remote-ls --user kjerag
app/dev.harding.Kjerag/x86_64/stable   x86_64   stable   13.1 MB
$ flatpak search Kjerag
Kjerag  Play Insta360 360 video  dev.harding.Kjerag  0.1.1  stable  kjerag
$ flatpak install --user kjerag dev.harding.Kjerag && flatpak run …//stable --version
kjerag 0.1.1
```

The remote is GPG verified, which is not a claim: it was added with no
`--no-gpg-verify` and the summary signature is what `remote-ls` checks before
it answers. `flatpak search` finding it is §4.1's AppStream point, arriving
from the remote's own `appstream2` ref. And the one-click file does the whole
of it in one command, remote included:

```
$ flatpak install --user --from …/dev.harding.Kjerag.flatpakref
$ flatpak remotes | grep kjerag
kjerag   user   https://kjerag.harding.dev/
```

One thing to expect: **GitHub Pages answered one pull with HTTP 503** and the
same command succeeded on retry a minute later. It is a static host with a
free tier, not a CDN anyone is paying for.

**And the update crosses the rebuild**, which is the one claim the bullet about
deltas above puts at risk: a repository built from an empty cache shares no
history with the one a client already has. Second dry run,
`0.1.1-pipelinetest2`, against a client installed from the first:

```
deployed x86_64 ref: 7d8c4c824c63202e215ea3642bfed94b16fdb54277340b7748ff467ea29b0d6d
installed before:    edc488803015a25cd8e3fec366e4ca0fee4712d788fc84a6c68c3babfe67f095
$ flatpak update dev.harding.Kjerag
Updates complete.
installed after:     7d8c4c824c63202e215ea3642bfed94b16fdb54277340b7748ff467ea29b0d6d
```

An OSTree client resolves the ref to whatever the summary now names and pulls
it; ancestry is not a condition. The same client asked between the two tags,
after the aarch64 half of the first run had deployed over the x86_64 half, said
`Nothing to do`, which is the other half of the check: the second arch's deploy
does not churn the first arch's ref.

Both dry runs ran under a scratch `FLATPAK_USER_DIR`, so nothing in them
touched the installation this desktop uses.

### 4.4 The single-file bundle stays

Because it is not a distribution channel and never was: `flatpak build-bundle`
produces one `.flatpak` that installs with no remote at all, which is how the
owner gets a build to click a `.insv` against, and how anyone with a machine
that should not carry a third-party remote gets one.

Since issue #106 that bundle is what a version tag produces: the release
workflow builds this manifest with Flatpak's own GitHub action, once per arch
on a runner of that arch, and attaches both bundles and their `.sha256` files
to a GitHub Release (docs/RELEASING.md). The x86_64 one is the one anybody has
run; the aarch64 one is compiled and unit tested and nothing more. The
action records the Flathub repository in the bundle as it exports it, which is
what lets it install on a machine that has never had Flathub configured: the
bundle carries the app, and that URL is where the runtime under it comes
from.

**Both routes install branch `stable`** since issue #137, which is the whole
point of naming it in the manifest rather than on a command line: a machine
that took a bundle and later adds the remote has one Kjerag on it, and
`flatpak update` reaches it.

---

## 5. What is settled, and what is left

Settled by the owner, on 2026-07-31 except where dated otherwise:

| question | answer |
| -------- | ------ |
| the icon | shipped, `resources/icons/` (issue #67, §2.4) |
| the app ID | `dev.harding.Kjerag`, in the tree since issue #75 (§3.6) |
| the ffmpeg pin | 7.1, and the dev box takes ffmpeg 7 from a PPA (§3.4) |
| the channel | our own signed repository at `kjerag.harding.dev`, 2026-08-01 (issue #137, §4.1) |
| the branch | `stable`, in the repository and the bundles alike (§4.2) |
| Flathub | not open to this project (§4.2) |
| the licence | `AGPL-3.0-only`, which is what the metainfo already says |

Left. The first is the owner's; the other two are work nobody has done:

1. **Screenshots.** The remote carries AppStream data, so COSMIC Store,
   GNOME Software and Discover list the app, and a listing with no picture is
   the one thing about that listing which still looks unfinished. Ours has to
   be a real window over real footage, which is the owner's to take and to
   agree to publish. The metainfo carries the commented-out `<screenshots>`
   block waiting for URLs.
2. **The X11 question.** `flatpak-builder-lint` wants `--socket=fallback-x11`
   and `--share=ipc`; the manifest omits both because the frame path is
   Wayland dmabuf and has never been run under Xwayland (§3.6). Nobody
   reviews us now, so nothing forces the question, but the answer is still
   unknown and the way to know it is to run the app under Xwayland.
3. **`xdg-config/cosmic:ro`.** The linter wants read-only; cosmic-config
   writes the app's own settings under that path and the two COSMIC apps
   installed on this box both take it read-write. Whether `:ro` costs
   persisted settings is untested (§3.6).
