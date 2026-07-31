# Distribution: opening a `.insv`, and shipping a Flatpak

Two questions, one document, because they share every file: what has to
exist for a double click on a `.insv` to start Kyerag, and what has to exist
for someone who does not build Rust to have Kyerag at all.

Everything called **measured** below was run on 2026-07-31 on the
development box (AMD Radeon 760M, Phoenix, `radeonsi`, kernel
7.0.11-76070011-generic, Pop!\_OS 24.04, `flatpak` 1.16.6). Nothing here is
quoted from documentation where a command could answer instead.

The prototypes are in the tree: `res/` (what gets installed onto a desktop),
`flatpak/app.kyerag.Kyerag.yml` (the manifest) and `justfile` (the install
and vendor recipes). `crates/app/res/` is a different thing and stays where
it is: those icons are `include_bytes!`d into the binary.

**Status.** The Flatpak builds, installs, and registers the type. A
double click resolves to Kyerag, verified end to end. One blocker stands
between this branch and a Flatpak built from `main` unchanged, and it is
section 3.4.

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
Default application for “video/x-insta360-insv”: app.kyerag.Kyerag.desktop
Registered applications:
	app.kyerag.Kyerag.desktop
	fr.handbrake.ghb.desktop
	com.system76.CosmicPlayer.desktop
	mpv.desktop
Recommended applications:
	app.kyerag.Kyerag.desktop
```

Kyerag is the default and the only *recommended* handler, and the three
generic players are still offered under Open With, inherited through the
subclass. Drop the subclass and they disappear: a pilot who wants to check
a file in mpv would have nothing to click. cosmic-files reaches them the
same way, through `mime_icon::parent_mime_types()`.

One trap comes with choosing a `video/…` name, and it is worth knowing
before a bug report arrives. cosmic-settings' default-applications page
handles its **Video** row by collecting every MIME type whose name starts
with `video` and setting the chosen application as the default for all of
them. A pilot who picks a video player in Settings silently takes `.insv`
away from Kyerag, and nothing tells them. That is COSMIC's behaviour, not
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

`res/app.kyerag.Kyerag.desktop`. Named for the app ID rather than the binary
because `flatpak build-export` only exports files under
`share/applications`, `share/mime/packages` and `share/metainfo` whose names
start with the app ID, and because cosmic-files, cosmic-player and
cosmic-edit all do the same (`res/com.system76.Cosmic*.desktop`).

### 2.2 `Exec=kyerag %f`, not `%U`

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
$ desktop-file-validate res/app.kyerag.Kyerag.desktop
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

### 2.4 The icon does not exist yet

`Icon=app.kyerag.Kyerag` names an icon nothing installs. The build says so:

```
WARNING: Icon referenced in desktop file but not exported: app.kyerag.Kyerag
```

and the About page already asks for the same name
(`crates/app/src/app.rs:1079`, `icon::from_name(App::APP_ID)`). This is an
asset the project has to acquire, not a decision code can make. What is
needed, from the specs rather than from taste: an SVG at
`res/icons/hicolor/scalable/apps/app.kyerag.Kyerag.svg`, a 48×48 PNG
(the Icon Theme Specification's stated minimum), and a 256×256 PNG
(Flathub's floor, "preferably a SVG icon or at least a 256x256 PNG"). The
file basename must equal the app ID, because Flatpak requires it. That is
also the layout cosmic-files ships, one SVG per nominal size 16 through 256
under `hicolor/<size>/apps/`. Until it exists the launcher shows a generic
placeholder and the About page shows nothing.

### 2.5 How a desktop actually finds Kyerag

Standard XDG, with one COSMIC wrinkle. The MIME database maps `*.insv` to
our type, `mimeinfo.cache` maps our type to our desktop file, and
`mimeapps.list` records a user's explicit choice if they make one. Measured
in an isolated `XDG_DATA_HOME` with only the two prototype files in it:

```
$ grep insta360 …/applications/mimeinfo.cache
video/x-insta360-insv=app.kyerag.Kyerag.desktop;
$ xdg-mime query default video/x-insta360-insv
app.kyerag.Kyerag.desktop
```

The wrinkle: **cosmic-files does not read `mimeinfo.cache` at all.** It
enumerates desktop entries itself and reads each one's `MimeType=` key
(`src/mime_app.rs`, `MimeAppCache::reload`), watching the directories for
changes. It does honour `mimeapps.list`, including the
`cosmic-mimeapps.list` variant, for defaults and added/removed
associations. So `update-desktop-database` is not what makes Kyerag appear
in COSMIC's own file manager, but it is still required, because every
GLib/GTK application on the machine does read that cache.

A second COSMIC-specific gap, for the `prefix=$HOME/.local` case:
cosmic-settings enumerates MIME types from `XDG_DATA_DIRS` only, not
`XDG_DATA_HOME`, so a type installed into `~/.local/share/mime` is invisible
to its default-application page. cosmic-files itself is unaffected: its
MIME database loads the user data directory first.

One more trap, measured and worth the line: **GIO hides a desktop entry
whose `Exec` program is not on `PATH`.** With `Exec=kyerag` and no `kyerag`
installed, `gio mime` listed cosmic-player and mpv and not Kyerag; changing
`Exec` to a program that exists made Kyerag both the default and the only
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

Four files land (binary, desktop entry, metainfo, MIME package) and then two
caches are refreshed, in this order and not the other:

```sh
update-mime-database  $prefix/share/mime
update-desktop-database $prefix/share/applications
```

`update-mime-database` is what compiles `res/app.kyerag.Kyerag.xml` into the
`globs2`/`subclasses` tables, so it is what makes `*.insv` mean anything at
all. `update-desktop-database` then records which application handles the
type it now knows about. Run them the other way round and the second one
writes a cache entry for a type the first has not created yet.

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
`[patch.crates-io]` fork exists for. Kyerag reports on that itself. Running
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
opened. Worth a startup check that says so out loud rather than a black
frame.

### 3.4 The blocker: the runtime's ffmpeg is 7.1, ours is pinned to 6.1

The workspace pins `ffmpeg-next = "6.1.1"` because Pop!\_OS 24.04 ships
ffmpeg 6.1 and the crate major must match the headers it binds (AGENTS.md).
The 25.08 runtime ships **ffmpeg 7.1.3**; 24.08 ships 7.0.3. There is no
freedesktop branch with ffmpeg 6.

Measured, building `-p kyerag-media` inside `org.freedesktop.Sdk//25.08`:

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

The fix, applied and verified in a scratch tree, is three hunks in one file:
drop the stored field, compute it from the channel count on demand
(`ChannelLayout::default(i32)` still exists in 7.1), and hoist one call out
of a `&mut self.resampler` borrow. `kyerag-media` then builds clean.

That leaves a real decision, which is the owner's:

- **Bump the pin to 7.1.** One file changes, and the flatpak builds from
  the tree unchanged. But the pin is what makes the app build against the
  system ffmpeg on the development box, and Pop!\_OS 24.04 is on 6.1: after
  the bump, `cargo build` on the dev box needs a newer ffmpeg than the
  distribution has. Either the dev box moves or the dev box breaks.
- **Build ffmpeg 6.1 as a manifest module.** No source change ever, and the
  Flatpak stops caring what the runtime's ffmpeg is, which also survives
  the 26.08 runtime moving to ffmpeg 8. The price is a large extra module,
  our own `--enable-vaapi --enable-decoder=hevc` build to maintain, and a
  Flathub reviewer asking why we bundle what the runtime already has.

Neither is obviously right and this document does not choose. The Flatpak
built for this branch took the first route in a scratch tree, because a
built artifact answers more questions than an argument does.

### 3.5 Sourcing cargo offline: `cargo vendor`, not the generator

flatpak-builder builds with no network, so every dependency has to be a
declared source. The workspace makes that harder than usual: eleven git
dependencies (libcosmic and the pop-os forks it drags along) plus a
`[patch.crates-io]` entry pointing at our own wgpu fork.

`cargo vendor` handles all of it in one step. Measured on this tree, it
emitted a source replacement per git remote, including the patch:

```toml
[source."git+https://github.com/aeharding/wgpu?branch=v28-drm-modifier-backport"]
git = "https://github.com/aeharding/wgpu"
branch = "v28-drm-modifier-backport"
replace-with = "vendored-sources"
```

and the vendored `wgpu-hal` is the forked one, not crates.io's: the
`VK_EXT_image_drm_format_modifier` hunk is present in
`vendor/wgpu-hal/src/vulkan/adapter.rs`. 668 crates, 988 MB, ~950 MB
tarred.

`just vendor` produces `vendor.tar`, and it is the only step that touches
the network. The manifest takes it as an `archive` source with
`strip-components: 0` and builds `--offline --locked`. The `head -n -1`
trick in the recipe, which drops cargo's absolute `directory = …` line and
appends a relative one, is cosmic-player's, from the same justfile the install
recipes come from.

The alternative is `flatpak-cargo-generator.py` from flatpak-builder-tools,
which reads `Cargo.lock` and emits a `cargo-sources.json` of one source per
crate. It was checked rather than assumed, and it does handle our case: run
against this tree's lock file it exits 0 with 1408 sources and writes the
right stanza for the fork,

```toml
[source."https://github.com/aeharding/wgpu"]
git = "https://github.com/aeharding/wgpu"
replace-with = "vendored-sources"
branch = "v28-drm-modifier-backport"
```

because it needs no `[patch]` support at all: cargo has already resolved the
patch into an ordinary git source in the lock file (`Cargo.lock:5713`, and
seven more crates including `naga`).

Two reasons to expect that route to win in the end, neither of them urgent:
the generated JSON is 501 KB and can live in the repository, where a 950 MB
tarball cannot; and it is what every COSMIC app on Flathub does
(`dev.edfloreshz.Tasks`, `dev.edfloreshz.CosmicTweaks`,
`io.github.pixeldoted.cosmic-ext-color-picker` all ship a
`cargo-sources.json`), which is also what Flathub's own requirements ask
for. Its trap is that the config it writes is `$CARGO_HOME/config`, so
`CARGO_HOME` must be exactly `/run/build/<module-name>/cargo` or cargo never
reads it.

`vendor.tar` is what this branch uses because it is what was built and
installed and run. For Flathub the tarball would have to be a release asset
fetched by URL and sha256; switching to `cargo-sources.json` instead is the
smaller change and the better-precedented one.

One real fragility either way: the patch entry pins a **branch**, and both
tools pin the commit that branch resolved to. Force-push the fork's branch
and every recorded build breaks. `rev = "fb66f36…"`, or a tag, costs nothing
and removes that.

### 3.6 The app ID is not usable as it stands

The code says `app.kyerag.Kyerag` (`crates/app/src/app.rs:235`). Flathub's
own linter, run against the manifest:

```json
"errors": [
  "finish-args-unnecessary-xdg-config-cosmic-rw-access",
  "finish-args-only-wayland",
  "appid-url-not-reachable"
],
"info": [
  "appid-url-not-reachable: Tried https://kyerag.app | … Failed to resolve
   'kyerag.app'"
]
```

**`kyerag.app` does not exist.** A reverse-DNS app ID asserts control of the
domain, and Flathub checks. Measured: `kyerag.app` has no DNS record;
`harding.dev` resolves and answers 200; `github.com/aeharding` exists.

The options, and none of them is free:

| ID | what it needs | note |
| -- | ------------- | ---- |
| `app.kyerag.Kyerag` | buy `kyerag.app` and serve it | `.app` is HSTS-preloaded, so it needs real HTTPS, not a parking page |
| `dev.harding.Kyerag` | nothing; `harding.dev` already answers | verifiable today |
| `io.github.aeharding.Kyerag` | the GitHub account, which exists | the convention for a project with no domain |

Changing it later is not a rename of one string. The app ID is the
cosmic-config path (`~/.config/cosmic/app.kyerag.Kyerag/`, so every stored
setting), the icon name, the desktop-entry and MIME-package file names, the
metainfo `<id>`, and on Flathub a published ID is close to permanent. This
is cheap to settle now and expensive to settle after the first release.

The other two linter errors are smaller:

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
  kyerag: saved state not saved: Read-only file system (os error 30)
          at path "/home/aeharding/.local/state/cosmic/app.kyerag.Kyerag/v1/…"
  ```

  so the recent-files list and the window state were silently discarded.
  `--filesystem=~/.local/state/cosmic` fixes it (verified writable after).
  There is no `xdg-state` token in flatpak, hence the literal path.
- **`xdg-pictures`** is where a saved still goes.
  `crates/app/src/shot.rs` resolves `XDG_SCREENSHOTS_DIR` or the pictures
  directory. Without this the PNG lands in the sandbox's private home and
  the pilot never finds it.
- **No general filesystem access at all**, and it is not needed. Measured
  inside the installed app: `ls ~` shows exactly one entry, `Pictures`.
- **Nothing for icons.** The UI asks the icon theme for
  `camera-photo-symbolic`, `view-fullscreen-symbolic` and friends, and
  `/usr/share/icons` in the sandbox holds an empty `hicolor`. They resolve
  anyway: flatpak puts the host's icon themes on `XDG_DATA_DIRS` as
  `/run/host/share`, and all four names the app uses were found there,
  `video-x-generic-symbolic` from the host's own `Cosmic` theme. A host with
  no COSMIC icons installed is the untested case; `com.system76.Cosmic.BaseApp`,
  which the COSMIC apps on Flathub build against, exists to cover it.

Files reach the app two ways and both are portal-shaped:

1. **The file chooser** is already the XDG portal, so the chosen file
   arrives through the document portal and needs no permission.
2. **A double click in a file manager** works because `flatpak
   build-export` rewrites the exported entry. Measured, straight out of
   `~/.local/share/flatpak/exports/share/applications/`:

   ```
   Exec=/usr/bin/flatpak run --branch=master --arch=x86_64 \
        --command=kyerag --file-forwarding app.kyerag.Kyerag @@ %f @@
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

```sh
just vendor
flatpak run org.flatpak.Builder --user --force-clean \
    --repo=scratch/fp/repo scratch/fp/build flatpak/app.kyerag.Kyerag.yml
flatpak build-bundle scratch/fp/repo kyerag.flatpak app.kyerag.Kyerag master
```

Release build in 2m44s inside the sandbox; a 7.6 MB single-file bundle. The
build tree had §3.4's three-hunk ffmpeg port applied; everything else,
manifest included, is what this branch contains.

Installed and checked:

```
$ flatpak run app.kyerag.Kyerag --version
kyerag 0.1.0
$ xdg-mime query default video/x-insta360-insv
app.kyerag.Kyerag.desktop
$ flatpak run --command=sh app.kyerag.Kyerag -c 'ls /dev/dri'
by-path  card1  renderD128
```

and then run on real footage under a headless compositor, which is where
§3.2's `dmabuf import: all extensions enabled` and 30 fps came from.

The MIME package rides along: flatpak exported it to
`~/.local/share/flatpak/exports/share/mime/packages/app.kyerag.Kyerag.xml`,
so installing the Flatpak teaches the whole desktop what a `.insv` is. No
separate step.

Two defects the run found, both now fixed in the manifest and neither
visible without running it: saved state was silently discarded (§3.7), and
the first version of the manifest had no `~/.local/state/cosmic` grant to
discard it into.

Tooling installed to get here, all `--user`, no root and no system packages:
`org.flatpak.Builder`, `org.freedesktop.Sdk.Extension.rust-stable//25.08`,
`org.freedesktop.Sdk.Extension.llvm21//25.08`.

---

## 4. Publishing

### 4.1 Flathub is owner-only

AGENTS.md: "ALL work stays inside the owner's repositories. Never open,
file, or comment on issues or pull requests of any outside project, ever."
A Flathub submission is a pull request against `flathub/flathub`, and the
subsequent app lives in a repository under the `flathub` organisation. **No
agent may do any of it.** What follows is a description so the owner can
decide, not a plan for anyone else to execute.

The shape of it: a submission PR carries the manifest and the app ID; the
ID's domain has to be one the owner controls (§3.6); the metainfo has to
carry at least one screenshot; the build has to pass
`flatpak-builder-lint`, which today reports the three errors in §3.6.
Review is by humans and the permission set is the part they read hardest,
which is the argument for §3.7 being as small as it is.

Realistically the prerequisites are: pick the app ID, get an icon, take
screenshots, resolve the ffmpeg pin, and decide the X11 question. None of
those is a Flathub problem; they are all upstream of it.

### 4.2 A self-hosted repo is entirely ours

Everything Flathub does can be done from this repository, with no outside
account and no outside PR, because a flatpak remote is a static OSTree
repository over HTTP:

- `flatpak-builder --repo=<dir>` already writes one (§3.8 did).
- `flatpak build-update-repo --gpg-sign=<key>` generates the summary and
  signs it. Unsigned works for `--user` installs with
  `--no-gpg-verify`, which is fine for testing and not fine for a thing
  strangers install.
- The repository is a directory of static files. GitHub Pages serves it.
- Users add it with a `.flatpakrepo` file (an INI with `Url`, `Title`,
  `GPGKey`), then `flatpak install kyerag app.kyerag.Kyerag`.
- `flatpak build-bundle` produces the single `.flatpak` file that installs
  with no remote at all, which is what this branch produced and what the
  owner can click a `.insv` against today.

Three details that decide whether it works rather than whether it exists:

- **AppStream is not optional.** cosmic-store enumerates each remote and
  reads its appstream branch; no appstream data means Kyerag is installable
  from the remote and invisible in COSMIC Store, GNOME Software and Discover.
  `flatpak-builder --repo` passes `--update-appstream` for you; a hand-run
  `flatpak build-export` does not.
- **GitHub Pages caps a site at 1 GB**, and flatpak-builder exports a
  `.Debug` ref alongside the app. Ours is 12 MB of debuginfo against 31 MB
  of app today, which is fine, but `--prune --prune-depth=1` on
  `build-update-repo` is what keeps history from eating the budget, and
  static deltas roughly double the size in exchange for much faster installs.
- **A bundle can carry its own update path.** `build-bundle --repo-url=…`
  makes installing the single file configure the remote too, and
  `--runtime-repo=…flathub.flatpakrepo` means a user without the freedesktop
  runtime gets offered it instead of an error. `--gpg-sign` on
  `build-bundle` is a no-op for `.flatpak` files (it signs OCI images only);
  the key goes in with `--gpg-keys`. Omitting `GPGKey=` from a
  `.flatpakrepo` sets `gpg-verify=false` for the remote automatically, so
  users need no `--no-gpg-verify` incantation.

The trade is real: a self-hosted repo gives up discovery (nobody browses
it), and it puts update delivery and key management on us. Against that, it
needs no permission from anyone and can ship the day the ffmpeg question is
answered. The two are not exclusive; a self-hosted repo is a reasonable
beta channel whether or not Flathub ever happens.

---

## 5. What the owner has to decide or supply

1. **An icon.** Nothing else can produce one. Blocks: a non-generic launcher
   entry, the About page, and Flathub.
2. **The app ID.** `kyerag.app` does not resolve, so `app.kyerag.Kyerag` is
   not valid on Flathub today. Buy the domain, or move to
   `dev.harding.Kyerag` / `io.github.aeharding.Kyerag`. Cheapest now,
   expensive after the first release (§3.6).
3. **The ffmpeg pin**: bump to 7.1 and move the dev box, or bundle ffmpeg
   6.1 in the manifest (§3.4).
4. **Whether Flathub is a goal at all**, which decides whether the X11 and
   `xdg-config/cosmic:ro` questions matter (§3.6) and whether screenshots
   are needed.
5. **The licence spelling.** `res/…metainfo.xml` says `AGPL-3.0-only`
   because the repository says "AGPL-3.0" and carries no per-file "or any
   later version" grant. If "or later" was intended, that string and the
   file headers should say so before anything is published under it.
