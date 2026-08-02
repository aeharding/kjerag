# Cutting a release

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

The same check answers for the channel, with the first two lines replaced by
`flatpak update dev.harding.Kjerag`. Both install branch `stable` and there is
only ever one of them on a machine, so whichever route the build arrived by,
`flatpak run dev.harding.Kjerag` is the same app.

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
