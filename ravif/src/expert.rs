//! Expert-only knobs for codec calibration and picker training.
//!
//! Anything in this module is **unstable**: it may change in any patch
//! release without semver justification, and is **not part of the
//! public API contract**. Reach for it only when:
//!
//! 1. Sweeping parameter combinations to feed a picker / regression /
//!    calibration training pipeline.
//! 2. Diagnosing codec behaviour by overriding speed-preset defaults.
//! 3. Wiring a `predict` feature (separate cargo feature, future) that
//!    selects [`InternalParams`] via a baked MLP.
//!
//! Everything in here lives behind the `__expert` cargo feature, whose
//! double-underscore signals "private — do not depend on this in
//! production code." Default builds expose only stable public knobs
//! ([`crate::Encoder::with_quality`], `with_speed`, etc.).
//!
//! # Where the overrides land
//!
//! Each `Some(_)` field replaces the value the speed preset would have
//! picked, **after** [`zenrav1e::prelude::SpeedSettings::from_preset`]
//! and **after** zenravif's own preset overrides in
//! `SpeedTweaks::from_my_preset` (see `av1encoder.rs`). `None` falls
//! through to whatever the preset chose. Apply via
//! [`crate::Encoder::with_internal_params`]; the call replaces *all*
//! four fields wholesale, so you can reset by passing
//! `InternalParams::default()`.

/// Expert override knobs for the AVIF encoder.
///
/// Each field is `Option<T>`: `None` (the [`Default`]) keeps the speed
/// preset's value, `Some(_)` overrides it. Apply via
/// [`crate::Encoder::with_internal_params`].
///
/// `#[non_exhaustive]` — fields may be added in any patch release.
/// Construct via [`Default::default`] and field-by-field assignment;
/// callers cannot use struct-literal syntax outside this crate.
///
/// # Example
///
/// ```ignore
/// # #[cfg(feature = "__expert")] {
/// use zenravif::{Encoder, expert::InternalParams};
///
/// let mut params = InternalParams::default();
/// params.partition_range = Some((4, 16));
/// params.lrf = Some(false);
///
/// let encoder = Encoder::new()
///     .with_quality(85.0)
///     .with_speed(6)
///     .with_internal_params(params);
/// # }
/// ```
#[non_exhaustive]
#[derive(Default, Clone, Debug)]
pub struct InternalParams {
    /// Partition block-size search range `(min, max)` in pixels. Each
    /// bound must be one of `{4, 8, 16, 32, 64}` and `min <= max`.
    /// (zenrav1e currently rejects `128` via a `max <= 64×64` debug
    /// assert in `encoder.rs:2958`/`:3231`; passing `128` triggers a
    /// debug-mode panic. The wider 128 path is reserved for future
    /// AV1 large-superblock support.)
    ///
    /// **Pipeline stage:** partition / mode decision. Drives the
    /// recursive AV1 superblock split during RDO. In zenrav1e the
    /// bounds gate the `must_split` / `can_split` decisions in
    /// `encode_partition_topdown` / `encode_partition_bottomup`
    /// (see `zenrav1e/src/encoder.rs:2958-2969` and `:3231-3242`).
    /// `bsize > max` forces a split; `bsize > min` allows one. The
    /// encoder never tries blocks outside the range, so this knob both
    /// caps speed and constrains the achievable RD curve.
    ///
    /// **Why override:**
    /// - **Sharp text / screen content** benefits from `Some((4, 16))`
    ///   — small blocks track glyph edges, and large blocks waste bits
    ///   on transform coefficients that the entropy coder can't reuse.
    /// - **Smooth photos at q ≥ 85** benefit from `Some((16, 64))` or
    ///   `Some((32, 64))` — the 4×4/8×8 partitions never win RDO at
    ///   high q (they pay a partition-flag cost for no distortion
    ///   improvement) and disabling them shaves encode time.
    /// - **Calibration sweeps** want `Some((4, 64))` to expose the
    ///   full RD frontier so a picker can learn where the partition
    ///   boundaries live (`128` is rejected by zenrav1e — see above).
    ///
    /// **Mechanism:** the encoder's RDO loop picks the partition shape
    /// per superblock by recursing within `[min, max]`. Setting both
    /// bounds equal (e.g. `Some((16, 16))`) forces fixed-size blocks
    /// and skips partition RDO entirely. Bounds outside the speed
    /// preset's range can both expand and contract the search space.
    ///
    /// **Speed-preset interaction:** zenravif's `SpeedTweaks` clamps
    /// the upper bound to 16 at high quality and reshapes the range
    /// per speed (see `av1encoder.rs:1331-1338`):
    /// - speed 0..=4: `(4, 16)` (or `(4, 64)` low-q)
    /// - speed 5..=8: `(8, 16)` (the typical default)
    /// - speed 9+:    `(16, 16)` (fixed 16×16, fastest)
    ///
    /// Underneath that, `SpeedSettings::from_preset` in zenrav1e widens
    /// to `(8, 64)` at speed 3 and shrinks to `(16, 32)` / `(32, 32)`
    /// at speed 9+ (`speedsettings.rs:135-188`); zenravif's preset
    /// overrides are applied on top.
    pub partition_range: Option<(u8, u8)>,

    /// Override intra prediction-mode search depth.
    /// `Some(true)` = `ComplexAll` (all intra modes searched on every
    /// frame). `Some(false)` = `Simple` (reduced mode set on every
    /// frame, plus `enable_filter_intra=false` in the AV1 sequence
    /// header).
    ///
    /// **Pipeline stage:** intra prediction / mode decision. Maps to
    /// `zenrav1e::api::PredictionModesSetting`. The setting is read
    /// in two places (`zenrav1e/src/rdo.rs:1351-1357`, `:1481-1491`)
    /// to decide how many candidate intra modes the RDO loop scores
    /// per block, and at sequence-header build time in
    /// `zenrav1e/src/encoder.rs:360-361` to decide whether
    /// `enable_filter_intra` is signalled in the bitstream at all.
    ///
    /// **Why override:**
    /// - **Calibration sweeps** that need the full intra search to
    ///   measure the upper bound of intra-only RD: `Some(true)`.
    /// - **Diagnosing the still-image guard:** zenravif forces
    ///   `Simple` for stills (`av1encoder.rs:1344`) because
    ///   `ComplexAll` triggers `filter_intra` RDO with broken cost
    ///   estimation that costs ~12 dB PSNR at speed 1
    ///   (zenrav1e#5). `Some(true)` lets you reproduce or verify
    ///   that regression. **Production stills should leave this at
    ///   `None`** — the override exists to expose the bug, not hide
    ///   it.
    /// - **Animated sequences** where the filter-intra bug is less
    ///   pronounced and the extra modes can recover RD on textured
    ///   inter frames.
    ///
    /// **Mechanism:** `Simple` searches **3** intra modes per block
    /// for inter frames and **3** for keyframes; `ComplexKeyframes`
    /// (the speed-preset default at speed 0..=6) searches **7** on
    /// keyframes; `ComplexAll` searches **7** on every frame and
    /// additionally enables filter-intra mode bits in the bitstream
    /// (rdo.rs:1481-1491; encoder.rs:360-361). For inter-mode RDO,
    /// `ComplexAll` switches from a 9-mode shortlist to the full
    /// inter-mode set (rdo.rs:1351-1357).
    ///
    /// **Speed-preset interaction:** zenrav1e's preset sets
    /// `ComplexAll` at speed 0..=1, `ComplexKeyframes` at speed 2..=6,
    /// and `Simple` at speed 7+ (`speedsettings.rs:128-158`). zenravif
    /// then **forces `Simple` regardless of speed** for still images
    /// (`av1encoder.rs:1344`). Setting this to `Some(true)`
    /// (=`ComplexAll`) defeats that guard.
    pub complex_prediction_modes: Option<bool>,

    /// Override loop restoration filter (LRF: Wiener + Self-Guided).
    /// `Some(true)` enables Wiener/SGR search and emits restoration
    /// units in the bitstream; `Some(false)` disables both and clears
    /// `enable_restoration` in the AV1 sequence header.
    ///
    /// **Pipeline stage:** post-filter (after deblock + CDEF, before
    /// frame output). LRF runs on the reconstructed frame and stores
    /// per-restoration-unit filter parameters in the bitstream.
    /// The flag is consumed at `zenrav1e/src/encoder.rs:372-373`
    /// (`enable_restoration: config.speed_settings.lrf && ...`),
    /// which gates whether Wiener/SGR searches run at all and whether
    /// the restoration unit headers are written.
    ///
    /// **Why override:**
    /// - **Noisy DSLR / film captures at low q (q ≤ 50)**: `Some(true)`
    ///   recovers measurable PSNR by smoothing reconstruction error
    ///   that survives deblock+CDEF. The preset already enables LRF
    ///   here, so the override is for sweeps that need to A/B it.
    /// - **Smooth photos at q ≥ 85**: `Some(false)` saves encode time
    ///   with no measurable RD loss — at high q the residual energy
    ///   LRF would smooth is already below quantization noise.
    /// - **Line art / pixel art / sharp text**: `Some(false)` prevents
    ///   LRF from over-softening hard edges that survived deblock.
    ///
    /// **Mechanism:** when enabled, the encoder per-frame searches
    /// Wiener filter coefficients and SGR (self-guided) parameters per
    /// restoration unit (typically 64×64 or 256×256 pixels). The cost
    /// is RDO over both filter types plus the rate of signalling the
    /// chosen coefficients. The SGR search depth is independently
    /// controlled by `sgr_complexity` (not exposed here). When
    /// disabled, `enable_restoration` in the sequence header is `0`
    /// and decoders skip the post-filter stage entirely.
    ///
    /// **Speed-preset interaction:** zenrav1e enables LRF at speed
    /// 0..=7 and disables it at speed 8+ (`speedsettings.rs:79`,
    /// `:170`). zenravif's `SpeedTweaks` then narrows that to
    /// `low_quality && speed <= 8` — i.e., LRF is only on when the
    /// quantizer is above ~150 (≈Q50 and below) AND speed ≤ 8
    /// (`av1encoder.rs:1365`). At Q ≥ 85 with any speed, the preset
    /// turns LRF off; this override is the way to flip it back on.
    pub lrf: Option<bool>,

    /// Override fast vs full deblock-filter level search.
    /// `Some(true)` = closed-form q-derived deblock level (fast).
    /// `Some(false)` = full SSE-driven search across deblock levels
    /// (slow, better edge preservation).
    ///
    /// **Pipeline stage:** post-filter (deblock filter level
    /// optimization, before CDEF). The flag is consumed in
    /// `zenrav1e/src/deblock.rs:1624` inside `deblock_filter_optimize`,
    /// which decides per frame what loop-filter level(s) the
    /// reconstruction pass will apply.
    ///
    /// **Why override:**
    /// - **Sharp text / screen content / line art**: `Some(false)`
    ///   keeps the SSE-driven search, which finds smaller deblock
    ///   levels and preserves the hard edges the closed-form formula
    ///   would over-smooth.
    /// - **Smooth photos / video where speed matters**: `Some(true)`
    ///   skips the per-frame search and uses a precomputed level. The
    ///   formula was fit on natural images; it produces the right
    ///   answer there but can over-blur or under-blur on outliers.
    /// - **Diagnosing edge artifacts**: flipping the flag is the
    ///   fastest way to confirm whether deblock-level search is the
    ///   cause.
    ///
    /// **Mechanism:** when fast, the level is computed in closed form
    /// from the AC quantizer and frame type via 8/10/12-bpc-specific
    /// fixed-point coefficients (`deblock.rs:1624-1654`). When slow,
    /// `sse_optimize` searches deblock levels by reconstructing each
    /// 4×4 luma block and minimizing reconstruction SSE against the
    /// source (`deblock.rs:1655-1668`). The slow path can run dozens
    /// of trial reconstructions per frame.
    ///
    /// **Speed-preset interaction:** zenrav1e enables `fast_deblock`
    /// at speed 7+ (`speedsettings.rs:165`). zenravif's `SpeedTweaks`
    /// further restricts that to `speed >= 7 && !high_quality`
    /// (`av1encoder.rs:1362`) — i.e., at Q ≥ 80 the slow search runs
    /// even at speed 10. Override `Some(true)` if you want the fast
    /// path at high q, or `Some(false)` to force the slow search at
    /// any speed for edge-sensitive content.
    pub fast_deblock: Option<bool>,
}
