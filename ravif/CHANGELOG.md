# Changelog

## [Unreleased]

## [0.1.2] - 2026-04-27

### Changed
- Bump `zenrav1e` minimum version to 0.1.4 to pull in the QM level-mapping
  and AV1 lossless-conformance fixes (zenrav1e#7). Without these, AVIF
  encodes with `with_qm(true)` produced severely degraded output across the
  q≥60 range and non-conformant bitstreams at zenavif quality=100. No API
  changes in zenravif itself.
