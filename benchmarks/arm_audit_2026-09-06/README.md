# Native ARM integration audit, 2026-09-06

Main baseline `ef80b57f`, Apple M4 Pro, Rust 1.98. `zenravif` delegates AV1
encoding to zenrav1e; there is no separate wrapper arcane/magetypes hot loop
to optimize. The initial integration audit uses backend `e4883037`.

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
by this audit. The audit push triggered CI automatically; run [34068104713](https://github.com/imazen/cavif-rs/actions/runs/34068104713) passed.

## Audited backend integration

Both native and WASM pins now select `605946821afa839ca80f2b9bb226917238e9dba3`,
the backend audited on ARM. Reviewing the complete source delta from `e4883037`
identified existing fixes in `src/encoder.rs` and `src/partition.rs`: joint
chroma prediction for 4x16/16x4 inter blocks and top-right motion-vector
availability for four-way rectangular partitions. These fixes predate this
audit and are not attributed to it. The backend version remains 0.2.0; no
wrapper public API changes. Cargo update changes only the backend git revision
in the ignored local lockfile.

The full integration recipe passes again against the updated backend: strict
all-target pure-Rust clippy; 49 runtime + seven doctests in both default and
pure-Rust modes; 79 runtime + seven doctests with expert/stop. Existing ignored
doctests remain unchanged. See [full log](updated-backend-integration.log).
