# Native ARM integration audit, 2026-09-06

Main baseline `ef80b57f`, Apple M4 Pro, Rust 1.98. No production source or
public API change. `zenravif` delegates AV1 encoding to pinned zenrav1e
`e4883037553434efb57ecbf4414c8b49922ba3e4`; there is no separate wrapper
arcane/magetypes hot loop to optimize.

`just arm-integration-audit` serializes strict pure-Rust all-target clippy,
default asm+threading tests, pure-Rust threading tests and `__expert,stop`
tests. All pass. The default and pure-Rust modes each run 49 runtime tests
plus seven doctests; four existing doctests remain ignored. Expert+stop runs
79 runtime tests plus seven doctests, with five existing ignored doctests.
The expert run includes all four frame-hint behavior tests, which are absent
from the default build by feature configuration.

Coverage includes RGBA/alpha, odd-sized 4:2:0 and grayscale, 10/12-bit output,
HDR CICP metadata, cancellation, pixel limits and live frame hints. This is
integration validation, not a new wrapper performance claim. Backend ARM
Rust/assembly measurements are in the zenrav1e audit.

Builds/tests used nice -n19 and four build/Rayon/OMP/test threads; no
`target-cpu=native`. The inherited CI comments claim a GitHub fork
acknowledgement explains missing push runs; that explanation is not established
by this audit. Current triggers and a dispatched run are checked directly.
