# Changelog

zenravif is Imazen's fork of the excellent [ravif](https://github.com/kornelski/cavif-rs)
encoder by Kornel Lesiński, extended for the zenrav1e fork's still-image work.
(Changelog started 2026-07-03; earlier history lives in `git log`.)

## [Unreleased]

### Added
- `GainMapData` carries the gain map's full mux description: `alt_colr_cicp`
  (CICP `colr` for the alternate rendition on the `tmap` item; unsupported
  code points fail with `Error::Unsupported` instead of being dropped),
  `alt_icc` (ICC-form alternate `colr`), and `chroma_subsampling` +
  `monochrome` (written into the gain-map item's `av1C`, which previously
  claimed 4:2:0 color for every byte-carried payload). Threaded to
  zenavif-serialize's `set_gain_map_{alt_colr,alt_icc,chroma_subsampling,monochrome}`.
- **`Encoder::encode_gray8` — true monochrome (Cs400) AVIF encoding**
  (imazen/zenavif#6): one `u8` luma sample per pixel in, a bitstream with
  only a luma plane out (no chroma planes coded). Output bytes are at
  parity with the gray→RGB expansion path on typical content (neutral
  chroma is skip-coded), but encoding skips chroma RDO entirely — measured
  2–3× faster (zenavif `benchmarks/mono_encode_ab_2026-06-11.txt`).
  Honors `with_bit_depth` (8/10/12), pixel range, CICP/ICC/EXIF/XMP,
  rotation/mirror, and the imazen-feature encoder knobs; the mono
  `av1C`/`pixi` container form comes from zenavif-serialize's
  `set_monochrome` (its spec-correct mono properties shipped earlier).
  MC is signaled `Unspecified` (mono has no chroma to describe).

### Changed
- `zenavif-serialize` dep floor → 0.2.0 (`set_gain_map_alt_icc`), supplied by
  a `[patch.crates-io]` git pin until 0.2.0 publishes (drop the patch at the
  dep bump).
- `expert::InternalParams.sb_q_scale`: per-64×64-superblock AC quantizer
  scale map for the color encode, forwarded to zenrav1e as
  `FrameHints::sb_q_scale` (the closed-loop second-pass channel — see
  zenavif `docs/DIFFMAP_TWO_PASS.md`). RELEASE-GATED behind
  `pub const FRAME_HINTS_LIVE = false` until the zenrav1e dep bumps past
  0.1.4; while false the map is accepted but not applied (byte-identical)
  and closed-loop callers must check the const (13b1ca4b).
