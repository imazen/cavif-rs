# Changelog

## [Unreleased]

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

### Changed
- `with_quality`, `with_alpha_quality`, `with_libavif_quality`, `with_speed`,
  and `with_num_threads` no longer panic on out-of-range inputs. They now
  silently accept the value and let the encode path clamp (matching the
  documented "use `validate()` for fail-fast" pattern). Callers that relied
  on the panic for input validation should call `validate()` instead.

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
- Theory-of-operation docs for each `InternalParams` field covering
  pipeline stage, override scenarios, mechanism, and speed-preset
  interaction, with citations into zenrav1e source (`encoder.rs:360`,
  `:372`, `:2958-3242`, `rdo.rs:1351-1491`, `deblock.rs:1624-1668`)
  and zenravif's own `SpeedTweaks::from_my_preset` overrides
  (`av1encoder.rs:1331-1365`) (5ea5487).
- 14-case `expert_tests` module covering per-field perturbation,
  idempotency, all-fields-set valid encode, default-equals-baseline,
  and wholesale reset semantics for `with_internal_params` (1e02e9e).

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
