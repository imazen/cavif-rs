//! Encoder configuration validation.
//!
//! [`Encoder::validate`] returns `Err(ValidationError)` for any
//! configuration the existing `encode_*` paths would silently clamp,
//! truncate, or `assert!`-panic on. Callers that want fail-fast
//! behaviour for batch jobs can call `validate()` before encoding;
//! the existing `encode_*` paths keep their clamping behaviour.
//!
//! # Source citations
//!
//! Each variant's valid range is taken from a verified source:
//!
//! - **`quality` / `alpha_quality` / `libavif_quality`**: panic-asserted
//!   `1..=100` in [`crate::Encoder::with_quality`] / `with_alpha_quality`
//!   / `with_libavif_quality` in `av1encoder.rs:308 / :337 / :359`.
//! - **`speed`**: panic-asserted `1..=10` in `with_speed`
//!   (`av1encoder.rs:374`).
//! - **`num_threads`**: panic-asserted `n > 0` when `Some(n)` in
//!   `with_num_threads` (`av1encoder.rs:403`).
//! - **`rotation`**: AVIF `irot` accepts the value `0..=3` (each unit
//!   = 90°, CCW). zenavif-serialize's `set_rotation` truncates with
//!   `angle & 0x03` (`zenavif-serialize/src/lib.rs:268`), so any value
//!   above 3 is silently re-mapped. We reject those rather than encode
//!   surprise rotations.
//! - **`mirror`**: AVIF `imir` accepts `0..=1`. zenavif-serialize
//!   truncates with `axis & 0x01` (`zenavif-serialize/src/lib.rs:272`).
//! - **`partition_range`**: bounds must be one of `{4, 8, 16, 32, 64}`
//!   and `min <= max`. The mapping panics on any other value
//!   (`av1encoder.rs:1416-1430`); zenrav1e additionally has a
//!   `max <= 64×64` debug-assert (encoder.rs:2958-2969 / :3231-3242),
//!   so 128 currently triggers a debug panic and is rejected here.
//! - **`vaq_strength`**: zenrav1e clamps to `0.0..=4.0`
//!   (`zenrav1e/src/encoder.rs:884`). Documented as "0.0 to 4.0,
//!   default 1.0" in `zenrav1e/src/api/config/encoder.rs:129`.
//! - **`seg_boost`**: zenrav1e clamps to `0.5..=4.0`
//!   (`zenrav1e/src/encoder.rs:913`).
//! - **Cross-param `Yuv420` + `RGB`**: rejected by
//!   `encode_raw_planes_internal` (`av1encoder.rs:984`) at encode
//!   time as `Error::Unsupported`.

use std::fmt;
use std::ops::RangeInclusive;

/// Errors returned from [`crate::Encoder::validate`].
///
/// `#[non_exhaustive]` — additional variants may be added in any
/// patch release without a major-version bump.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// `quality` (0–100 user scale) outside `1.0..=100.0`.
    QualityOutOfRange { value: f32, valid: RangeInclusive<f32> },
    /// `alpha_quality` (0–100 user scale) outside `1.0..=100.0`.
    AlphaQualityOutOfRange { value: f32, valid: RangeInclusive<f32> },
    /// `libavif_quality` (0–100 linear avifenc scale) outside `1.0..=100.0`.
    LibavifQualityOutOfRange { value: f32, valid: RangeInclusive<f32> },
    /// `speed` outside `1..=10` (`with_speed` panics on out-of-range).
    SpeedOutOfRange { value: u8, valid: RangeInclusive<u8> },
    /// `num_threads` was `Some(0)` (panic-asserted in `with_num_threads`).
    NumThreadsZero,
    /// `rotation` outside `0..=3` (AVIF `irot.angle`, multiples of 90°).
    /// Any other value is silently truncated by zenavif-serialize.
    RotationOutOfRange { value: u8, valid: RangeInclusive<u8> },
    /// `mirror` axis outside `0..=1`. Other values are truncated to
    /// the low bit by zenavif-serialize, producing an unexpected axis.
    MirrorOutOfRange { value: u8, valid: RangeInclusive<u8> },
    /// `vaq_strength` outside zenrav1e's accepted `0.0..=4.0`. Values
    /// outside this range are clamped at encode time.
    VaqStrengthOutOfRange { value: f64, valid: RangeInclusive<f64> },
    /// `seg_boost` outside zenrav1e's accepted `0.5..=4.0` (when the
    /// boost is non-trivially active). 1.0 is the no-op default and
    /// always passes; anything else outside the range is clamped.
    SegBoostOutOfRange { value: f64, valid: RangeInclusive<f64> },
    /// `partition_range` invariant violated: each bound must be one of
    /// `{4, 8, 16, 32, 64}` and `min <= max`. zenrav1e currently
    /// rejects 128 via a debug-assert; we surface it as an error here.
    PartitionRangeInvalid { min: u8, max: u8 },
    /// Two settings are mutually exclusive at encode time. Currently
    /// only `chroma_subsampling = Yuv420` with `color_model = RGB` (the
    /// encode path returns `Error::Unsupported` for that combination).
    MutuallyExclusive { a: &'static str, b: &'static str },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QualityOutOfRange { value, valid } => {
                write!(f, "quality {value} out of valid range {valid:?}")
            }
            Self::AlphaQualityOutOfRange { value, valid } => {
                write!(f, "alpha_quality {value} out of valid range {valid:?}")
            }
            Self::LibavifQualityOutOfRange { value, valid } => {
                write!(f, "libavif_quality {value} out of valid range {valid:?}")
            }
            Self::SpeedOutOfRange { value, valid } => {
                write!(f, "speed {value} out of valid range {valid:?}")
            }
            Self::NumThreadsZero => f.write_str("num_threads must be > 0 when Some"),
            Self::RotationOutOfRange { value, valid } => write!(
                f,
                "rotation {value} out of valid range {valid:?} (units of 90° CCW)"
            ),
            Self::MirrorOutOfRange { value, valid } => write!(
                f,
                "mirror axis {value} out of valid range {valid:?} (0=vertical, 1=horizontal)"
            ),
            Self::VaqStrengthOutOfRange { value, valid } => {
                write!(f, "vaq_strength {value} out of valid range {valid:?}")
            }
            Self::SegBoostOutOfRange { value, valid } => {
                write!(f, "seg_boost {value} out of valid range {valid:?}")
            }
            Self::PartitionRangeInvalid { min, max } => write!(
                f,
                "partition_range {min}..{max} invalid: must satisfy min <= max and both ∈ {{4, 8, 16, 32, 64}}"
            ),
            Self::MutuallyExclusive { a, b } => {
                write!(f, "mutually exclusive: {a} and {b} cannot be combined")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Range constants (single source of truth for tests + impls).
pub(crate) const QUALITY_RANGE: RangeInclusive<f32> = 1.0..=100.0;
pub(crate) const SPEED_RANGE: RangeInclusive<u8> = 1..=10;
pub(crate) const ROTATION_RANGE: RangeInclusive<u8> = 0..=3;
pub(crate) const MIRROR_RANGE: RangeInclusive<u8> = 0..=1;
#[cfg(feature = "imazen")]
pub(crate) const VAQ_STRENGTH_RANGE: RangeInclusive<f64> = 0.0..=4.0;
#[cfg(feature = "imazen")]
pub(crate) const SEG_BOOST_RANGE: RangeInclusive<f64> = 0.5..=4.0;

#[cfg(feature = "imazen")]
#[inline]
pub(crate) fn is_valid_partition_size(s: u8) -> bool {
    matches!(s, 4 | 8 | 16 | 32 | 64)
}
