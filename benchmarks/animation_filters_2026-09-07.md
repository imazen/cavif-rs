# Animation CDEF and restoration forwarding — 2026-09-07

Base owner revision: `939fa5a3`. Animation ignored explicit CDEF and loop
restoration settings. Both sequence encoding and container configuration now
use `Encoder::animation_speed`, which applies those two color overrides after
the preset. Alpha keeps its independent preset policy, matching still encoding.
Default settings and the still encoder are unchanged.

The packet-level test encodes two textured 65×67 frames at 8 and 10 bits,
with both filters off, CDEF only, restoration only, and both on. Each individual
override changes the color packets; toggling both leaves alpha packets identical.
Before the fix the corrected row-based fixture fails the CDEF assertion.
Removing only restoration forwarding after the fix fails its own assertion.
Restoring forwarding passes. The initial diagnostic fixture wrote through
plane storage rather than its visible row origin; only the corrected
`animation-filters-red-rows.log` is the final fixture's before evidence.

Owner all-feature tests pass 92 (five existing ignored doctests), and
no-default-feature tests pass 58 (four existing ignored doctests). Clippy first
rejected a test-only default-field reassignment; replacing it with an initializer
passes all-feature/all-target clippy. The final focused test passes again after
the mutation is restored. No expectations or skips were changed.

Independent `/usr/bin/aomdec` accepts all ten exported streams and produces
exactly two frames of the expected geometry and sample depth for each.
This proves decodability, not encoder-reconstruction equality. The first decode
script incorrectly expected chroma output for monochrome; libaom correctly
writes only luma. The corrected plane-count check passes all ten streams.
Raw streams and decoded planes: `~/tmp/slower-preset-probe/animation-filters/`.
Logs: `~/tmp/slower-preset-probe/animation-filters-*.log` (the independent final
result is `animation-filters-decode-verified.log`). Tests are serialized through
run-heavy with 16G, four jobs and RUST_TEST_THREADS=4.

Remaining option wiring includes other speed overrides, VAQ/trellis/tune,
lossless behavior, chroma/range/color-model selection and per-frame hints.
Exact non-millisecond public timing and broader metadata/track support also
remain open. This correction does not resolve the separate quality-gate audit.
