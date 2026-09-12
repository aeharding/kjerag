# Research and oracle navigation

Research records explain where arithmetic and decisions came from. They are
dated evidence, not a declaration of what the current player runs or what is
left to ship. Start with [the current roadmap](../ROADMAP.md),
[architecture](../ARCHITECTURE.md) and [delivery qualification](../MERGE_READINESS.md).
The complete pre-release experiment chronology is preserved in
[roadmap history](../ROADMAP-HISTORY-20260912.md).

## What to read for a specific question

| Question | Primary local record |
| --- | --- |
| Current ONE X2 Studio-derived stitching evidence | [Stitching summary](studio-seam-re.md) |
| Original camera frame to Studio export-frame association | [ONE X2 video oracle](studio-video-oracle-602.json), [X4 reference](studio-x4-video-reference-602.json) |
| Photometric lens color matching and isolated Studio setting | [Spatial fusion](studio-image-fusion-spatial.md), [on/off oracle](studio-image-fusion-oracle-602.json), [map receipt](studio-image-fusion-maps-602.json) |
| Temporal filtering, motion and historical GPU experiments | [Temporal fusion](studio-image-fusion-temporal-602.md) |
| Camera ISO and effective denoise parameters | [ISO decoding](studio-denoise-iso-602.md), [parameter selection](studio-denoise-config-602.md) |
| Decoder/map ownership, submission and retirement invariants | [Root capture](gpu-root-capture-state-qualification.md), [drawable](gpu-drawable-install-qualification.md), [retirement](gpu-nonblocking-retirement-qualification.md), [draw retirement](gpu-draw-retirement-qualification.md) |
| Exact motion/map context and asynchronous validity | [Motion context](gpu-motion-context-qualification.md), [final map context](gpu-final-map-context-qualification.md), [validity gate](gpu-async-validity-gate-qualification.md) |
| Camera metadata/calibration fixtures | [INSV format](insv-format.md), [X4 Air fixture](x4air-calibration.json), [ONE X2 fixture](onex2-calibration.json), [OSV format](osv-format.md) |
| Original stack feasibility and rejected alternatives | [GPU feasibility](gpu-pipeline.md), [Linux landscape](linux-landscape.md) |
| Prior seam experiments and owner rejection evidence | [Seam blending](seam-blending.md), [temporal seam](seam-temporal.md), [two-axis work](seam-two-axis.md), [stage 9](stage9.md) |

## How to use the evidence

- Preserve the distinction between a binary-read mechanism, a captured Studio
  input/output pair, a CPU/GPU regression and an owner-reviewed moving result.
  None automatically proves the others or certifies every recording mode.
- Oracle manifests identify exact inputs, settings and hashes. Referenced
  personal footage and capture corpora remain local; a manifest is not a
  substitute for those bytes. Ordinary builds do not require private corpora.
- Keep readable arithmetic and its regression controls. A removed optimization
  experiment remains in Git history and its dated record; do not reactivate it
  because a historical benchmark looks promising.
- The current selected correction uses reduced-resolution fields, source-rate
  prefiltering, periodic horizontal sampling and a source-owned world chart.
  Older body/full-resolution captures remain references, not current defaults.
- Broader optimization reverse engineering remains frozen. The release target
  is owner-accepted Studio-like playback, not perfect internal reproduction.
  Stronger isolated photometric parity and unqualified camera/hardware coverage
  remain explicitly tracked rather than inferred from a good sample.

Do not remove or rewrite historical measurements to make them agree with the
current implementation. Update the living documents and add dated evidence.
