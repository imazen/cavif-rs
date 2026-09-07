# Fast-preset mode-budget correction — 2026-09-07

The forced one-candidate intra-mode budget at speeds 9/10 was fitted under
Tune::Ssimulacra2 plus palette, a configuration the shipped wrapper did not use.
Its later as-shipped RD claims were invalidated by an ARM decoder bug. A new
x86 audit now demonstrates a concrete quality defect: the speed-10 photo/q35
engineering-gate cell was 1165 bytes / SSIM2 53.312, compared with the original
1064 / 57.477. Restoring the preset's full mode budget yields 1016 / 59.913.
The matching-feature consumer regression fails before and passes after, at the
original 0.5-score / 2%-size tolerances. No existing test was weakened.

The correction retains the CDEF and transform-domain settings, the backend's
explicit mode-budget knob, and its SpeedTweaks application. No C feature or
coding implementation was removed. The historical SATD-only table is annotated
as such; its old fit percentages do not describe the corrected default.

Verification: every one of 35 source images (25 GB82 photos, 10 GB82-SC screen
PNGs) improves its matched-quality rate curve at both affected speeds. Inputs
are explicitly white-composited 8-bit RGB and downsampled to at most a 512-pixel
long edge with triangle filtering. Both arms have byte-identical normalized
source pixels; decoder and fast-ssim2 0.8.2 are fixed. Nine quality points per
image and speed produce 315 cells per arm at each speed. Piecewise-linear
log-rate integration on Pareto fronts, over their common SSIM2 range clipped
to [30,90], gives:

| Speed | Median rate change | Range | Median encode-time ratio |
|---|---:|---:|---:|
| 10 | -3.332% | -8.635% to -0.630% | 1.233 |
| 9 | -5.202% | -8.809% to -2.044% | 1.493 |

Timing is a single-pass comparison, not a stable latency bound. This is not HDR,
alpha, full-resolution, or a cubic/PCHIP BD-rate result. The search cost increase
is substantial, especially at speed 9, and is not hidden by a benchmark re-pin.

Canonical evidence and reproducible probe/analysis source live in
`zenavif/benchmarks/quality_drift_2026-09-07/` on the shared review branch.
The audit additionally reproduces all 27 original baseline file sizes using
owner adb88ddc + registry zenrav1e 0.1.4, and separates four preset switches
from backend changes. The backend-only comparison is byte-identical to the
four-switch-disabled diagnostic in all 27 files. No production switches were
disabled. Parent/child probes establish added search cost in transform-type
and SPLIT estimation changes, with identical output for one timing witness;
that is not justification to revert correctness/search work for other inputs.

Independent libavif 1.3.0 successfully decodes all 1381 captured files across
these experiments. Owner all-feature tests/doctests pass 86 (five existing
ignored doctests). Canonical all-feature nextest passes 898/898 (nine existing
skips). Full terminal logs are under `~/tmp/quality-drift-probe`.
Owner all-target clippy and canonical all-feature library clippy pass with
warnings denied. Determinism passes 25 legs, reference conformance 56 cells
(optional armed CLI unrun). The ladder reports 52 failures: 32 non-timing
changes plus 20 timing misses. Its symmetric bounds also flag the corrected
photo's smaller size and higher score; this count is not a count of quality
regressions. Monotone now reports six inversions: the two pre-existing screen
cases plus four photo cases where improved speed 10 dominates slower presets
(s6/s7/s8 at q40, s5 at q80). Those slower-preset gaps remain unresolved.
No threshold/envelope changes, main merge, or CI run is part of this correction.
