# Changelog

zenravif is Imazen's fork of the excellent [ravif](https://github.com/kornelski/cavif-rs)
encoder by Kornel Lesiński, extended for the zenrav1e fork's still-image work.
(Changelog started 2026-07-03; earlier history lives in `git log`.)

## [Unreleased]

### Added
- `expert::InternalParams.sb_q_scale`: per-64×64-superblock AC quantizer
  scale map for the color encode, forwarded to zenrav1e as
  `FrameHints::sb_q_scale` (the closed-loop second-pass channel — see
  zenavif `docs/DIFFMAP_TWO_PASS.md`). RELEASE-GATED behind
  `pub const FRAME_HINTS_LIVE = false` until the zenrav1e dep bumps past
  0.1.4; while false the map is accepted but not applied (byte-identical)
  and closed-loop callers must check the const (13b1ca4b).
