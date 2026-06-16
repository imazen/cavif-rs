//! Pre-flight pixel-cap guard for the encode entry points.
//!
//! The audit recorded in the proderrdoc found that zenravif's encode paths
//! forced zenrav1e's `max_pixel_count` to `u64::MAX` (nulling rav1e's own
//! guard) and had no zenravif-side dimension cap, so a server passing
//! attacker-controlled `w x h` got no pre-flight rejection.
//!
//! These tests exercise the dimension-only early-rejection path: they pass
//! oversized `width`/`height` to `encode_raw_planes_8_bit` together with an
//! **empty** plane iterator. The pre-flight `check_pixel_limit` runs at the
//! top of `encode_raw_planes_internal`, before the iterator is consumed or
//! any rav1e context is built, so no large buffer is ever allocated.
//!
//! What this covers:
//! - default 120 MP cap rejects an oversized request with `Error::TooManyPixels`
//!   (pre-flight — the empty iterator is never read);
//! - a small (<= 120 MP) request is NOT rejected by the cap (it proceeds past
//!   the guard and actually encodes a 2x2 image successfully);
//! - the opt-out (`with_max_pixels(0)`) lets an oversized *declaration* through
//!   the guard.
//!
//! What this does NOT cover: it does not encode a real 120 MP buffer (that would
//! defeat the point of a pre-flight guard) and does not assert anything about
//! zenrav1e's internal guard beyond the fact that zenravif forwards the cap to
//! it (verified separately by grep, since the field is forwarded, not asserted
//! here).

use zenravif::{Encoder, Error, MatrixCoefficients, PixelRange};

/// `12000 * 12000 = 144_000_000` pixels — above the 120 MP default cap.
const OVERSIZED_W: usize = 12_000;
const OVERSIZED_H: usize = 12_000;

/// An empty 8-bit plane iterator. The pre-flight cap check rejects oversized
/// dimensions before this is ever consumed, so it never needs to be the right
/// length — that is exactly the early-rejection property under test.
fn empty_planes() -> impl IntoIterator<Item = [u8; 3]> + Send {
    std::iter::empty::<[u8; 3]>()
}

#[test]
fn oversized_dimensions_rejected_preflight_by_default() {
    let enc = Encoder::new(); // default cap = 120 MP
    let result = enc.encode_raw_planes_8_bit(
        OVERSIZED_W,
        OVERSIZED_H,
        empty_planes(),
        None::<[u8; 0]>,
        PixelRange::Full,
        MatrixCoefficients::BT601,
    );
    match result {
        Err(Error::TooManyPixels {
            width,
            height,
            max_pixels,
        }) => {
            assert_eq!(width, OVERSIZED_W);
            assert_eq!(height, OVERSIZED_H);
            assert_eq!(max_pixels, 120_000_000);
        }
        other => panic!("expected Error::TooManyPixels, got {other:?}"),
    }
}

#[test]
fn small_image_under_cap_encodes_ok() {
    // 16x16 = 256 pixels, far under the 120 MP cap. A real (correctly sized)
    // plane buffer so the encode actually proceeds past the guard and produces
    // a bitstream — proving the cap does not block normal-sized images.
    let enc = Encoder::new().with_speed(10).with_num_threads(Some(1));
    let planes = vec![[16u8, 128, 128]; 16 * 16]; // 16x16 mid-gray YCbCr
    let result = enc.encode_raw_planes_8_bit(
        16,
        16,
        planes,
        None::<[u8; 0]>,
        PixelRange::Full,
        MatrixCoefficients::BT601,
    );
    let encoded = result.expect("16x16 encode under the cap should succeed");
    assert!(
        !encoded.avif_file.is_empty(),
        "encoded AVIF should be non-empty"
    );
}

#[test]
fn opt_out_zero_allows_oversized_declaration() {
    // with_max_pixels(0) disables the cap. The oversized *declaration* must
    // get past the pre-flight guard. We pass an empty plane iterator so the
    // encode then fails downstream for a different reason (not the cap) — the
    // point is only that it is NOT rejected with Error::TooManyPixels.
    let enc = Encoder::new().with_max_pixels(0);
    let result = enc.encode_raw_planes_8_bit(
        OVERSIZED_W,
        OVERSIZED_H,
        empty_planes(),
        None::<[u8; 0]>,
        PixelRange::Full,
        MatrixCoefficients::BT601,
    );
    assert!(
        !matches!(result, Err(Error::TooManyPixels { .. })),
        "with_max_pixels(0) must disable the cap, so the oversized request \
         must not be rejected with TooManyPixels; got {result:?}"
    );
}

#[test]
fn custom_cap_below_default_rejects_between() {
    // A cap of 1 MP must reject a 2 MP declaration that the default 120 MP cap
    // would have allowed — proves the cap value is actually honored, not a
    // hardcoded 120 MP.
    let enc = Encoder::new().with_max_pixels(1_000_000); // 1 MP
    let result = enc.encode_raw_planes_8_bit(
        2000,
        1000, // 2_000_000 pixels > 1 MP cap, < 120 MP default
        empty_planes(),
        None::<[u8; 0]>,
        PixelRange::Full,
        MatrixCoefficients::BT601,
    );
    assert!(
        matches!(result, Err(Error::TooManyPixels { max_pixels, .. }) if max_pixels == 1_000_000),
        "1 MP cap must reject a 2 MP request with TooManyPixels(max=1_000_000); got {result:?}"
    );
}
