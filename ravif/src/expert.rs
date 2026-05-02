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
    /// Partition block-size range `(min, max)` in pixels. Each must be
    /// one of `{4, 8, 16, 32, 64, 128}` and `min <= max`. A narrow fine
    /// range (e.g. `(4, 16)`) helps text/screen content; a coarse range
    /// (e.g. `(32, 64)`) speeds up smooth photo encoding.
    pub partition_range: Option<(u8, u8)>,

    /// Override prediction-modes setting. `Some(true)` = ComplexAll
    /// (slowest, all intra modes). `Some(false)` = Simple (DC + smooth
    /// + nearest only). Currently disabled by default for stills via
    /// the imazen still-image guard.
    pub complex_prediction_modes: Option<bool>,

    /// Override loop restoration filter (Wiener / Self-Guided). Helps
    /// smooth/noisy content; can over-soften line art and text.
    pub lrf: Option<bool>,

    /// Override fast vs full deblock filter search. `Some(true)` = fast
    /// (less search). `Some(false)` = full (better edge preservation).
    pub fast_deblock: Option<bool>,
}
