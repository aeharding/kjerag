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

The pre-release hook first regenerates and checks `flatpak/cargo-sources.json`.
The generator needs the network. A workspace-version-only change normally
regenerates the identical source list; it still runs and is checked. If the
generated file differs from the reviewed commit, the hook stops before UI
testing or tagging. Review and land that change through a normal PR first.
Even a failed dry run can leave this generated diff in the checkout; inspect
it rather than assuming a dry run cannot write files. Regeneration, source
checking and UI testing were exercised by both the 0.3.0 dry run and execution
using a one-off config. The permanent hook, including its guard against an
unexpected generated diff, passed both the ordinary 0.3.1 dry run and execution.

`--execute` then bumps `[workspace.package] version`, refreshes `Cargo.lock`,
stamps a dated `<release>` entry at the top of the metainfo's changelog,
commits that as `release: 0.2.0`, tags it `0.2.0` (the plain version, no `v`),
and pushes both. Configuration is `release.toml`.

The tag is what builds the app. `.github/workflows/release.yml` runs the CI
gates, then Flatter builds and signs each architecture once on its native
x86_64 or aarch64 runner. Each passes its repository and signed source/version/
commit record through a separately named workflow artifact. The assembler
authenticates both, imports only their exact signed app and Debug commits,
generates the combined signed channel, and exports the two app-only bundles
from it. Cache contents and matrix execution order do not select release refs.

**Both routes use the same signed app commits** (issue #146). The complete
repository, bundles and checksums receive a signed publication manifest before
either destination is eligible. Publication jobs independently verify that
manifest against the signing job's public key and expected source/tag. Neither
publishing job needs a private key. The builder and assembler still need
`GPG_PRIVATE_KEY` and `GPG_PASSPHRASE`; a missing key or failed architecture
withholds the entire release, not an unsigned or single-architecture fallback.

GitHub receives `kjerag-0.2.0-x86_64.flatpak`,
`kjerag-0.2.0-aarch64.flatpak` and a `.flatpak.sha256` for each. The publisher
checks the remote tag's exact commit, creates a draft with generated notes,
uploads only missing files, and downloads all four to verify their bytes before
publishing. Existing differing assets are refused, never overwritten. Only a
successful Release job permits the signed channel deployment at
`https://kjerag.harding.dev/`. Those two destinations are not atomic: a Pages
failure leaves a valid Release available and the previous channel in place.

Before a release-workflow change is ready, exercise its native builds and
artifact handoffs without publication:

```sh
gh workflow run release.yml --ref <reviewed-working-branch>
```

This dispatch route requires a non-default branch, derives the version from
that checkout's `Cargo.toml`, and keeps disposable-signature caches off release
tags and the default branch. It generates one disposable validation key and
passes it to both builders and assembly as a separately named, one-day artifact.
That **test-only private key is intentionally disposable**, not a production
credential. It is never included in repository or bundle payloads. The two
production-secret import steps and both publishing jobs are tag-push-only.

The same build/stage/assemble/export/sign/verify path produces distinctly named
`validation-*` artifacts. A final read-only job downloads and authenticates the
combined payload. No release, Pages deployment or host installation occurs.
The generated bundles use a test signer and are not releases or replacements
for the installed app. Once the validation key artifact expires, rerun the
whole validation, not a mixture of old and new signed handoffs.

This run does not qualify production credentials, either public destination,
live HTTPS updates or playback. Manual dispatch availability on a newly edited
workflow must be checked on GitHub; do not create a temporary release tag to
work around an unavailable dispatch.

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

Run the same installed checks for the signed channel too, with
the first two lines replaced by `flatpak update dev.harding.Kjerag` if the app
already follows the signed public remote. Verify `flatpak info --show-origin`
and the remote URL first: a scratch test origin does not follow the public
channel. Record the installed OSTree commit and executable SHA256 for each
route, and require equality for a release produced by the single-build workflow.
Both install branch `stable`, so only one can be active per installation.

The signed bundle records the official channel URL. Installing a newer bundle
over the matching signed channel uses
`flatpak install --user --reinstall ./bundle.flatpak`; do not disable GPG
verification or uninstall first. Flatpak 1.14.6 refuses an **identical already
installed commit** with "already installed", even with `--reinstall`; 1.18.1
accepts it. If the verified intended commit is already installed, no replacement
is needed. This is not permission to ignore a different commit or another error.
Verify the retained origin, settings and subsequent channel update. The local
synthetic lifecycle test covers an older channel build, a different signed bundle,
the identical-bundle repeat, and a subsequent channel update. It exercises client
mechanics, not a live HTTPS deployment or real playback.
Keep the previous verified package available for rollback throughout.

Historical 0.3.1 and earlier downloads came from a separate unsigned build.
They do not gain signatures retroactively and cannot be used to prove the new
workflow's identity or reinstall contract.

Check the actual installed license payload on both routes:
`/app/share/licenses/dev.harding.Kjerag/kjerag/LICENSE` must match the source
`LICENSE`. The manifest installs it explicitly as of 0.3.1; automatic license
collection differed between the two builders in 0.3.0.

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

If a run fails, inspect its failed job and rerun failed jobs. Private handoffs
are kept for14 days. A failed build can reuse its successful sibling's artifact;
a failed Pages job reuses the already authenticated publication without another
build or Release upload. A partial draft resumes only when its existing files
match. Rebuilding the same tag is not permission to replace different published
bytes: use a new patch tag for a changed payload. If the staging artifact has
expired, do not reconstruct and silently replace an existing release.

Release and site-only deployments share one job-level lock covering checkout
and deployment. The channel's signed `kjerag-release.txt` marker rejects older
source history, non-increasing semantic versions and mismatched same-source
records after the first single-build release. Build metadata is not a version
increase. A site-only republish must preserve that record and its signature;
it cannot silently migrate the marker to a different signing key.
GitHub concurrency is not FIFO; a pending deployment can be canceled and need
a retry. The lock does not cover manual pushes or older workflow revisions.
Do not delete a published Release/tag as an automatic retry strategy: that
cannot roll back an already deployed channel.

One thing this does not do. It writes no prose: the changelog entry carries a
version and a date, and if a release deserves words in a software centre, add
a `<description>` to its entry in `resources/dev.harding.Kjerag.metainfo.xml`
and push it like any other commit.
