use zenravif::{AnimFrame, AnimFrame16, AnimFrameRgba, AnimFrameRgba16, CancellationToken, Encoder, Error, Img, RGB8, RGBA8};

fn check_all_formats(enc: &Encoder<'_>) {
    let rgb8 = vec![RGB8::new(40, 90, 180); 32 * 32];
    let rgba8 = vec![RGBA8::new(40, 90, 180, 128); 32 * 32];
    let rgb16 = vec![rgb::RGB::new(160u16, 360, 720); 32 * 32];
    let rgba16 = vec![rgb::RGBA::new(160u16, 360, 720, 512); 32 * 32];
    let results = [
        enc.encode_animation_rgb(&[AnimFrame { rgb: Img::new(&rgb8, 32, 32), duration_ms: 20 }]),
        enc.encode_animation_rgba(&[AnimFrameRgba { rgba: Img::new(&rgba8, 32, 32), duration_ms: 20 }]),
        enc.encode_animation_rgb16(&[AnimFrame16 { rgb: Img::new(&rgb16, 32, 32), duration_ms: 20 }]),
        enc.encode_animation_rgba16(&[AnimFrameRgba16 { rgba: Img::new(&rgba16, 32, 32), duration_ms: 20 }]),
    ];
    for (format, result) in results.into_iter().enumerate() {
        assert!(matches!(result.as_ref().map_err(|e| e.error()), Err(Error::Cancelled)), "format {format} must return Cancelled, got {:?}", result.map(|_| ()));
    }
}

#[test]
fn animation_honors_cancelled_token_in_every_format() {
    let token = CancellationToken::new();
    token.cancel();
    check_all_formats(&Encoder::new().with_speed(10).with_cancellation_token(token));
}

#[test]
fn animation_honors_zero_timeout_in_every_format() {
    check_all_formats(&Encoder::new().with_speed(10).with_timeout(std::time::Duration::ZERO));
}

#[cfg(feature = "stop")]
#[test]
fn animation_honors_direct_stop_in_every_format() {
    let token = CancellationToken::new();
    token.cancel();
    check_all_formats(&Encoder::new().with_speed(10).with_stop(zenravif::StopToken::new(token)));
}
