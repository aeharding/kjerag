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
