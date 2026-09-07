# Animation color formats and range

Animation previously hardcoded full-range 4:2:0 BT.601, even when the
encoder requested 4:4:4, RGB identity, or limited range. Both the actual
encoding context and the sequence-header context now consume those
settings. Pixel preparation, quantizer selection, AV1 profile/subsampling
flags, and container color metadata use the same selected format.

The default 4:4:4 setting is now honored, so default animation bytes
change. Explicit 4:2:0/full-range retains its existing conversion
arithmetic. Full-range RGB uses exact G/B/R planes. RGB identity requires
full-range 4:4:4 and rejects incompatible combinations. Limited-range
YCbCr maps quantized full-range planes to studio-range code values;
YCbCr conversion/subsampling is not source-RGB lossless. Alpha remains
full-range monochrome and uses its independent coding policy.

The existing public animation partition-override test exposed a backend
panic once 4:4:4 was honored. zenrav1e `1447c200` fixes that partition
guard and two independently reproduced chroma-edge distortion errors.
Both host and wasm dependency declarations pin that revision. Its
source-exact regression covers 144 configurations / 288 frames against
rav1d-safe and libaom. See that repository's
`benchmarks/sub8_inter_2026-09-07.md`.

`ravif/tests/animation_color.rs` produces 20 two-frame files: RGB8,
RGBA8, native RGB10 and RGBA10, each with 420/full, 420/limited,
444/full, 444/limited and RGB identity/full. It checks the container
profile, depth, chroma flags, matrix, range and frame count.
`scripts/verify_animation_color.py` checks all 20 files with libavif,
decodes all 40 color frames with libaom, and requires the four RGB
streams to equal their original G/B/R samples exactly. The verified
hashes are in `animation_color_2026-09-07.tsv`.

With the local backend fix, the full suite passed 99 tests (five
existing ignored doctests); minimal features passed 59 (four existing
ignored doctests). Clippy found four redundant u16 conversions, now
removed. Against the published backend revision, the color and partition
regressions pass, all 20 files pass independent verification, and clippy
is clean. Restoring the previous color wiring makes the new regression
fail on the limited-range flag; restoring the fix passes. Artifacts and logs are retained under
`~/tmp/slower-preset-probe/animation-color-*`.
