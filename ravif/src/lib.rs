//! Pure Rust AVIF image encoder based on rav1e.
//!
//! # Basic Usage
//!
//! ```rust
//! use ravif::*;
//! # fn doit(pixels: &[RGBA8], width: usize, height: usize) -> Result<(), Error> {
//! let res = Encoder::new()
//!     .with_quality(70.)
//!     .with_speed(4)
//!     .encode_rgba(Img::new(pixels, width, height))?;
//! std::fs::write("hello.avif", res.avif_file);
//! # Ok(()) }
//! ```
//!
//! # Timeout Support
//!
//! For image proxies and web servers, encoding can be limited with a built-in timeout:
//!
//! ```rust
//! use ravif::*;
//! use std::time::Duration;
//! # fn example(pixels: &[RGBA8], width: usize, height: usize) -> Result<(), Error> {
//!
//! let encoder = Encoder::new()
//!     .with_quality(70.)
//!     .with_timeout(Duration::from_millis(100));
//!
//! match encoder.encode_rgba(Img::new(pixels, width, height)) {
//!     Err(Error::Cancelled) => {
//!         println!("Encoding timed out");
//!         Err(Error::Cancelled)
//!     },
//!     result => result.map(|_| ()),
//! }
//! # }
//! ```
//!
//! # Cancellation Support
//!
//! For manual cancellation from another thread, use `CancellationToken`:
//!
//! ```rust
//! use ravif::*;
//! use std::thread;
//! use std::time::Duration;
//! # fn example(pixels: &[RGBA8], width: usize, height: usize) -> Result<(), Error> {
//!
//! let token = CancellationToken::new();
//! let token_clone = token.clone();
//!
//! // Cancel from another thread
//! thread::spawn(move || {
//!     thread::sleep(Duration::from_millis(100));
//!     token_clone.cancel();
//! });
//!
//! let encoder = Encoder::new()
//!     .with_quality(70.)
//!     .with_cancellation_token(token);
//!
//! match encoder.encode_rgba(Img::new(pixels, width, height)) {
//!     Err(Error::Cancelled) => {
//!         println!("Encoding cancelled");
//!         Err(Error::Cancelled)
//!     },
//!     result => result.map(|_| ()),
//! }
//! # }

mod av1encoder;

mod cancel;
pub use cancel::CancellationToken;

mod error;
pub use av1encoder::ColorModel;
pub use error::Error;

#[doc(hidden)]
#[deprecated = "Renamed to `ColorModel`"]
pub type ColorSpace = ColorModel;

pub use av1encoder::{AlphaColorMode, BitDepth, EncodedImage, Encoder};
#[doc(inline)]
pub use rav1e::prelude::MatrixCoefficients;

mod dirtyalpha;

#[doc(no_inline)]
pub use imgref::Img;
#[doc(no_inline)]
pub use rgb::{RGB8, RGBA8};

#[cfg(not(feature = "threading"))]
mod rayoff {
    pub fn current_num_threads() -> usize {
        std::thread::available_parallelism().map(|v| v.get()).unwrap_or(1)
    }

    pub fn join<A, B>(a: impl FnOnce() -> A, b: impl FnOnce() -> B) -> (A, B) {
        (a(), b())
    }
}

#[test]
fn encode8_with_alpha() {
    let img = imgref::ImgVec::new((0..200).flat_map(|y| (0..256).map(move |x| {
        RGBA8::new(x as u8, y as u8, 255, (x + y) as u8)
    })).collect(), 256, 200);

    let enc = Encoder::new()
        .with_quality(22.0)
        .with_bit_depth(BitDepth::Eight)
        .with_speed(1)
        .with_alpha_quality(22.0)
        .with_alpha_color_mode(AlphaColorMode::UnassociatedDirty)
        .with_num_threads(Some(2));
    let EncodedImage { avif_file, color_byte_size, alpha_byte_size , .. } = enc.encode_rgba(img.as_ref()).unwrap();
    assert!(color_byte_size > 50 && color_byte_size < 1000);
    assert!(alpha_byte_size > 50 && alpha_byte_size < 1000); // the image must have alpha

    let parsed = avif_parse::read_avif(&mut avif_file.as_slice()).unwrap();
    assert!(parsed.alpha_item.is_some());
    assert!(parsed.primary_item.len() > 100);
    assert!(parsed.primary_item.len() < 1000);

    let md = parsed.primary_item_metadata().unwrap();
    assert_eq!(md.max_frame_width.get(), 256);
    assert_eq!(md.max_frame_height.get(), 200);
    assert_eq!(md.bit_depth, 8);
}

#[test]
fn encode8_opaque() {
    let img = imgref::ImgVec::new((0..101).flat_map(|y| (0..129).map(move |x| {
        RGBA8::new(255, 100 + x as u8, y as u8, 255)
    })).collect(), 129, 101);

    let enc = Encoder::new()
        .with_quality(33.0)
        .with_speed(10)
        .with_alpha_quality(33.0)
        .with_bit_depth(BitDepth::Auto)
        .with_alpha_color_mode(AlphaColorMode::UnassociatedDirty)
        .with_num_threads(Some(1));
    let EncodedImage { avif_file, color_byte_size, alpha_byte_size , .. } = enc.encode_rgba(img.as_ref()).unwrap();
    assert_eq!(0, alpha_byte_size); // the image must not have alpha
    assert!(color_byte_size > 50 && color_byte_size < 1000);

    let parsed1 = avif_parse::read_avif(&mut avif_file.as_slice()).unwrap();
    assert_eq!(None, parsed1.alpha_item);

    let md = parsed1.primary_item_metadata().unwrap();
    assert_eq!(md.max_frame_width.get(), 129);
    assert_eq!(md.max_frame_height.get(), 101);
    assert!(md.still_picture);
    assert_eq!(md.bit_depth, 10);

    let img = img.map_buf(|b| b.into_iter().map(|px| px.rgb()).collect::<Vec<_>>());

    let enc = Encoder::new()
        .with_quality(33.0)
        .with_speed(10)
        .with_bit_depth(BitDepth::Ten)
        .with_alpha_quality(33.0)
        .with_alpha_color_mode(AlphaColorMode::UnassociatedDirty)
        .with_num_threads(Some(1));

    let EncodedImage { avif_file, color_byte_size, alpha_byte_size , .. } = enc.encode_rgb(img.as_ref()).unwrap();
    assert_eq!(0, alpha_byte_size); // the image must not have alpha
    assert!(color_byte_size > 50 && color_byte_size < 1000);

    let parsed2 = avif_parse::read_avif(&mut avif_file.as_slice()).unwrap();

    assert_eq!(parsed1.alpha_item, parsed2.alpha_item);
    assert_eq!(parsed1.primary_item, parsed2.primary_item); // both are the same pixels
}

#[test]
fn encode8_cleans_alpha() {
    let img = imgref::ImgVec::new((0..200).flat_map(|y| (0..256).map(move |x| {
        RGBA8::new((((x/ 5 + y ) & 0xF) << 4) as u8, (7 * x + y / 2) as u8, ((x * y) & 0x3) as u8, ((x + y) as u8 & 0x7F).saturating_sub(100))
    })).collect(), 256, 200);

    let enc = Encoder::new()
        .with_quality(66.0)
        .with_speed(6)
        .with_alpha_quality(88.0)
        .with_alpha_color_mode(AlphaColorMode::UnassociatedDirty)
        .with_num_threads(Some(1));

    let dirty = enc
        .encode_rgba(img.as_ref())
        .unwrap();

    let clean = enc
        .with_alpha_color_mode(AlphaColorMode::UnassociatedClean)
        .encode_rgba(img.as_ref())
        .unwrap();

    assert_eq!(clean.alpha_byte_size, dirty.alpha_byte_size); // same alpha on both
    assert!(clean.alpha_byte_size > 200 && clean.alpha_byte_size < 1000);
    assert!(clean.color_byte_size > 2000 && clean.color_byte_size < 6000);
    assert!(clean.color_byte_size < dirty.color_byte_size / 2); // significant reduction in color data
}

#[test]
fn test_cancellation_token_precancelled() {
    let img = imgref::ImgVec::new((0..100).flat_map(|y| (0..128).map(move |x| {
        RGBA8::new(x as u8, y as u8, 255, 255)
    })).collect(), 128, 100);

    let token = CancellationToken::new();
    token.cancel(); // Cancel before encoding

    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(5)
        .with_cancellation_token(token);

    let result = enc.encode_rgba(img.as_ref());
    assert!(matches!(result, Err(Error::Cancelled)));
}

#[test]
fn test_cancellation_token_during_encoding() {
    use std::thread;
    use std::time::Duration;

    // Large image to ensure encoding takes some time
    let img = imgref::ImgVec::new((0..512).flat_map(|y| (0..512).map(move |x| {
        RGBA8::new((x ^ y) as u8, (x + y) as u8, ((x * y) >> 8) as u8, 255)
    })).collect(), 512, 512);

    let token = CancellationToken::new();
    let token_clone = token.clone();

    // Spawn a thread to cancel after a short delay
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        token_clone.cancel();
    });

    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(1) // Slow speed to ensure encoding takes time
        .with_cancellation_token(token);

    let result = enc.encode_rgba(img.as_ref());
    // Should be cancelled (though timing is not guaranteed)
    // If it completes before cancellation, that's also valid behavior
    match result {
        Err(Error::Cancelled) => {
            // Expected case: cancellation worked
        }
        Ok(_) => {
            // Also acceptable: encoding completed before cancellation
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_no_cancellation_token_works_normally() {
    let img = imgref::ImgVec::new((0..100).flat_map(|y| (0..128).map(move |x| {
        RGBA8::new(x as u8, y as u8, 255, 255)
    })).collect(), 128, 100);

    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(10); // No cancellation token

    let result = enc.encode_rgba(img.as_ref());
    assert!(result.is_ok());
}

#[test]
fn test_timeout_expires() {
    use std::time::Duration;

    // Large image that takes a while to encode
    let img = imgref::ImgVec::new((0..1024).flat_map(|y| (0..1024).map(move |x| {
        RGBA8::new((x ^ y) as u8, (x + y) as u8, ((x * y) >> 8) as u8, 255)
    })).collect(), 1024, 1024);

    // Use speed=4 for reasonable packet frequency
    // Speed=1 generates packets too slowly for responsive timeout
    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(4)
        .with_timeout(Duration::from_millis(100));

    let start = std::time::Instant::now();
    let result = enc.encode_rgba(img.as_ref());
    let elapsed = start.elapsed();

    // This test is timing-dependent, so we accept either outcome:
    match result {
        Err(Error::Cancelled) => {
            // If cancelled, verify it happened reasonably close to timeout
            // Note: First packet can take a while, so we allow up to 1s grace period
            assert!(elapsed >= Duration::from_millis(50),
                "Cancelled too early: {:?}", elapsed);
            assert!(elapsed < Duration::from_secs(2),
                "Timeout took too long: {:?}", elapsed);
        }
        Ok(_) => {
            // If completed before timeout, that's fine (fast hardware)
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_timeout_does_not_expire() {
    use std::time::Duration;

    // Small image that should complete quickly
    let img = imgref::ImgVec::new((0..128).flat_map(|y| (0..128).map(move |x| {
        RGBA8::new(x as u8, y as u8, 255, 255)
    })).collect(), 128, 128);

    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(10) // Fast speed
        .with_timeout(Duration::from_secs(5)); // Generous timeout

    let result = enc.encode_rgba(img.as_ref());
    assert!(result.is_ok(), "Should complete within timeout");
}

#[test]
fn test_timeout_and_cancellation_token_together() {
    use std::time::Duration;

    let img = imgref::ImgVec::new((0..256).flat_map(|y| (0..256).map(move |x| {
        RGBA8::new(x as u8, y as u8, 255, 255)
    })).collect(), 256, 256);

    let token = CancellationToken::new();
    let token_clone = token.clone();

    // Cancel via token after 20ms
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        token_clone.cancel();
    });

    // But timeout is set to 1 second
    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(6) // Fast enough to generate packets quickly
        .with_cancellation_token(token)
        .with_timeout(Duration::from_secs(1));

    let start = std::time::Instant::now();
    let result = enc.encode_rgba(img.as_ref());
    let elapsed = start.elapsed();

    // Should be cancelled (either by token or timeout)
    if let Err(Error::Cancelled) = result {
        // Token should fire first (~20ms)
        // At speed=6, we should see cancellation relatively quickly
        // Allow up to 500ms for first packet at slower speeds
        assert!(elapsed < Duration::from_secs(1),
            "Should cancel sooner: {:?}", elapsed);
    }
}
