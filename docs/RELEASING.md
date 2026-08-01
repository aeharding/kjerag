# Cutting a release

What a person does, in order. Everything after the tag push is
`.github/workflows/release.yml`: it runs the gates on the tagged commit,
builds the Flatpak with flatpak-builder from `flatpak/dev.harding.Kjerag.yml`
and the committed `flatpak/cargo-sources.json`, and publishes a GitHub Release
carrying one installable `kjerag-<version>-x86_64.flatpak` and its
`.sha256`.

The version is one string in three places: `[workspace.package] version` in
`Cargo.toml` (the source of truth, and what `kjerag --version` prints), the
newest `<release>` in `resources/dev.harding.Kjerag.metainfo.xml` (the
changelog a software centre shows, and the GitHub Release body), and the tag.
`scripts/version-check.sh` is what says they agree, and the tag build stops on
its answer before it builds anything.

## 1. Bump and write the notes

In `Cargo.toml`:

```toml
[workspace.package]
version = "0.2.0"
```

In `resources/dev.harding.Kjerag.metainfo.xml`, a new entry at the **top** of
`<releases>`, newest first, with today's date:

```xml
<release version="0.2.0" type="development" date="2026-08-14">
  <description>
    <p>What changed, in plain words. No em dashes.</p>
  </description>
</release>
```

`<p>` and `<ul><li>` are what the release body renders; anything else is
refused by `scripts/version-check.sh --notes` rather than silently dropped.
Drop `type="development"` when the app is no longer pre-alpha.

Then `cargo check` once, so `Cargo.lock` records the new version for the
workspace's own crates. That does not touch `flatpak/cargo-sources.json`:
path crates carry no `source` entry, so they are not sources, and
`scripts/cargo-sources.sh --check` passes unchanged.

## 2. Check it here, including the one CI cannot

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/name-check.sh
scripts/cargo-sources.sh --check
scripts/version-check.sh v0.2.0
scripts/uitest.sh ~/Videos/<file>.insv
```

The last one is the reason this list exists. CI runs every other line, and it
cannot run that one: decode is VA-API against `/dev/dri/renderD128` and a
runner has no GPU, so the window, the keys and the frame path are checked on
this box or not at all.

## 3. Land the bump, then tag what landed

The bump goes to `main` through a pull request like any other change. Then,
on the merged commit:

```sh
git tag -m 'Kjerag 0.2.0' v0.2.0 && git push origin v0.2.0
```

That is the release. Watch it with `gh run watch` or the Actions tab. Measured
on the pipeline's own test tag: 10m44s end to end, nine minutes of it the
Flatpak build.

The `-m` is not decoration on this box: `tag.gpgsign` is on, a signed tag is
an annotated tag, and a bare `git tag v0.2.0` stops with `fatal: no tag
message?` before it makes anything.

## 4. Verify the artifact

```sh
gh release download v0.2.0
sha256sum -c kjerag-0.2.0-x86_64.flatpak.sha256
flatpak install --user ./kjerag-0.2.0-x86_64.flatpak
flatpak run dev.harding.Kjerag --version
```

## If it goes wrong

A tag is cheap to withdraw before anybody has it:

```sh
gh release delete v0.2.0 --yes --cleanup-tag
```

Fix, land the fix, tag again. Re-pushing a tag to a different commit is not
the move: the tag is what a downloaded bundle claims to be.

To exercise the pipeline without spending a version number, tag a prerelease
of the version the tree already carries:

```sh
git tag -m 'Pipeline test' v0.2.0-rc1 && git push origin v0.2.0-rc1
```

The version in front of the dash still has to be the workspace's, the build is
the same build, and GitHub marks the release a prerelease.

## What this does not do

Flathub. The submission is owner-led and owner-coordinated
(docs/DISTRIBUTION.md 4.1); this pipeline publishes to this repository's own
Releases and nowhere else.
