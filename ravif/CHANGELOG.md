# Changelog

## [Unreleased]

### Changed
- `zenrav1e` (0.2.0, unpublished) is a git-rev dependency on imazen/zenrav1e
  master (`e4883037`) instead of the sibling path `../../zenrav1e` (09a0dba3).
  A path that escapes the repo made zenravif unresolvable for every git
  consumer (zenavif main → zencodecs / zenpipe). Same compiled sources; the
  wasm32 target dep carries the same rev. For sibling-lockstep work add an
  uncommitted `[patch."https://github.com/imazen/zenrav1e"]` path override at
  the workspace root.

### Added
- Small-rendition effort mode behind `SpeedTweaks::SMALL_PX_RDO_TX_LIVE`
  (bae4880, byte-identical while `false`): when flipped, frames with long
  edge < 1024 keep tx-size/type RDO on at high quality too. Measured on the
  zenavif size-decay non-tune A/B (2026-07-03): median ssim2 BD-rate +0.80
  @256 / +0.88 @512 vs the byte-identical baseline, better on 12/12 origins
  at both sizes, butteraugli agreeing; cost confined to the changed hi-q
  cells (~6.5x those cells, ~0.3->2.0 s at 256px). `from_my_preset` now
  takes the frame long edge (internal signature).

## [0.2.0] - 2026-06-16

### Changed (BREAKING)
- The public encode API now returns `At<Error>` instead of `Error`. Every encode
  method — `encode_rgba` / `encode_rgb` / `encode_raw_planes_{8,10,12}_bit` /
  `encode_animation_{rgb,rgba,rgb16,rgba16}` — returns
  `core::result::Result<_, At<Error>>` (re-exported as the crate's `Result<T>`
  alias). The wrapped [`whereat`](https://lib.rs/crates/whereat) `At<Error>`
  carries the originating `file:line:col` (and a clickable GitHub link when
  built from a checkout of this repo) for server-side debuggability, at zero
  cost on the success path. **Migration:** match the underlying error by
  borrowing it — `Err(e) if matches!(e.error(), Error::Cancelled)` instead of
  `Err(Error::Cancelled)` — and print the location trail with `e.full_trace()`.
  The plain `Display` of `At<Error>` shows only the error message, so code that
  just prints the error keeps working. `At<Error>` implements
  `std::error::Error` (delegating `source()` to `Error`), so `?`-conversion into
  `Box<dyn Error>` / `anyhow::Error` continues to work. `Error` itself, its
  variants, and `ValidationError` are unchanged; `Encoder::validate()` still
  returns `Result<(), ValidationError>`.

### Fixed
- Errors reported by the underlying rav1e encoder now **preserve rav1e's own
  reason string** (config-validation message, encoder-status reason) instead of
  the fixed `"Encoding error reported by rav1e"` placeholder. The `From`
  conversions for `zenrav1e::InvalidConfig` / `zenrav1e::EncoderStatus` capture
  the error's `Display` into `EncodingErrorDetail` (now a struct with a public
  `reason()` accessor), so a failed encode reports *what* failed — e.g.
  `Encoding error reported by rav1e: invalid width 70000 (expected >= 16, <= 65535)`.
  This addresses the audit's debuggability finding (the reason was discarded).

### Added
- `whereat` dependency for traced errors; `pub use whereat::At` and a crate-level
  `Result<T>` alias.

### Note (future zenrav1e 0.2.0 bump)
- zenravif currently consumes **published zenrav1e 0.1.4**, whose config/status
  errors are *bare* (`InvalidConfig` / `EncoderStatus`, no `At`). The context
  construction sites therefore start a fresh trace via `at!(Error::from(e))`.
  When zenravif bumps to `zenrav1e ^0.2.0` (which returns `At<InvalidConfig>`),
  switch those sites to `.map_err_at(Error::from)?` to carry zenrav1e's own
  trace through. The consumption sites are marked with a
  `// TODO(whereat): map_err_at once on zenrav1e 0.2.0` comment.

### Added (carried from the unreleased 0.1.x work, now shipping in 0.2.0)
- `Encoder::with_max_pixels(u64)` builder and `DEFAULT_MAX_PIXELS` const (120
  megapixels). The encode functions reject any request whose `width * height`
  exceeds the cap **pre-flight** — before allocating planes or building the
  rav1e context — returning the `Error::TooManyPixels { width, height,
  max_pixels }` variant (now traced via `at!`). Pass `with_max_pixels(0)` to
  disable the cap (unlimited) for already-trusted dimensions.

### Fixed (carried from the unreleased 0.1.x work)
- Encode paths no longer fail open on attacker-controlled dimensions. The three
  call sites that forced zenrav1e's `max_pixel_count` guard to `u64::MAX` (in
  `av1encoder.rs` and `animated.rs`) now forward the configured `max_pixels`
  value instead, so rav1e's own guard is not nulled, and a zenravif-side
  pre-flight dimension check runs at every still/animation encode entry point.
  Previously a server passing unbounded `w x h` got no pre-flight rejection.

### Changed (carried from the unreleased 0.1.x work)
- Add `CHANGELOG.md` to published package `include` list so crate consumers see release history

## [0.1.3] - 2026-05-02

### Added
- `Encoder::validate()` method returning `Result<(), ValidationError>` for
  fail-fast configuration checking. Existing `encode_*` methods retain their
  silent clamping behaviour; call `validate()` first when a batch job should
  reject out-of-range configs before spending compute. `ValidationError` is
  `#[non_exhaustive]` and covers `quality` / `alpha_quality` /
  `libavif_quality` / `speed` / `num_threads` / `rotation` / `mirror` /
  `vaq_strength` / `seg_boost` / `partition_range`, plus the
  `chroma_subsampling=Yuv420` × `color_model=RGB` cross-parameter rejection
  the encode path already returns as `Error::Unsupported`. Each variant is
  cited against the zenrav1e / zenavif-serialize source it mirrors.
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
- Theory-of-operation docs for each `InternalParams` field covering
  pipeline stage, override scenarios, mechanism, and speed-preset
  interaction, with citations into zenrav1e source (`encoder.rs:360`,
  `:372`, `:2958-3242`, `rdo.rs:1351-1491`, `deblock.rs:1624-1668`)
  and zenravif's own `SpeedTweaks::from_my_preset` overrides
  (`av1encoder.rs:1331-1365`) (5ea5487).
- 14-case `expert_tests` module covering per-field perturbation,
  idempotency, all-fields-set valid encode, default-equals-baseline,
  and wholesale reset semantics for `with_internal_params` (1e02e9e).

### Changed
- `with_quality`, `with_alpha_quality`, `with_libavif_quality`, `with_speed`,
  and `with_num_threads` no longer panic on out-of-range inputs. They
  silently accept the value and let the encode path clamp (matching the
  documented "use `validate()` for fail-fast" pattern). Callers that
  relied on the panic for input validation should call `validate()` instead.

### Fixed
- `partition_range` documentation now reflects zenrav1e's actual
  `max <= 64x64` constraint; passing `128` triggers a debug-mode
  panic at `zenrav1e/src/encoder.rs:2958/3231` (1e02e9e).
- Pre-existing dead-code warnings on the `imazen`-gated
  `override_cdef` / `override_rdo_tx_decision` fields of
  `Av1EncodeConfig` annotated `#[allow(dead_code)]` so the
  `__expert`-feature clippy pass is clean (bb67188).

## [0.1.2] - 2026-04-27

### Changed
- Bump `zenrav1e` minimum version to 0.1.4 to pull in the QM level-mapping
  and AV1 lossless-conformance fixes (zenrav1e#7). Without these, AVIF
  encodes with `with_qm(true)` produced severely degraded output across the
  q≥60 range and non-conformant bitstreams at zenavif quality=100. No API
  changes in zenravif itself.
