//! `Encoder::encode_gray8` — true monochrome (Cs400) AVIF encoding
//! (imazen/zenavif#6).
//!
//! `avif_parse::primary_item_metadata()` parses the AV1 sequence header out
//! of the payload, so `monochrome == true` here proves the *bitstream* was
//! coded Cs400 (luma only), not merely that the container claims mono.
//! Pixel-exact decode verification lives on the zenavif side, which owns the
//! gray decode path (rav1d-safe).

use zenravif::{BitDepth, Encoder};

/// Gradient + texture so the encoder has real luma structure to code.
fn gray_image(w: usize, h: usize) -> imgref::ImgVec<u8> {
    imgref::ImgVec::new(
        (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    let base = (16 + (x * 2 + y) % 224) as u8;
                    if (x / 4 + y / 4) % 2 == 0 { base.saturating_add(24) } else { base }
                })
            })
            .collect(),
        w,
        h,
    )
}

#[test]
fn gray8_encodes_true_monochrome_av1() {
    let img = gray_image(96, 64);
    let enc = Encoder::new()
        .with_quality(60.0)
        .with_speed(10)
        // Default output depth is Auto = 10-bit (matches encode_rgb);
        // pin 8-bit explicitly for this contract.
        .with_bit_depth(BitDepth::Eight)
        .with_num_threads(Some(1));
    let result = enc.encode_gray8(img.as_ref()).unwrap();
    assert!(result.color_byte_size > 0);
    assert_eq!(result.alpha_byte_size, 0);

    let parsed = avif_parse::read_avif(&mut result.avif_file.as_slice()).unwrap();
    let md = parsed.primary_item_metadata().unwrap();
    assert!(md.monochrome, "sequence header must signal monochrome (Cs400)");
    // avif-parse reports (false, false) for mono streams: the subsampling
    // bits are not coded when mono_chrome = 1 (AV1 spec treats them as
    // implicitly 1; the container av1C carries 1,1 via set_monochrome).
    assert_eq!(md.chroma_subsampling, (false, false));
    assert_eq!(md.bit_depth, 8);
    assert_eq!(md.max_frame_width.get(), 96);
    assert_eq!(md.max_frame_height.get(), 64);
    assert!(md.seq_profile == 0 || md.seq_profile == 2, "mono is profile 0/2");
}

#[test]
fn gray8_ten_bit_output_widens_depth() {
    let img = gray_image(64, 48);
    let enc = Encoder::new()
        .with_quality(60.0)
        .with_speed(10)
        .with_bit_depth(BitDepth::Ten)
        .with_num_threads(Some(1));
    let result = enc.encode_gray8(img.as_ref()).unwrap();

    let parsed = avif_parse::read_avif(&mut result.avif_file.as_slice()).unwrap();
    let md = parsed.primary_item_metadata().unwrap();
    assert!(md.monochrome);
    assert_eq!(md.bit_depth, 10);
}

/// Odd dimensions exercise the mono plane edge handling (no chroma halving
/// involved, but the luma plane geometry must still round-trip).
#[test]
fn gray8_odd_dimensions() {
    let img = gray_image(129, 101);
    let enc = Encoder::new()
        .with_quality(60.0)
        .with_speed(10)
        .with_num_threads(Some(1));
    let result = enc.encode_gray8(img.as_ref()).unwrap();

    let parsed = avif_parse::read_avif(&mut result.avif_file.as_slice()).unwrap();
    let md = parsed.primary_item_metadata().unwrap();
    assert!(md.monochrome);
    assert_eq!(md.max_frame_width.get(), 129);
    assert_eq!(md.max_frame_height.get(), 101);
}
