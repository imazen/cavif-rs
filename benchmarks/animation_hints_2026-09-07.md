# Animation quantizer hints — 2026-09-07

Base owner: `ecbf41ec42f05568e0f36074edc7f946ebdb12bf`.
Backend: `1d5a6e04e80216b1063faa3baa7cbb2189d132f3`.

The animation send loop previously submitted only a frame, dropping the
configured expert superblock quantizer map. It now attaches FrameParameters
to every color submission. Alpha receives default parameters, matching the
still-image policy. The same configured map is submitted for each frame;
this does not add an API for different maps per frame. Backend application
remains limited to non-lossless KEY/intra-only frames; inter support remains
unfinished. Changes in later packets alone do not prove inter hint support,
because altered reference frames can also change later packets.

The regression uses two 129×67 frames at 8/10 bits, color/alpha, and no map,
a neutral six-entry map, or alternating 0.5/2.0 scales. It failed on the old
send loop at depth 8/frame 0. After correction both color packets differ;
neutral maps and alpha packets remain byte-identical. The all-feature suite
passes 97 tests (five existing ignored doctests), and no-default-features
passes 58 (four existing ignored doctests). All-target Clippy passes after
correcting a test initializer warning; no lints or assertions were disabled.

`ZENRAVIF_HINT_ARTIFACTS` lets the caller export all 12 raw streams.
`scripts/verify_animation_hints.py` decodes them independently with libaom,
requires exactly two frames at the expected dimensions/depth, and checks
neutral-map/alpha pixel equality and non-neutral color pixel differences.
All 12 streams / 24 frames pass. Hashes are in the matching TSV. This is a
wiring/conformance check, not an RD improvement claim.

```sh
cargo test -p zenravif --all-features
cargo test -p zenravif --no-default-features
cargo clippy -p zenravif --all-features --all-targets -- -D warnings
python3 scripts/verify_animation_hints.py "$HOME/tmp/slower-preset-probe/animation-hints" --manifest benchmarks/animation_hints_2026-09-07.tsv
```

All heavy commands use the shared 16G/four-job wrapper with RUST_TEST_THREADS=4.
Logs and raw streams: `~/tmp/slower-preset-probe/animation-hints*`.
The canonical test separately fails on the previous published owner because
non-neutral hints produce an identical AVIF. Its integration is validated in
zenavif. No quality envelope, floor or skip changed; wrapper main remains
pending the independent quality investigation.
