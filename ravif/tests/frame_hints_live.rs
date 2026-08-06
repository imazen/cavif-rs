//! Does a supplied per-superblock quantizer map actually reach the encoder?
//!
//! This is the gate that `FRAME_HINTS_LIVE` claims to answer. Before the
//! zenrav1e dep moved past 0.1.4 the answer was NO: `InternalParams::sb_q_scale`
//! was accepted and then silently dropped, so a closed-loop caller paid for a
//! full second encode that could not steer a single bit. The constant existed so
//! callers could detect that and fail honestly.
//!
//! A boolean constant is not evidence, though — it is a claim about the build.
//! These tests check the behaviour it claims, so the constant cannot drift away
//! from reality: if someone flips it without the dep, or bumps the dep and
//! forgets the flip, one of these fails.

#![cfg(feature = "__expert")]

use zenravif::expert::InternalParams;
use zenravif::Encoder;
use rgb::RGBA8;

const W: usize = 256;
const H: usize = 256;

/// High-frequency content on purpose: a flat or smooth image quantizes to
/// almost nothing, and then even a real quantizer change moves no bytes, which
/// would make this test silently vacuous.
fn source() -> imgref::ImgVec<RGBA8> {
    let mut px = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            px.push(RGBA8 {
                r: ((x * 7 + y * 3) % 251) as u8,
                g: ((x ^ y) % 253) as u8,
                b: ((x.wrapping_mul(y) >> 3) % 249) as u8,
                a: 255,
            });
        }
    }
    imgref::Img::new(px, W, H)
}

/// Superblocks are 64x64, and the map is one entry per superblock in raster
/// order: `ceil(ceil(w/8)/8)` columns by `ceil(ceil(h/8)/8)` rows.
fn sb_count(w: usize, h: usize) -> usize {
    let cols = w.div_ceil(8).div_ceil(8);
    let rows = h.div_ceil(8).div_ceil(8);
    cols * rows
}

fn encode(map: Option<Box<[f32]>>) -> Vec<u8> {
    // `InternalParams` is #[non_exhaustive], so it is built by mutating the
    // default rather than with a struct literal — same as zenavif's own
    // closed-loop caller does.
    let mut params = InternalParams::default();
    params.sb_q_scale = map;
    let enc = Encoder::new()
        .with_quality(60.0)
        .with_speed(8)
        .with_internal_params(params);
    enc.encode_rgba(source().as_ref())
        .expect("encode succeeds")
        .avif_file
}

#[test]
fn a_non_neutral_sb_q_scale_map_changes_the_bitstream() {
    let n = sb_count(W, H);

    // Half the superblocks strongly finer, half strongly coarser. A map this
    // aggressive must move bytes if it is applied at all; asserting only on a
    // subtle map would make a silent no-op hard to distinguish from noise.
    let skewed: Box<[f32]> = (0..n)
        .map(|i| if i % 2 == 0 { 0.5 } else { 2.0 })
        .collect();

    let baseline = encode(None);
    let hinted = encode(Some(skewed));

    assert!(!baseline.is_empty() && !hinted.is_empty());
    assert_ne!(
        baseline, hinted,
        "a per-SB quantizer map of alternating 0.5/2.0 produced a byte-identical \
         encode — the map is being accepted and discarded, which is exactly the \
         state FRAME_HINTS_LIVE=false described. Either the zenrav1e dep lost \
         FrameHints support, or the hinted send in av1encoder::encode_to_av1 \
         stopped being taken."
    );
}

#[test]
fn an_all_neutral_map_is_equivalent_to_no_map() {
    // 1.0 is documented as neutral. If a neutral map changed the output, the
    // scale would be applied with an off-by-something and every closed-loop
    // second pass would pay a quality shift it never asked for.
    let neutral: Box<[f32]> = vec![1.0f32; sb_count(W, H)].into_boxed_slice();
    assert_eq!(
        encode(None),
        encode(Some(neutral)),
        "an all-1.0 map must be a no-op; it is documented as neutral"
    );
}

#[test]
fn a_wrongly_sized_map_is_ignored_rather_than_misapplied() {
    // zenrav1e documents that maps not matching the superblock grid are
    // ignored. Silently applying a mis-sized map would smear the wrong scales
    // across the wrong superblocks — wrong pixels, no error.
    let too_short: Box<[f32]> = vec![0.5f32; 1].into_boxed_slice();
    assert_eq!(
        encode(None),
        encode(Some(too_short)),
        "a map whose length does not match the SB grid must be ignored, not \
         applied to a prefix of the frame"
    );
}

#[test]
fn frame_hints_live_agrees_with_observed_behaviour() {
    // Keep the advertised constant honest against what the build actually does.
    let n = sb_count(W, H);
    let skewed: Box<[f32]> = (0..n).map(|i| if i % 2 == 0 { 0.5 } else { 2.0 }).collect();
    let moved = encode(None) != encode(Some(skewed));
    assert_eq!(
        zenravif::FRAME_HINTS_LIVE,
        moved,
        "FRAME_HINTS_LIVE says {} but a non-neutral map {} the bitstream. \
         Callers branch on this constant to decide whether a spatial second \
         pass is worth running, so a stale value costs them a wasted encode \
         (or a silently ineffective one).",
        zenravif::FRAME_HINTS_LIVE,
        if moved { "DID change" } else { "did NOT change" }
    );
}
