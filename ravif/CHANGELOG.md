# Changelog

## [Unreleased]

## [0.1.3] - 2026-04-30

### Added
- 4 new `Encoder` builder methods exposing speed-preset overrides for
  content-dependent internal knobs:
  - `with_partition_range(Option<(u8, u8)>)` — block size range
  - `with_complex_prediction_modes(Option<bool>)` — ComplexAll vs Simple
  - `with_lrf(Option<bool>)` — loop restoration filter
  - `with_fast_deblock(Option<bool>)` — fast vs full deblock
  All gated on `imazen` feature like the existing `with_cdef` /
  `with_rdo_tx_decision` overrides. Wires through into `SpeedTweaks`
  before encode; `None` keeps the speed preset's default. Used by the
  zenavif rav1e knob predictor MLP training harness.

## [0.1.2] - 2026-04-27

### Changed
- Bump `zenrav1e` minimum version to 0.1.4 to pull in the QM level-mapping
  and AV1 lossless-conformance fixes (zenrav1e#7). Without these, AVIF
  encodes with `with_qm(true)` produced severely degraded output across the
  q≥60 range and non-conformant bitstreams at zenavif quality=100. No API
  changes in zenravif itself.
