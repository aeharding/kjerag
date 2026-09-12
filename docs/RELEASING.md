# Cutting a release

CI's compiler, formatter and Clippy are pinned together to Rust 1.97.1 in
`.github/workflows/ci.yml`, matching the qualified native and 25.08 SDK builds.
This is the verification toolchain, not a change to the package's minimum
Rust version. Update the pin deliberately and run the same gates locally:
moving `stable` introduced a new denied lint in existing parser code on
2026-09-07 while the qualified 1.97.1 build still passed. That observation
does not qualify newer Clippy versions or change the playback executable.

Once per machine:

```sh
cargo install cargo-release   # 1.1.3 is what this was set up against
```

Then, on `main` with a clean tree:

```sh
cargo release patch             # dry run, which is cargo-release's default
cargo release patch --execute   # or minor, major, or a version: 0.2.0
```

The dry run is not a preview. It runs `scripts/uitest.sh`, the headless GPU
harness CI has no device for, so a build that would not open a window cannot
reach a tag. Give it an idle box, and set
`KJERAG_TEST_MEDIA=~/Videos/<file>.insv` to include the playback checks.

The pre-release hook first regenerates and checks `flatpak/cargo-sources.json`,
so every Cargo.lock change carries matching offline sources in the release
commit. The generator needs the network. A workspace-version-only change
normally regenerates the identical source list; it still runs and is checked.
This combined hook was exercised by both the 0.3.0 dry run and execution using
a one-off config before being made the default here.

`--execute` then bumps `[workspace.package] version`, refreshes `Cargo.lock`,
stamps a dated `<release>` entry at the top of the metainfo's changelog,
commits that as `release: 0.2.0`, tags it `0.2.0` (the plain version, no `v`),
and pushes both. Configuration is `release.toml`.

The tag is what builds the app. `.github/workflows/release.yml` runs the CI
gates on the tagged commit, builds the Flatpak with Flatpak's own GitHub
action on an x86_64 and an aarch64 runner, and publishes a Release carrying
`kjerag-0.2.0-x86_64.flatpak`, `kjerag-0.2.0-aarch64.flatpak` and a `.sha256`
for each, with notes GitHub generates from what merged since the last tag.
About ten minutes; the two builds run side by side.

**The same tag publishes the channel** (issue #137, docs/DISTRIBUTION.md 4.3).
A second pair of builds is GPG signed and exported into the OSTree repository
at `https://kjerag.harding.dev/`, which is where an installed Kjerag gets this
version from and where a new one is installed from with a click. That half
builds one arch at a time, because both write into one repository, so a tag is
nearer twenty-five minutes end to end than ten. It needs the `GPG_PRIVATE_KEY`
and `GPG_PASSPHRASE` repository secrets; without them that job fails and the
Release is published anyway, which is the right way round.

**A description is not a release.** What a software centre reads about the
channel is written by `scripts/pages-site.sh`, and the `site` workflow runs it
against the published repository on a dispatch, so a fixed summary, a new icon
or a renamed descriptor ships in a minute with nothing rebuilt
(docs/DISTRIBUTION.md 4.5):

```sh
gh workflow run site.yml --ref main
```

The app's own page in a Store is the exception: that data lives inside the
built commit and moves at the next tag.

Then check what shipped, which is not the same question as whether it builds:

```sh
gh release download 0.2.0
flatpak install --user ./kjerag-0.2.0-x86_64.flatpak
KJERAG_FLATPAK=dev.harding.Kjerag scripts/uitest.sh ~/Videos/<file>.insv
```

The signed channel is a separate build, not necessarily the same executable
as the GitHub bundle. Run the same installed checks for that route too, with
the first two lines replaced by `flatpak update dev.harding.Kjerag` if the app
already follows the signed public remote. Verify `flatpak info --show-origin`
and the remote URL first: a scratch test origin does not follow the public
channel. Record the installed OSTree commit and executable SHA256 for each
route. Both install branch `stable`, so only one can be active per installation.

That last line is the release check. The dry run above proved a **binary**
opens a window on this box; this proves the **bundle** plays real footage
inside the sandbox, where the Mesa, the ffmpeg and the libva are the runtime's
and not this machine's, and where the file arrives the way flatpak hands one
over. 0.1.1 shipped with nothing between those two: it was installed, started
by hand, and seen to draw. Starting is not playing, and the sandbox is where
the frame path is least like the one the dry run tested.

Only the x86_64 half is ever checked that way. The aarch64 bundle is compiled
and unit tested by CI and run by nobody: no GPU on a runner, and no aarch64
machine on this end (README).

Verify both public architecture-specific app and AppStream refs, but do not
report that as an ARM install/playback test. `flatpak remote-info --arch=aarch64`
can query the foreign app; AppStream update on an x86-only host is not a valid
substitute for testing with a supported aarch64 client. Preserve any failed
attempt with that qualification rather than calling the publication broken.

Functional UI qualification does not establish the active-playback capacity
target. Keep source cadence and completion-time spikes beside redraw throughput;
over 240 redraws/sec while a 29.97 fps source falls behind is not a pass.
Record current failures even if the same binary passed an earlier cohort.

If the tag run fails, take the tag back, fix, and tag again:

```sh
gh release delete 0.2.0 --yes --cleanup-tag
```

That takes the Release and the tag back. It does not take the repository back,
because the deploy already happened: fix, tag the next patch version, and the
next deploy replaces it.

One thing this does not do. It writes no prose: the changelog entry carries a
version and a date, and if a release deserves words in a software centre, add
a `<description>` to its entry in `resources/dev.harding.Kjerag.metainfo.xml`
and push it like any other commit.
