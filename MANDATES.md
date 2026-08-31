# MANDATES

Owner rulings for the seam/flow work. They override convenience and my own judgment.
**Read at the start of every session, and before every claim, merge, or "it's done".**
When a result and one of these conflict, the mandate wins.

## 1. Faithful to Studio at the video boundary — no exceptions
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
