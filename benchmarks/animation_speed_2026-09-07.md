# Animation speed override forwarding — 2026-09-07

Base owner revision: `b829a9ee`.

Animation's shared speed derivation now applies the eight remaining explicit
speed controls already consumed by still color encoding: transform RDO,
SGR complexity, restoration-on-skip, segmentation complexity, bottom-up
partition search, partition size range, prediction-mode complexity and fast
deblocking. Explicit prediction modes clear the speed-6–8 top-seven override
so the preset cannot overwrite the caller's choice. Color settings do not
change alpha's independent preset policy. CDEF/restoration forwarding from
the preceding change remains in place.

`animation_partition_override_changes_color_packets` encodes two 65×67 frames
at both 8-bit and native 10-bit input. Before the correction, forcing a 4×4
partition range produces identical AVIF bytes and fails the regression. After
the correction, the files differ at both depths. The configuration test checks
both Boolean values at speeds 1/6/8/10, the explicit partition range, final
backend speed settings, and unchanged alpha configuration. This configuration
test is complementary to the packet regression, not evidence that every
search option changes output on every image.

All-feature owner validation passes 94 tests (five existing ignored doctests),
no-default-feature validation passes 58 (four existing ignored doctests), and
all-feature/all-target clippy passes with warnings denied. Shared run-heavy
limits are 16G/four jobs, with RUST_TEST_THREADS=4. Logs are retained under
`~/tmp/slower-preset-probe/animation-speed-*.log`.

VAQ, segmentation boost, trellis, tune, lossless, per-frame hints, pixel-format
options and exact public timing remain separate wiring work. No quality floor,
envelope, or skip was changed; the wrapper quality audit remains open.
