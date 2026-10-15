# Animation coding controls and lossless verification — 2026-09-07

Base owner: `a8eaf997`. Backend correction: zenrav1e
`1d5a6e04e80216b1063faa3baa7cbb2189d132f3` (previously `60594682`).

Animation now forwards VAQ enable/strength, segmentation boost, trellis and
StillImage/Psychovisual tune to its color context. Lossless sets both base and
minimum quantizer to zero on color and alpha. Alpha retains its independent
perceptual-tool policy. Both context-construction paths use the same helper.

The before test fails because a lossless request changes no packets. The
configuration test checks all forwarded values and the alpha policy; the
packet test exercises baseline, lossless, tuned and lossless+tuned at both
depths for color and alpha, with two frames in each stream. Color controls
leave the alpha packets identical; lossless affects both tracks.

The combined test exposed an existing backend defect: trellis optimized
lossless WHT coefficients, producing changed decoded samples at quantizer
zero. Disabling VAQ did not repair it; disabling only trellis did. VAQ is
already disabled by the backend's quantizer-zero segmentation guard. The
backend fix returns lossless WHT coefficients unchanged at the optimizer
entry, protecting every caller. Its source-exact regression passes 72
configurations / 108 frames at 8/10/12-bit, mono/420/444, still/sequence,
Psychovisual/StillImage and trellis off/on. Its 225-test suite, clippy,
81+360-cell identity gate and 54-cell two-decoder reconstruction gate pass.

With the published backend fix and trellis restored to ON, independent
libaom decodes all 16 owner streams / 32 frames. All eight lossless streams
match their generated source planes exactly, including native 10-bit low
bits, alpha and the combined tuning arm. Hashes are in
`animation_coding_2026-09-07.tsv`. The separately named trellis-off manifest
records only the diagnostic isolation arm, not the final fix.

Reproduce the external check after exporting the fixture via
ZENRAVIF_CODING_ARTIFACTS and the animation_coding unit test:
`python3 scripts/verify_animation_coding.py <artifacts> --decoder /usr/bin/aomdec --manifest <manifest.tsv>`.
All heavy work uses run-heavy (16G/four jobs) and RUST_TEST_THREADS=4.
Logs: `~/tmp/slower-preset-probe/animation-coding-*.log`; final raw streams and
source/decoded planes: `~/tmp/slower-preset-probe/animation-coding-backend-fixed/`.

This establishes lossless coding of the input AV1 planes, not lossless RGB:
animation still converts color to 4:2:0 BT.601. Pixel-format options,
per-frame hints, exact public timing and broader metadata/track support remain
open. No quality threshold, envelope or test skip was changed.
Published-backend owner validation passes 96 all-feature tests (five existing
ignored doctests), 58 no-default-feature tests (four existing ignored doctests),
and all-feature/all-target clippy with warnings denied.
