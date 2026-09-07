# MANDATES

Owner rulings for the seam/flow work. They override convenience and my own judgment.
**Read at the start of every session, and before every claim, merge, or "it's done".**
When a result and one of these conflict, the mandate wins.

## Current goal, owner clarification 2026-09-05

"our end goal is very performant stitching that is studio-like (doesnt need to be perfect)"

This supersedes the earlier requirement for perfect Studio reproduction below.
Prioritize smooth playback, responsive seeking and Studio-like visible stitching.
Numerical identity is a useful diagnostic, not a shipping requirement. Simpler
arithmetic, scheduling and temporal restart choices may be evaluated against the
existing oracle without reverse engineering every difference. Do not silently
accept visible defects or claim an unreviewed tradeoff is approved: real rendered
sequences, the reported riser defect and the owner's eye remain the quality gate.
Keep the readable reference implementation and existing evidence for comparison.
Broader optimization reverse engineering remains frozen.

### Shared product direction, owner approval 2026-09-05

The owner approved taking product ownership and generalizing the stitcher:
one shared GPU engine with camera-specific calibration/geometry inputs, not
a separate implementation project for every camera. Prioritize usable,
zero-config playback, a stable horizon, responsive seeking and the capacity
target below. Existing architecture may be replaced where it obstructs that
result. ONE X2 and the owner's X4 Air footage must both be regression inputs;
success on one camera must not be reported as success on all supported files.
The coordinator owns prioritization and verification. Owner-visible tradeoffs
and branch acceptance remain the owner's decisions before merge.

### Seam-preserving performance work, owner clarification 2026-09-07

The owner rejected staggering the two lens directions' full refreshes if it
affects the seam: "if its going to affect the seam then we need to figure out
a better way", and added "also consider rearchitecutre". The staggered-cadence
prototype was removed before any build or playback. Pursue execution and
architecture changes that preserve the existing seam result and source-frame
refresh cadence; do not silently trade temporal stitching behavior for speed.
This does not reinstate the superseded requirement for exact Studio internals.
The owner also clarified that keeping the installed Flatpak untouched is not
a requirement. Deliver qualified branch builds through the installed app;
this does not waive sandbox verification or owner acceptance before merge.

The owner additionally requires "chromatic calibration w parity to Studio's"
when appropriate. Studio's term means photometric matching of the two lenses'
color and brightness, represented in its project as `image_fusion`; it does
not mean optical chromatic-aberration correction or per-channel lens
displacement. `docs/research/linux-landscape.md` section 6 records the maker
SDK's matching `EnableStitchFusion` description. The exact Studio 6.0.2 Flow
On and Flow Off project blobs sealed by
`docs/research/studio-video-oracle-602.json` both have Image Fusion enabled, as
authenticated by a read-only inspection of those manifest-named projects. They
therefore do not isolate Image Fusion's effect, coefficients or parity.
After the shared camera stitcher is working, add this photometric lens-fusion
work at the shared source-sampling/fusion boundary and verify it against the
relevant Studio setting and real output. It is required follow-on work, not a
completed feature. Do not invent Studio coefficients, build speculative
scaffolding, reopen unrelated reverse engineering, or delay the current X4 Air
stitching deliverable for it. A new photometric-isolation export is not part
of this stitching deliverable. This does not exclude ordinary Studio output
verification of the owner's reported X4 stitching defect.

### Player performance target, owner clarification 2026-09-05

"we need to do at least 240fps on this computer", clarified as "when I play
back in player", not an isolated stitching-kernel benchmark. Target responsive
240 fps view rendering during actual playback on this machine (about 4.17 ms
per display frame), while presenting source video at its recorded cadence and
keeping audio synchronized. A 29.970 fps source does not contain 240 distinct
video frames per second; do not claim repeated pictures as faster decoding or
invent interpolation as a requirement. Reuse each completed source frame's
stitch result across view redraws. Report output resolution, display refresh
limits and frame-time spikes as well as throughput. Isolated measurements guide
optimization but cannot establish this actual-player target. Full-rate 30 fps
video playback is necessary for this clip, not sufficient for completion.

Read-only display inspection found the active internal panel at 2256x1504,
59.999 Hz, with no advertised 240 Hz mode. The coordinator disclosed this to
the owner. Keep the 4.17 ms rendering-capacity target, but do not promise 240
distinct visible updates on a 60 Hz panel or busy-redraw an idle view solely
to inflate an fps counter. Smooth native presentation must use the display's
actual cadence. This hardware limit does not excuse video or audio stalls.

The owner then explicitly clarified "240fps regardless of display", "capacity",
and confirmed sustained rendering capacity with stitching active, not just
paused-view rendering. Display refresh is therefore not a blocker or an excuse
to lower this performance target. Measure uncapped capacity and frame-time
spikes during actual source playback; keep the 4.17 ms rendered-frame budget.

## 1. Earlier exact-parity standard (superseded where inconsistent above)
For the same supported input, view, time and user-visible settings, correctness with Studio is
required at the stitched video result, not as identity of internal execution. Every
output-affecting semantic mechanism, constant, threshold, kernel, boundary condition and cadence
in a Studio-equivalent path is one of: (a) **READ** from the Studio binary, (b) owner-synced, or
(c) a **DISCLOSED** live knob. Never invented. Never tuned to hit a target number. No "faithful in
spirit."

Internal scheduling, data layout, caching, vectorization, pass fusion and other implementation or
optimization choices may differ, and Studio's own optimizations may be reused, when the difference
is disclosed and the resulting video is verified through the computed seam trace, real rendered
sequences and the owner's eye. This permission does not authorize clip-specific fitting, unread
semantic inputs, configuration rituals or treating one good frame as proof of video parity.

## 2. Complete the RE before building on it
No seam code stands on an **approximated or UNREAD Studio-derived or output-semantic foundation**.
If a value is blocked (e.g. RELR
relocations in the Android lib), get a tool that reads it (Ghidra; or the Windows DLL) and
**READ it**. Do not approximate-and-ship. Do not report a "milestone" that rests on a hole —
name the hole first, close it, then build.

## 3. The owner's eye is the merge gate
Never call something "clean", "fixed", "harmless", "inert", or "parity" from a metric, a
median, or any pooled statistic. **VIEW the actual output pixels**, and **SEND them to the
owner** (my tool-views do NOT reach him — use SendUserFile). When a measurement disagrees with
the owner's eye, the **instrument is wrong** — fix the instrument, don't defend the number.

## 4. Verdicts on the seam trace, not pooled stats
Judge a seam defect on the computed seam trace + the real render (with decoy nulls). Never on
band-vs-surround averages: antisymmetric corrections cancel in symmetric stats, and local
spikes hide under a small median (this is how "harmless" flow smeared the helmet).

## 5. Parity is priority #1
`seam=factory` is the parity base. Non-parity mechanisms (`Seam::File` / `seam=pool`) are
removed **with** their parity replacement, not before.

## 6. Report faithfully; no over-claiming
State the real state — failures, skipped steps, uncertainty — plainly. Don't dress up a broken
result. Don't pinpoint what you haven't actually located. Never cite my own capacity or context
budget as a reason to stop.

## 7. Hygiene
Commits authored `noreply@harding.dev` (verify `%ae` after each). Subagents that write scratch
use a **worktree**, never the repo tree. Refused/retired code is deleted (git is the archive);
instruments and the RE record (`docs/research/studio-seam-re.md`) stay.
