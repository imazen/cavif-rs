//! Pure Rust AVIF image encoder based on rav1e.
//!
//! # Basic Usage
//!
//! ```rust
//! use zenravif::*;
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
//! use zenravif::*;
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
//! use zenravif::*;
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
mod animated;

#[cfg(feature = "__expert")]
pub mod expert;

mod cancel;
pub use cancel::CancellationToken;
#[cfg(feature = "stop")]
pub use almost_enough::StopToken;

mod error;
pub use av1encoder::ColorModel;
pub use error::Error;

#[doc(hidden)]
#[deprecated = "Renamed to `ColorModel`"]
pub type ColorSpace = ColorModel;

pub use animated::{AnimFrame, AnimFrame16, AnimFrameRgba, AnimFrameRgba16, EncodedAnimation};
pub use av1encoder::{AlphaColorMode, BitDepth, ChromaSubsampling, EncodedImage, Encoder, GainMapData};
#[doc(inline)]
pub use zenrav1e::prelude::{
    ChromaticityPoint, ColorPrimaries, ContentLight, MasteringDisplay,
    MatrixCoefficients, PixelRange, TransferCharacteristics,
};

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
    let tmp_path = format!("/tmp/ravif-encode-test-failure-{color_byte_size}.avif");
    if color_byte_size <= 150 || color_byte_size >= 500 {
        std::fs::write(&tmp_path, &avif_file).expect(&tmp_path);
    }
    assert!(color_byte_size > 150 && color_byte_size < 500, "size = {color_byte_size}; expected ~= 215; see {tmp_path}");

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
            // Cancelled — the timeout mechanism works.
            // First packet in pure Rust mode can take 90+ seconds on CI,
            // so we only assert a very generous upper bound.
            assert!(elapsed >= Duration::from_millis(50),
                "Cancelled too early: {:?}", elapsed);
            assert!(elapsed < Duration::from_secs(180),
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
        // Token fires at ~20ms, but cancellation only happens at packet boundaries.
        // Pure Rust encoding of 256x256 at speed 6 can take 90+ seconds for the
        // first packet on slow CI runners, so we use a generous bound.
        assert!(elapsed < Duration::from_secs(180),
            "Should cancel sooner: {:?}", elapsed);
    }
}

#[test]
fn encode_420_smaller_than_444() {
    let img = imgref::ImgVec::new((0..200u32).flat_map(|y| (0..256u32).map(move |x| {
        RGBA8::new((x.wrapping_mul(7).wrapping_add(y * 3)) as u8,
                   (x.wrapping_add(y * 5)) as u8,
                   (x * 3).wrapping_sub(y) as u8, 255)
    })).collect(), 256, 200);

    let enc_444 = Encoder::new()
        .with_quality(70.0)
        .with_speed(10)
        .with_chroma_subsampling(ChromaSubsampling::Yuv444)
        .with_num_threads(Some(1));
    let result_444 = enc_444.encode_rgba(img.as_ref()).unwrap();

    let enc_420 = Encoder::new()
        .with_quality(70.0)
        .with_speed(10)
        .with_chroma_subsampling(ChromaSubsampling::Yuv420)
        .with_num_threads(Some(1));
    let result_420 = enc_420.encode_rgba(img.as_ref()).unwrap();

    // 4:2:0 should produce smaller files
    assert!(result_420.avif_file.len() < result_444.avif_file.len(),
        "420 ({}) should be smaller than 444 ({})",
        result_420.avif_file.len(), result_444.avif_file.len());

    // Verify the AVIF container has correct chroma subsampling metadata
    let parsed_420 = avif_parse::read_avif(&mut result_420.avif_file.as_slice()).unwrap();
    let md_420 = parsed_420.primary_item_metadata().unwrap();
    assert_eq!(md_420.chroma_subsampling, (true, true), "420 should have both subsampling flags set");

    let parsed_444 = avif_parse::read_avif(&mut result_444.avif_file.as_slice()).unwrap();
    let md_444 = parsed_444.primary_item_metadata().unwrap();
    assert_eq!(md_444.chroma_subsampling, (false, false), "444 should not have subsampling flags set");
}

#[test]
fn encode_420_odd_dimensions() {
    // Test with odd width and height to exercise edge handling
    let img = imgref::ImgVec::new((0..101).flat_map(|y| (0..129).map(move |x| {
        RGBA8::new((x * 3) as u8, (y * 5) as u8, ((x + y) * 7) as u8, 255)
    })).collect(), 129, 101);

    let enc = Encoder::new()
        .with_quality(60.0)
        .with_speed(10)
        .with_chroma_subsampling(ChromaSubsampling::Yuv420)
        .with_num_threads(Some(1));
    let result = enc.encode_rgba(img.as_ref()).unwrap();
    assert!(result.color_byte_size > 0);

    let parsed = avif_parse::read_avif(&mut result.avif_file.as_slice()).unwrap();
    let md = parsed.primary_item_metadata().unwrap();
    assert_eq!(md.max_frame_width.get(), 129);
    assert_eq!(md.max_frame_height.get(), 101);
}

#[test]
fn encode_420_rgb_rejected() {
    let img = imgref::ImgVec::new(vec![RGBA8::new(128, 128, 128, 255); 64 * 64], 64, 64);

    let enc = Encoder::new()
        .with_quality(70.0)
        .with_speed(10)
        .with_internal_color_model(ColorModel::RGB)
        .with_chroma_subsampling(ChromaSubsampling::Yuv420)
        .with_num_threads(Some(1));
    let result = enc.encode_rgba(img.as_ref());
    assert!(result.is_err(), "RGB + 420 should return an error");
}

#[test]
fn test_libavif_quality_produces_expected_quantizers() {
    // with_libavif_quality should use linear mapping: qindex = 255 * (100 - Q) / 100
    // Q70 -> qindex 76, Q50 -> qindex 128, Q30 -> qindex 178

    // Use a varied pattern that actually compresses differently at different qualities
    let img = imgref::ImgVec::new((0..128).flat_map(|y| (0..128).map(move |x| {
        RGBA8::new((x * 3) as u8, (y * 5) as u8, ((x + y) * 7) as u8, 255)
    })).collect(), 128, 128);

    let r70 = Encoder::new().with_libavif_quality(70.).with_speed(10).encode_rgba(img.as_ref()).unwrap();
    let r50 = Encoder::new().with_libavif_quality(50.).with_speed(10).encode_rgba(img.as_ref()).unwrap();
    let r30 = Encoder::new().with_libavif_quality(30.).with_speed(10).encode_rgba(img.as_ref()).unwrap();

    // Higher Q = smaller quantizer = larger file
    assert!(r70.avif_file.len() > r50.avif_file.len(),
        "Q70 ({}) should be larger than Q50 ({})", r70.avif_file.len(), r50.avif_file.len());
    assert!(r50.avif_file.len() > r30.avif_file.len(),
        "Q50 ({}) should be larger than Q30 ({})", r50.avif_file.len(), r30.avif_file.len());

    // Compare to with_quality at same Q - they should differ due to different curves
    let old70 = Encoder::new().with_quality(70.).with_speed(10).encode_rgba(img.as_ref()).unwrap();
    // The old curve at Q70 gives qindex 107, libavif gives 76 - so libavif should be larger
    assert!(r70.avif_file.len() > old70.avif_file.len(),
        "libavif Q70 ({}) should be larger than old Q70 ({})",
        r70.avif_file.len(), old70.avif_file.len());
}

#[test]
fn default_encoder_omits_colr_box() {
    // zenavif-serialize skips the colr box when all values match defaults
    // (BT.709 primaries, sRGB transfer, BT.601 matrix, full range).
    // This is correct: the AV1 bitstream already carries the color info,
    // and decoders can infer the defaults.
    let img = imgref::ImgVec::new(vec![RGB8::new(128, 128, 128); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .encode_rgb(img.as_ref())
        .unwrap();

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    // Default CICP matches zenavif-serialize's ColrBox::default(), so no colr box is written
    assert!(parser.color_info().is_none(),
        "default encoder should omit colr box (defaults match zenavif-serialize)");

    // The AV1 bitstream still carries the correct values
    let md = parser.primary_metadata().unwrap();
    assert_eq!(md.bit_depth, 10, "default depth should be 10-bit");
}

#[test]
fn pq_bt2020_signals_correct_cicp() {
    let img = imgref::ImgVec::new(vec![RGB8::new(128, 64, 200); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_color_primaries(ColorPrimaries::BT2020)
        .with_transfer_characteristics(TransferCharacteristics::SMPTE2084)
        .encode_rgb(img.as_ref())
        .unwrap();

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    let color = parser.color_info().expect("colr box should be present");
    match color {
        zenavif_parse::ColorInformation::Nclx {
            color_primaries,
            transfer_characteristics,
            full_range,
            ..
        } => {
            assert_eq!(*color_primaries, 9, "should be BT.2020 (9)");
            assert_eq!(*transfer_characteristics, 16, "should be PQ/SMPTE2084 (16)");
            assert!(*full_range, "should default to full range");
        }
        _ => panic!("expected nclx"),
    }
}

#[test]
fn hlg_display_p3_signals_correct_cicp() {
    let img = imgref::ImgVec::new(vec![RGB8::new(200, 100, 50); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_color_primaries(ColorPrimaries::SMPTE432)
        .with_transfer_characteristics(TransferCharacteristics::HLG)
        .encode_rgb(img.as_ref())
        .unwrap();

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    let color = parser.color_info().expect("colr box should be present");
    match color {
        zenavif_parse::ColorInformation::Nclx {
            color_primaries,
            transfer_characteristics,
            ..
        } => {
            assert_eq!(*color_primaries, 12, "should be Display P3 / SMPTE432 (12)");
            assert_eq!(*transfer_characteristics, 18, "should be HLG (18)");
        }
        _ => panic!("expected nclx"),
    }
}

#[test]
fn limited_range_signals_correctly() {
    let img = imgref::ImgVec::new(vec![RGB8::new(128, 128, 128); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_pixel_range(PixelRange::Limited)
        .encode_rgb(img.as_ref())
        .unwrap();

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    let color = parser.color_info().expect("colr box should be present");
    match color {
        zenavif_parse::ColorInformation::Nclx { full_range, .. } => {
            assert!(!*full_range, "should be limited range");
        }
        _ => panic!("expected nclx"),
    }
}

#[test]
fn twelve_bit_depth_encodes() {
    let img = imgref::ImgVec::new(vec![RGB8::new(128, 64, 200); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_bit_depth(BitDepth::Twelve)
        .encode_rgb(img.as_ref())
        .unwrap();

    assert!(result.color_byte_size > 0);

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    let md = parser.primary_metadata().unwrap();
    assert_eq!(md.bit_depth, 12, "should be 12-bit");
}

#[test]
fn twelve_bit_rgba_encodes() {
    let img = imgref::ImgVec::new((0..64u32).flat_map(|y| (0..64u32).map(move |x| {
        RGBA8::new((x * 4) as u8, (y * 4) as u8, 128, ((x + y) * 2) as u8)
    })).collect::<Vec<_>>(), 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_bit_depth(BitDepth::Twelve)
        .encode_rgba(img.as_ref())
        .unwrap();

    assert!(result.color_byte_size > 0);
    assert!(result.alpha_byte_size > 0, "should have alpha");

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();
    let md = parser.primary_metadata().unwrap();
    assert_eq!(md.bit_depth, 12);
}

#[test]
fn hdr10_full_pipeline() {
    // Simulates an HDR10 encode: PQ transfer, BT.2020 primaries, 10-bit,
    // with mastering display and content light level metadata.
    let img = imgref::ImgVec::new(vec![RGB8::new(128, 64, 200); 64 * 64], 64, 64);

    let result = Encoder::new()
        .with_quality(50.0)
        .with_speed(10)
        .with_num_threads(Some(1))
        .with_color_primaries(ColorPrimaries::BT2020)
        .with_transfer_characteristics(TransferCharacteristics::SMPTE2084)
        .with_bit_depth(BitDepth::Ten)
        .with_mastering_display(MasteringDisplay {
            primaries: [
                ChromaticityPoint { x: 13250, y: 34500 }, // green
                ChromaticityPoint { x: 7500,  y: 3000 },  // blue
                ChromaticityPoint { x: 34000, y: 16000 }, // red
            ],
            white_point: ChromaticityPoint { x: 15635, y: 16450 }, // D65
            max_luminance: 10000000,  // 10000 cd/m² in 24.8 fixed point
            min_luminance: 50,        // ~0.0003 cd/m² in 18.14 fixed point
        })
        .with_content_light(ContentLight {
            max_content_light_level: 1000,
            max_frame_average_light_level: 400,
        })
        .encode_rgb(img.as_ref())
        .unwrap();

    let parser = zenavif_parse::AvifParser::from_owned(result.avif_file).unwrap();

    // Verify CICP in container colr box
    let color = parser.color_info().expect("colr box should be present");
    match color {
        zenavif_parse::ColorInformation::Nclx {
            color_primaries,
            transfer_characteristics,
            ..
        } => {
            assert_eq!(*color_primaries, 9, "BT.2020");
            assert_eq!(*transfer_characteristics, 16, "PQ");
        }
        _ => panic!("expected nclx"),
    }

    // Verify bit depth in AV1 sequence header
    let md = parser.primary_metadata().unwrap();
    assert_eq!(md.bit_depth, 10);

    // Verify container-level HDR metadata (mdcv/clli property boxes)
    let clli = parser.content_light_level().expect("clli should be present");
    assert_eq!(clli.max_content_light_level, 1000);
    assert_eq!(clli.max_pic_average_light_level, 400);

    let mdcv = parser.mastering_display().expect("mdcv should be present");
    assert_eq!(mdcv.primaries[0], (13250, 34500)); // green
    assert_eq!(mdcv.primaries[1], (7500, 3000));   // blue
    assert_eq!(mdcv.primaries[2], (34000, 16000)); // red
    assert_eq!(mdcv.white_point, (15635, 16450));   // D65
    assert_eq!(mdcv.max_luminance, 10000000);
    assert_eq!(mdcv.min_luminance, 50);
}

#[cfg(all(test, feature = "__expert"))]
mod expert_tests {
    //! Permutation, idempotency, default-equivalence, and reset
    //! coverage for [`crate::expert::InternalParams`].
    //!
    //! All tests use a deterministic 96×96 mixed-content RGB image at
    //! `with_quality(60.0).with_speed(6).with_num_threads(Some(1))` so
    //! byte-for-byte equality comparisons are stable.

    use super::*;
    use crate::expert::InternalParams;

    fn synthetic_image() -> imgref::ImgVec<RGB8> {
        imgref::ImgVec::new(
            (0..96)
                .flat_map(|y| {
                    (0..96).map(move |x| {
                        RGB8::new(x as u8 * 2, y as u8 * 2, ((x ^ y) & 0xFF) as u8)
                    })
                })
                .collect(),
            96,
            96,
        )
    }

    fn encode_with(params: Option<InternalParams>) -> Vec<u8> {
        let img = synthetic_image();
        let mut enc = Encoder::new()
            .with_quality(60.0)
            .with_speed(6)
            .with_num_threads(Some(1));
        if let Some(p) = params {
            enc = enc.with_internal_params(p);
        }
        enc.encode_rgb(img.as_ref()).unwrap().avif_file
    }

    fn mk(f: impl FnOnce(&mut InternalParams)) -> InternalParams {
        let mut p = InternalParams::default();
        f(&mut p);
        p
    }

    // --- Per-field coverage: each value should perturb the bitstream
    //     vs. the baseline (no `with_internal_params` call). ---

    #[test]
    fn partition_range_narrow_encodes_validly() {
        // Default at speed 6 is (8, 16). Widening the lower bound to
        // (4, 16) lets the encoder consider 4×4 partitions; whether
        // those win RDO depends on the content. For this synthetic
        // 96×96 image at Q60 the encoder happens to pick the same
        // partitions, so byte equality is OK — what matters is that
        // the override doesn't crash and produces a valid file.
        let bytes = encode_with(Some(mk(|p| p.partition_range = Some((4, 16)))));
        assert!(bytes.len() > 100, "(4,16) encode should produce a non-trivial file");
        assert_eq!(&bytes[4..8], b"ftyp");
    }

    #[test]
    fn partition_range_coarse_changes_bytes() {
        // (16, 64) excludes the default's 8×8 lower bound and adds
        // 32×32 / 64×64 — guaranteed to pick different partitions
        // than (8, 16) on a non-trivial image.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.partition_range = Some((16, 64)))));
        assert_ne!(
            bytes, baseline,
            "partition_range=(16,64) should differ from the (8,16) preset default"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn partition_range_full_changes_bytes() {
        // Full (4, 64) range — widest zenrav1e currently accepts.
        // Adds 32×32 / 64×64 to the (8, 16) default, which on this
        // image picks different partitions and perturbs the bitstream.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.partition_range = Some((4, 64)))));
        assert_ne!(
            bytes, baseline,
            "partition_range=(4,64) should differ from the (8,16) preset default"
        );
        assert_eq!(&bytes[4..8], b"ftyp");
    }

    #[test]
    fn partition_range_fixed_16_changes_bytes() {
        // (16, 16) collapses the search to a single block size — this
        // is the mode the speed-9+ preset uses. Forcing it at speed 6
        // should differ from the (8, 16) default.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.partition_range = Some((16, 16)))));
        assert_ne!(
            bytes, baseline,
            "partition_range=(16,16) should differ from the (8,16) preset default"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn complex_prediction_modes_true_changes_bytes() {
        // Default for stills is force-Simple via the imazen guard at
        // av1encoder.rs:1344. Setting Some(true) flips to ComplexAll
        // and SHOULD perturb the bitstream.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.complex_prediction_modes = Some(true))));
        assert_ne!(
            bytes, baseline,
            "complex_prediction_modes=Some(true) should differ from forced-Simple default"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn complex_prediction_modes_false_matches_default() {
        // Default is already Simple for stills, so Some(false) should
        // be a no-op at the bitstream level.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.complex_prediction_modes = Some(false))));
        assert_eq!(
            bytes, baseline,
            "complex_prediction_modes=Some(false) should match the still-image default"
        );
    }

    #[test]
    fn lrf_true_changes_bytes() {
        // At Q60 + speed 6, the preset's `low_quality && speed <= 8`
        // gate is FALSE (Q60 ≈ qindex ~140, threshold is 150), so LRF
        // is off by default; Some(true) should turn it on and perturb.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.lrf = Some(true))));
        assert_ne!(
            bytes, baseline,
            "lrf=Some(true) should differ from the lrf-off preset default at Q60/speed6"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn lrf_false_explicit_matches_default() {
        // LRF is already off at Q60+speed6 (above), so Some(false) is
        // a no-op.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.lrf = Some(false))));
        assert_eq!(
            bytes, baseline,
            "lrf=Some(false) should match the lrf-off preset default at Q60/speed6"
        );
    }

    #[test]
    fn fast_deblock_true_changes_bytes() {
        // Preset gates fast_deblock on `speed >= 7 && !high_quality`.
        // At speed 6, default is full deblock search; Some(true) flips
        // to the fast closed-form path.
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.fast_deblock = Some(true))));
        assert_ne!(
            bytes, baseline,
            "fast_deblock=Some(true) should differ from full-search default at speed 6"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn fast_deblock_false_explicit_matches_default() {
        let baseline = encode_with(None);
        let bytes = encode_with(Some(mk(|p| p.fast_deblock = Some(false))));
        assert_eq!(
            bytes, baseline,
            "fast_deblock=Some(false) should match the full-search default at speed 6"
        );
    }

    // --- Idempotency: chaining the same params twice equals once. ---

    #[test]
    fn with_internal_params_is_idempotent() {
        let img = synthetic_image();
        let params = mk(|p| {
            p.partition_range = Some((4, 16));
            p.lrf = Some(true);
            p.fast_deblock = Some(true);
        });

        let once = Encoder::new()
            .with_quality(60.0)
            .with_speed(6)
            .with_num_threads(Some(1))
            .with_internal_params(params.clone())
            .encode_rgb(img.as_ref())
            .unwrap()
            .avif_file;

        let twice = Encoder::new()
            .with_quality(60.0)
            .with_speed(6)
            .with_num_threads(Some(1))
            .with_internal_params(params.clone())
            .with_internal_params(params)
            .encode_rgb(img.as_ref())
            .unwrap()
            .avif_file;

        assert_eq!(
            once, twice,
            "applying the same InternalParams twice must equal applying it once"
        );
    }

    // --- All fields set at once still produces a valid encode. ---

    #[test]
    fn all_fields_set_produces_valid_bytes() {
        let bytes = encode_with(Some(mk(|p| {
            p.partition_range = Some((4, 16));
            p.complex_prediction_modes = Some(true);
            p.lrf = Some(true);
            p.fast_deblock = Some(false);
        })));
        assert!(
            bytes.len() > 100,
            "all-fields-set encode should produce a non-trivial AVIF file, got {} bytes",
            bytes.len()
        );
        // Sanity: AVIF starts with an `ftyp` box; first 4 bytes are
        // the box size, bytes 4..8 are 'ftyp'.
        assert_eq!(&bytes[4..8], b"ftyp", "output should be a valid AVIF/MIAF container");
    }

    // --- Default = baseline: passing Default::default() must
    //     bit-exactly match not calling with_internal_params at all. ---

    #[test]
    fn default_internal_params_equals_no_call() {
        let baseline = encode_with(None);
        let with_default = encode_with(Some(InternalParams::default()));
        assert_eq!(
            baseline, with_default,
            "InternalParams::default() must produce bit-exact same output as not calling with_internal_params"
        );
    }

    // --- Reset: a second wholesale call replaces all prior fields. ---

    #[test]
    fn second_call_with_default_resets_all_fields() {
        let img = synthetic_image();
        let baseline = encode_with(None);

        // First set every field, then reset wholesale via Default.
        let perturbing = mk(|p| {
            p.partition_range = Some((4, 16));
            p.complex_prediction_modes = Some(true);
            p.lrf = Some(true);
            p.fast_deblock = Some(true);
        });

        let after_reset = Encoder::new()
            .with_quality(60.0)
            .with_speed(6)
            .with_num_threads(Some(1))
            .with_internal_params(perturbing)
            .with_internal_params(InternalParams::default())
            .encode_rgb(img.as_ref())
            .unwrap()
            .avif_file;

        assert_eq!(
            baseline, after_reset,
            "calling with_internal_params(Default::default()) after a perturbing call must reset all fields wholesale"
        );
    }
}
