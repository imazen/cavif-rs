use quick_error::quick_error;

#[derive(Debug)]
#[doc(hidden)]
pub struct EncodingErrorDetail; // maybe later

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
        EncodingError(e: EncodingErrorDetail) {
            display("Encoding error reported by rav1e")
            from(_e: zenrav1e::InvalidConfig) -> (EncodingErrorDetail)
            from(_e: zenrav1e::EncoderStatus) -> (EncodingErrorDetail)
        }
    }
}
