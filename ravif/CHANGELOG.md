# Changelog

## [Unreleased]

## [0.1.3] - 2026-05-02

### Added
- New `__expert` cargo feature exposing `expert::InternalParams`, an
  `Option<T>` struct of speed-preset overrides for content-dependent
  internal knobs:
  - `partition_range: Option<(u8, u8)>` — block size range
  - `complex_prediction_modes: Option<bool>` — ComplexAll vs Simple
  - `lrf: Option<bool>` — loop restoration filter
  - `fast_deblock: Option<bool>` — fast vs full deblock
  Apply via `Encoder::with_internal_params(InternalParams)`.
  `#[non_exhaustive]` + `Default` so callers tolerate field additions
  in any patch. Each `None` keeps the speed preset's default; each
  `Some(_)` overrides. Implies `imazen` (the underlying overrides).
- The double-underscore prefix on `__expert` signals private/unstable
  surface — anything in the `expert` module may change without semver
  bumps. Used by the zenavif rav1e knob predictor MLP training
  harness; not for production code.

## [0.1.2] - 2026-04-27

### Changed
- Bump `zenrav1e` minimum version to 0.1.4 to pull in the QM level-mapping
  and AV1 lossless-conformance fixes (zenrav1e#7). Without these, AVIF
  encodes with `with_qm(true)` produced severely degraded output across the
  q≥60 range and non-conformant bitstreams at zenavif quality=100. No API
  changes in zenravif itself.
