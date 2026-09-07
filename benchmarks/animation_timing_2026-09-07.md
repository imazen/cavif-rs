# Exact animation timing — 2026-09-07

Base owner: `5b7c50b95bc5071bf29a46b0b5e3a6b2801f07d1`.
Backend: `1d5a6e04e80216b1063faa3baa7cbb2189d132f3`.

`TimedAnimFrame` and the four `encode_animation_*_timed` entry points accept
positive u32 durations in a caller-provided positive u32 timescale. RGB/RGBA
8-bit and native 10-bit inputs share the existing coding, alpha, metadata,
thread and cancellation paths. Legacy millisecond entry points adapt to
1000 ticks/second and retain one deadline across adaptation and encoding.
`EncodedAnimation` adds exact `timescale` and `total_duration_ticks` fields;
its existing millisecond total rounds down for non-millisecond input.

Container media headers and sample timing tables preserve exact integers.
Totals use checked u64 addition; conversion to the compatibility millisecond
field uses u128 before checked narrowing. Zero clocks and durations fail.
The serializer retains responsibility for repeated-duration overflow checks.

Encoder cadence and container timing are distinct. The initial attempt used
one container tick as the encoder frame interval and failed at the microsecond
clock: zenrav1e enforces a 65536-fps safety limit. The corrected code uses the
shortest frame interval as the nominal coding cadence, capped at that backend
limit, in both encoder and sequence-header contexts. AV1 timing-info is
disabled; mdhd/stts retain exact presentation timing even above that nominal
rate. The historical 1000-fps nominal cadence is retained for millisecond
calls to preserve their coding behavior. This is not a claim of realtime
encoding throughput or unrestricted AV1 level conformance at extreme rates.

`ravif/tests/animation_timing.rs` exercises all four input types at clocks
30000 (durations 1001/2002), 1000000 (1/7), u32::MAX (two maximum durations),
and 1000 (20/30): 16 complete files, including alpha and totals above u32.
It checks every media header and expanded stts table, exact result fields,
all four legacy/timed millisecond byte identities, and invalid inputs.
A mutation restoring the muxer's default 1000 clock fails with 1000 vs 30000;
restoring the timescale setter passes. The test is not just checking results
against their own returned metadata.

`scripts/verify_animation_timing.py` independently decodes every frame with
libavif (one worker), asserting exact integer timescale, PTS and duration.
All 16 files / 32 frames pass, including the one-microsecond arm. Hashes are
in the matching TSV. Detailed libavif output is beside each retained AVIF.

All-feature tests pass 98 (five existing ignored doctests); no-default-feature
tests pass 59 (four existing ignored doctests). All-target Clippy passes after
correcting argument grouping and test-only lint findings. No assertions,
thresholds or skips were relaxed. Commands run through the shared 16G/four-job
wrapper with TMPDIR=$HOME/tmp and RUST_TEST_THREADS=4:

```sh
cargo test -p zenravif --all-features
cargo test -p zenravif --no-default-features
cargo clippy -p zenravif --all-features --all-targets -- -D warnings
cargo test -p zenravif --all-features --test animation_timing
python3 scripts/verify_animation_timing.py "$HOME/tmp/slower-preset-probe/animation-timing" --manifest benchmarks/animation_timing_2026-09-07.tsv
```

Set ZENRAVIF_TIMING_ARTIFACTS to export the fixture files. Logs and artifacts:
`~/tmp/slower-preset-probe/animation-timing*`. Exact timing still needs the
canonical zenavif public and codec-adapter integration; this owner API is
not completion of the full animation objective. Wrapper main remains held
by the separate quality investigation.
