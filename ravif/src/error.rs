use quick_error::quick_error;
use std::fmt;

/// The reason a rav1e encode failed, captured from the underlying
/// [`zenrav1e`] error's [`Display`] output.
///
/// Previously this was a unit struct that discarded rav1e's reason; it now
/// carries the original message so error reports say *what* failed (e.g.
/// `invalid width 8 (expected >= 16, <= 65535)`) instead of a fixed string.
///
/// [`Display`]: std::fmt::Display
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct EncodingErrorDetail {
    /// The rav1e error reason, taken verbatim from its `Display` impl.
    reason: String,
}

impl EncodingErrorDetail {
    /// Capture the reason from any rav1e error that implements [`Display`].
    ///
    /// [`Display`]: std::fmt::Display
    fn from_reason(reason: impl fmt::Display) -> Self {
        Self { reason: reason.to_string() }
    }

    /// The preserved rav1e error reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for EncodingErrorDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

quick_error! {
    /// Failures enum
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum Error {
        /// Slices given to `encode_raw_planes` must be `width * height` large.
        TooFewPixels {
            display("Provided buffer is smaller than width * height")
        }
        /// The requested image dimensions exceed the configured pixel cap.
        ///
        /// Returned pre-flight (before any heavy encoding work) when
        /// `width * height` is greater than the encoder's `max_pixels`
        /// limit (default 120 megapixels). Raise or disable the limit with
        /// [`crate::Encoder::with_max_pixels`] (pass `0` to disable).
        TooManyPixels { width: usize, height: usize, max_pixels: u64 } {
            display("Image {}x{} ({} pixels) exceeds the configured limit of {} pixels", width, height, (*width as u64).saturating_mul(*height as u64), max_pixels)
        }
        Unsupported(msg: &'static str) {
            display("Not supported: {}", msg)
        }
        /// Encoding was cancelled via a cancellation token
        Cancelled {
            display("Encoding was cancelled")
        }
        /// An error reported by the underlying rav1e encoder.
        ///
        /// The contained [`EncodingErrorDetail`] preserves rav1e's own reason
        /// string (config validation message, encoder-status reason, etc.) so
        /// the message reflects *what* failed rather than a fixed placeholder.
        EncodingError(e: EncodingErrorDetail) {
            display("Encoding error reported by rav1e: {}", e)
            from(e: zenrav1e::InvalidConfig) -> (EncodingErrorDetail::from_reason(e))
            from(e: zenrav1e::EncoderStatus) -> (EncodingErrorDetail::from_reason(e))
        }
    }
}
