# Changelog

zenravif is Imazen's fork of the excellent [ravif](https://github.com/kornelski/cavif-rs)
encoder by Kornel Lesiński, extended for the zenrav1e fork's still-image work.
(Changelog started 2026-07-03; earlier history lives in `git log`.)

## [Unreleased]

### Fixed
- **Default tile count no longer scales with host core count** (55f8c935):
  the default policy computed `tiles = min(threads, px/min_tile_size²)`
  with a 128–256 px floor at s≥4, so a 1 MP s6 encode on a 48-core host
  silently split into 64 tiles — each tile restarts entropy-context (CDF)
  adaptation and truncates cross-tile intra prediction. Measured on
  1024px-class stills (train26, tune-ss2): the 48-core default cost
  **+7.4% median ssim2 BD-rate (0/24 images better, up to +19.9% on a
  single cell)** versus single-tile, and even 2 tiles cost +0.96% median
  with zero winners. New default caps tiles so each keeps ≥1 MP of pixels:
  ≤1 MP images never tile (bytes identical from 1 core to 48, verified
  18/18 md5), larger images tile only as far as ≥1 MP tiles allow.
  `--threads 1` output is byte-identical to the old policy (18/18 md5).
  Honest give-back: tiling bought a real 5.9×/6.8× wall speedup at s6/s4
  on 48 cores (170→1005 / 871→5911 ms/MP solo), which the default no
  longer takes from bytes — use faster `-s` presets for cheap speed
  (`--threads` still sizes the pool; pool width beyond the tile count was
  measured bitstream-inert and buys no wall time). Record: zenavif
  `benchmarks/rd_gap_fastwins_2026-07-04.tsv` + `docs/SPEED_LADDER.md`.

### Added
- **s4–s8 rect-partition liveness arms** (release-gated `S6_PART_PRUNE_LIVE=
  false`; P1PART 2026-07-04, FAST_TIER_PARITY_PLAN P1 lever 1): the speed
  table amputated HORZ/VERT at s4+ (`non_square_partition_max_threshold`
  8×8); these arms raise the threshold to 16×16 under the zenrav1e
  `topdown_prune` NONE-first candidate walk with the measured gate triple
  (skip-gated `none_breakout` 1.0, 16-parent 4-ways restricted to
  SPLIT-dominant blocks via one-sided `four_way_margin=0.0`, homogeneity
  vargate 2.0) — cheaper than the same liveness ungated at every tier
  (solo 2.16/2.08/1.75× vs 2.33/2.23/1.91× at s6/s8/s4). Full-grid 12-q
  confirms (train26 tune-ss2, ssim2/ba3n/bamax medians vs the s6+size1 /
  s8+size1 / stock-s4 bases): s6 −2.89/−2.51/−2.45 (24/24 both
  primaries), s8 −3.00/−2.49/−2.86 (24/24), s4 −1.94/−2.32/−2.74 (22/23),
  no butteraugli-max veto. Ladder movement (photos, vs cached
  aom-allintra): s6 vs cpu4def-ai +1.4→−4.6/−6.3 (crossed), s8 vs
  cpu6iq-ai +0.3→−3.6/−5.1 (crossed), s4 vs cpu2def-ai +2.8→−0.9/−5.6
  (crossed). The beyond-budget vargate/max32 arms (88–104% of the
  remaining s6→s4 step at 2.4–2.9×) are recorded in the zenavif TSV as
  per-image-hint targets, not shipped. Byte-identical at
  every speed while gated (18/18-cell md5 vs 4f2caa93; the threshold value
  is gated too — it is live in bottom-up edge-superblock coding even on
  registry zenrav1e). Flip at the zenrav1e dep bump (knob lands
  post-0.1.4). Record: zenavif `benchmarks/rd_gap_p1part_2026-07-04.tsv`.

- **s6–s8 depth-1 intra tx-size RDO arms** (release-gated
  `S6_TX_SIZE_RDO_LIVE = false`, byte-identical until the zenrav1e
  dep bump; 7baad5f9): the s4→s6 rdo_tx cliff decomposition (zenavif
  FAST_TIER_PARITY_PLAN P0) measured that keeping ONLY the tx-SIZE half
  of the coupled `rdo_tx_decision` boolean alive, depth-limited to 1
  split level with DCT-only types, recovers 51% of the whole s6→s4 RD
  step — full-grid confirm: s6 ssim2/ba3n/bamax median BD
  −2.78/−3.95/−6.01 (18–20/24 better) at 1.67× solo wall; s8
  −2.89/−3.52/−5.49 at 1.43×. The tx-TYPE half alone costs 2.4× with a
  butteraugli-max veto and only pays composed (size1+reduced-types = 92%
  of the step at 4.6× solo — recorded as P1 seed data, not shipped);
  `reduced_tx_set` alone at s6/s8 is a measured null. At the dep bump:
  flip the const + uncomment the two apply lines in `speed_settings()`.
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
