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
action, and publishes a Release carrying `kjerag-0.2.0-x86_64.flatpak` and its
`.sha256`, with notes GitHub generates from what merged since the last tag.
About ten minutes. Then check what shipped:

```sh
gh release download 0.2.0
flatpak install --user ./kjerag-0.2.0-x86_64.flatpak
flatpak run dev.harding.Kjerag
```

If the tag run fails, take the tag back, fix, and tag again:

```sh
gh release delete 0.2.0 --yes --cleanup-tag
```

Two things this does not do. It writes no prose: the changelog entry carries a
version and a date, and if a release deserves words in a software centre, add
a `<description>` to its entry in `resources/dev.harding.Kjerag.metainfo.xml`
and push it like any other commit. And it does not submit to Flathub, which is
owner-led and owner-coordinated (docs/DISTRIBUTION.md 4.1).
