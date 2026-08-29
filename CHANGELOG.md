# Changelog

zenravif is Imazen's fork of the excellent [ravif](https://github.com/kornelski/cavif-rs)
encoder by Kornel Lesiński, extended for the zenrav1e fork's still-image work.
(Changelog started 2026-07-03; earlier history lives in `git log`.)

## [Unreleased]

### Changed
- **rav1d-safe dev-dep pin `a6a7e232` → `66f58fa6`** (this commit), aligning
  this repo with the rev zenavif settled on so the workspace decodes AVIF
  through one decoder. Dev-only: rav1d-safe is a dev-dependency of the root
  `cavif` package used by `examples/gate_sweep.rs` and
  `examples/hdr_pq10_probe.rs`; nothing in the published `zenravif` crate
  touches it, and no CI job compiles it (every job is `-p zenravif`).
  Measured before landing, not assumed —
  `benchmarks/rav1d_pin_66f58fa6_2026-08-29.tsv{.zst,.meta}`, 1200 cells per
  arm (5 content classes × 64/256/1024/2048 × speeds 1,2,4,6,8,10 × q5..100
  step 5), plus a repeat-run determinism control that differs in 0 rows:
  - **`Settings::strictness` now defaults to `Strict`** (rav1d-safe@2e0f7e8):
    non-conforming streams return `Error::InvalidData` instead of being
    concealed. Both examples take that default. **0 newly-rejected streams**
    — 1200 accepted before, 1200 after. Keeping Strict: the corpus here is
    zenravif's own output, so a stream we emit that libaom would reject is an
    encoder bug the harness should surface, not conceal.
  - **Encoder output is untouched**: `bytes`, `bpp` and `sha256` identical on
    all 1200 cells.
  - **Decoded pixels move on aarch64, off a wrong decoder onto a correct
    one.** 699/1200 cells score a different SSIMULACRA2 (mean |Δ| 0.039, max
    0.61, 402 up / 297 down). Bisected: `a6a7e232 → 140f9145` moves 339/400
    grid-A cells, `140f9145 → 66f58fa6` moves 0/400 — the whole movement is
    the aarch64 NEON conformance campaign of 2026-08-07/08 (rav1d-safe
    @ef01e85 itx eob/rect2-rounding +293 vectors, @ddbe8ba 16bpc PREP_BIAS +
    `intermediate_bits` +91, 8bpc compound +80), which took rav1d-safe from
    **302/766 to 766/766** against dav1d's published MD5s on aarch64.
    x86_64 was already 766/766 and is unaffected.
  - `Decoder::flush()` draining owed frames (rav1d-safe@59eb17b) does not
    affect this repo — neither example calls it, and the `ctx.flush()` calls
    in `src/{animated,av1encoder}.rs` are zenrav1e's *encoder* context.
- **The SSIMULACRA2 column of `benchmarks/gate_flip_summary_2026-08-06.tsv`
  is not comparable to post-bump runs on aarch64.** That record names
  `rav1d-safe a6a7e232 (decode side only)` and was taken on an Apple M4 Pro,
  i.e. through the pre-campaign decoder described above. Its bytes/BD-rate
  half is unaffected (encoder output is byte-identical across the bump);
  re-measure an ssim2 baseline on the new pin before the next gate flip.
- **The zenrav1e dep-bump release gates are cashed in** (c69050a + this
  commit). The dep moved to zenrav1e master/0.2.0 in 619d81a, which is the
  condition four speed-table gates named, so `S1_DEEP_ARMS_LIVE`,
  `S6_TX_SIZE_RDO_LIVE`, `S6_PART_PRUNE_LIVE` and `S10_RETIER_LIVE` are now
  `true` **and their apply blocks in `speed_settings()` are uncommented** — a
  flipped const over a commented apply block is a no-op that looks done, so
  both halves landed together. Every entry below that says "release-gated",
  "OFF until the zenrav1e dep bump" or "byte-identical while off" is
  superseded by this one for those four arms.

  Re-measured as-shipped rather than assumed —
  `benchmarks/gate_flip_summary_2026-08-06.tsv` (+ the compressed raw cells
  and the `.meta`). Harness: `examples/gate_sweep.rs` (encode → decode via
  rav1d-safe → SSIMULACRA2, one row per cell; its q100 cells score exactly
  100.0000, which pins the decode + full-range BT.601 inverse as exact).
  Grid: 5 content classes (2 photo, screen, text/UI, line-art/UI) × long-edge
  {64, 256, 1024, 2048} × quality 5..100 step 5 (step 10 at 2048 and for the
  s1 rows) × speeds 1-10, ~9,500 encode+score cells. Metric is pareto-front
  SSIMULACRA2 BD-rate; negative = fewer bytes at matched quality.

  **Shipped 4-arm config vs the pre-flip baseline (main grid, 64/256/1024,
  s4-s10, n=100 curves): median −2.9% BD-rate**, every speed row negative.
  All five arms together (i.e. including the still-off intra7 row) measure
  median −5.4% on that grid and **−9.95% at the 2048 tier** (n=20, no
  positive cell there). Speed rows carry it unevenly: s4 ≈ −1%, s6-s8 −4 to
  −11%, s9/s10 −12 to −15% (the S10 re-tier). Content: text/UI and screen
  gain most (−8 to −34% at 1024), photos least (−0.1 to −6%); the only
  material regressors anywhere are photo_a at s5 (+6.5% at 256, +1.5% at
  1024) and photo_a at s6/1024 (+0.5%).

  Honest caveats: (1) the arms were originally fit under
  `Tune::Ssimulacra2` (+ palette for the S10 rows) and ravif selects
  NEITHER, so these rows now ship in a configuration the original grids did
  not cover — that is exactly why they were re-measured tune-off; (2)
  `S6_PART_PRUNE_LIVE` arms the band 4..=8 but P1PART only ever fit s4, s6
  and s8 — s5 and s7 ride along, and s5 is the weakest row in the new data
  (median −2.0%, and the home of both photo regressions); (3) no arm touches
  per-SB delta-q, Variance Boost, or segmentation (ravif never sets
  `variance_boost_*` and never selects the ss2 tune), so the
  delta-q-disables-segmentation step is *not* inside this before/after; (4)
  the encode-time cost is reported separately in the `.meta` from a solo,
  un-niced run — the sweep's own `enc_ms` column came from a contended
  multi-process run and is a ratio at best. Solo cost of the shipped config
  vs baseline: 1.5-2.5x at s1/s4/s5, 2.3-5.0x at s6/s8, 1.3-1.8x at s9,
  0.84-0.97x at s10 (the re-tier is both better AND slightly faster there).

- **Known: the armed rows turn zenavif's
  `tests/hdr_roundtrip.rs::pq10_pixel_fidelity_within_bounds` red**, and the
  threshold was deliberately NOT raised. Reproduced and bisected in-crate with
  the new `examples/hdr_pq10_probe.rs`: at q95/s8 the 10-bit identity-matrix
  roundtrip's max per-channel |Δ| goes 607 → 1281 (16-bit units) against that
  test's 900 budget. `S6_TX_SIZE_RDO_LIVE` is the dominant cause (1281 on its
  own); `S6_PART_PRUNE_LIVE` alone reaches 960; `S1_DEEP_ARMS_LIVE` and
  `S10_RETIER_LIVE` do not apply at s8 and are exactly inert there. The
  evidence says RD reallocation rather than corruption: exactly one cell of
  9,216 crosses the budget (one channel of a single max-white specular pixel,
  1023 → 1003 in ten-bit units), the ramp and colour-patch regions keep
  identical maxima, mean |Δ| improves at every quality, bytes drop 18-25%,
  and **at matched bytes the arm wins the tail as well** (baseline q90 =
  1125 B / max 1665 / mean 85 vs armed q95 = 1127 B / max 1281 / mean 47).
  Caveats: n = 1 synthetic fixture and no perceptual metric on the 10-bit
  path. Whether that budget should move is a user decision. Full record: the
  "10-BIT HDR TAIL" section of
  `benchmarks/gate_flip_summary_2026-08-06.tsv.meta`.

- **`S6_INTRA7_LIVE` stays `false` — measured, positive, blocked on a test
  premise, NOT on RD.** Isolated on the same grid (4-arm shipped vs 5-arm)
  it is a win that reproduces its train26 record: **s6 −0.55% median BD
  (n=14 curves), s7 −1.29% (n=13), s8 −1.29% (n=13)**, with every other
  speed row byte-identical (100/100 cells — which also proves its apply
  block is live and bounded to 6..=8). It is off only because arming it
  changes the s6-s8 *default* prediction-mode setting from forced-`Simple`
  to `ComplexKeyframes` + `filter_intra=Some(false)`, and two `expert_tests`
  (`complex_prediction_modes_false_matches_default`,
  `partition_range_fixed_16_changes_bytes`) assert the old default on a
  speed-6 fixture. Their premise genuinely changed; fixing them means
  editing test fixtures, which is a human decision, so the arm waits rather
  than the tests being weakened. To arm: repoint those two tests at a speed
  outside the 6..=8 band (assertions unchanged — only the fixture speed),
  then flip the const.

- **`FRAME_HINTS_LIVE` is `true`** and the hinted `send_frame` is live
  (619d81a), so `expert::InternalParams::sb_q_scale` now actually reaches
  zenrav1e as `FrameHints::sb_q_scale`. Pinned by
  `ravif/tests/frame_hints_live.rs`.

### Fixed
- **An explicit `InternalParams::complex_prediction_modes` override is no
  longer clobbered by the speed table's intra_top7 arm.** Exposed by the
  gate flip: both apply sites (still-image and the alpha/second config path)
  now disarm `intra_top7` when the caller names that axis. The expert
  surface owns the axis it names.

### Added
- **`examples/hdr_pq10_probe.rs`** — reproduces zenavif's PQ10 fidelity test
  inside this crate (same fixture, same `encode_raw_planes_10_bit` + MC=Identity
  path, same reconstruction, same statistic; it reproduces the documented
  baseline 607/mean-50 exactly), so a speed-table change can be attributed to a
  specific arm without building the downstream crate. `--dump` prints the worst
  cells, a per-region breakdown, and how many cells cross the budget.
- **`examples/gate_sweep.rs`** — the release-gate A/B harness described
  above. Root package only (`cavif`, `publish = false`), so its extra
  dev-dependencies (`rav1d-safe`, `fast-ssim2`, `zenresize`) never reach a
  published crate. `rav1d-safe` is pinned to a git rev rather than registry
  0.5.7 because 0.5.7's aarch64 NEON self-guided loop-restoration path
  panics with an out-of-bounds index decoding our own LRF-on (low-quality)
  encodes; the 0.6.0-staging rev decodes the same cells cleanly.

- **Re-tiered s9/s10 rows** (S10 program, zenavif
  `docs/S10_PROGRAM.md` + `benchmarks/rd_gap_s10_2026-07-05.tsv`):
  `S10_RETIER_LIVE` const + `SpeedTweaks.num_modes_rdo_override`. The
  JPEG-anchored scoreboard measured the shipped s10 row losing to
  mozjpeg-class JPEG outright on bytes at matched ssim2 (registry 1.05-1.06x
  at ssim2≤60), and decomposed the cliff: `tx_domain_rate` −7.45% median
  ssim2 BD for 1.14x time, the (16,16) partition floor −13.5% at s9, CDEF-on
  −1.70% at 1.04x, depth-1 tx-size RDO −7.8%. Re-tiered rows: **s10' = txdr
  off + CDEF on + SATD-decides intra (−5.7/−6.9/−7.8 BD vs the old rung at
  0.95x its time — strictly better and faster; 4.3x mozjpeg-class encode
  time at 0.69-0.78x its bytes)**; **s9' = s10' + partition floor (8,16) +
  depth-1 tx-size RDO (−15.1/−18.2/−23.6 vs old s9 at 1.62x; 9.0x jpeg-moz
  at 0.54-0.60x bytes)**. NOW LIVE (`S10_RETIER_LIVE = true`); the byte
  gate that proved it inert while off (6/6 md5 at s9/s10 × q30/60/90) is
  superseded by the dep-bump A/B, where s9/s10 are the biggest movers
  (−12 to −15% median BD). Conformance: 0 failures across ~4,000 PALCONF'd
  cells at landing. Note the measured configs included the ss2 tune +
  palette and ravif selects neither — see the dep-bump entry's caveats.
- **s6-s8 top-7 keyframe intra RDO arm, still default-off** (9e413ac0 message /
  4b98f0f8 content): `S6_INTRA7_LIVE` const + `SpeedTweaks.intra_top7` —
  the P2HEADS-measured global fast-tier arm (`ComplexKeyframes` +
  `filter_intra=Some(false)`, the zenrav1e#5-safe top-7 form; the table's
  forced-Simple top-3 stands otherwise). Measured (train26, tune-ss2,
  veto-adjusted): s6 −0.56 / s8 −1.17 median BD, composition-stable on the
  P1 partition ship point; on the P2 composed fast mode it added −0.39 med
  train / −1.34 med val (composed+i7 13/13 better vs base, 0 butteraugli
  vetoes). STILL OFF, but no longer for a dep-bump reason — see the
  `S6_INTRA7_LIVE` entry at the top of [Unreleased] for the re-measurement
  and the exact unblocking step. Record: zenavif
  `benchmarks/rd_gap_p2heads_2026-07-04.tsv`.

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
- **s4–s8 rect-partition liveness arms** (`S6_PART_PRUNE_LIVE`, now
  `true`; P1PART 2026-07-04, FAST_TIER_PARITY_PLAN P1 lever 1): the speed
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
  per-image-hint targets, not shipped. NOW LIVE
  (`S6_PART_PRUNE_LIVE = true`), which also un-gates the 16×16 threshold
  value. Scope caveat: the shipped band is 4..=8 but only s4/s6/s8 were ever
  fit — s5 and s7 ride along un-fit, and s5 is the weakest row in the
  dep-bump re-measurement. Record: zenavif
  `benchmarks/rd_gap_p1part_2026-07-04.tsv` + this crate's
  `benchmarks/gate_flip_summary_2026-08-06.tsv`.

- **s6–s8 depth-1 intra tx-size RDO arms** (`S6_TX_SIZE_RDO_LIVE`, now
  `true`; 7baad5f9): the s4→s6 rdo_tx cliff decomposition (zenavif
  FAST_TIER_PARITY_PLAN P0) measured that keeping ONLY the tx-SIZE half
  of the coupled `rdo_tx_decision` boolean alive, depth-limited to 1
  split level with DCT-only types, recovers 51% of the whole s6→s4 RD
  step — full-grid confirm: s6 ssim2/ba3n/bamax median BD
  −2.78/−3.95/−6.01 (18–20/24 better) at 1.67× solo wall; s8
  −2.89/−3.52/−5.49 at 1.43×. The tx-TYPE half alone costs 2.4× with a
  butteraugli-max veto and only pays composed (size1+reduced-types = 92%
  of the step at 4.6× solo — recorded as P1 seed data, not shipped);
  `reduced_tx_set` alone at s6/s8 is a measured null. Armed at the dep
  bump (const + both apply lines).
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
  zenavif `docs/DIFFMAP_TWO_PASS.md`). LIVE as of 619d81a
  (`FRAME_HINTS_LIVE = true`); the const remains public so a caller built
  against a zenrav1e without `FrameParameters::frame_hints` can still fail
  honestly instead of silently double-encoding (13b1ca4b).
