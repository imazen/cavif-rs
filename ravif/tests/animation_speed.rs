#![cfg(feature = "__expert")]
use zenravif::{AnimFrame, AnimFrame16, Encoder, Img, RGB8};

#[test]
fn animation_partition_override_changes_color_packets() {
    let pixels = (0..65 * 67).map(|i| RGB8::new((i * 17) as u8, (i * 31) as u8, (i * 7) as u8)).collect::<Vec<_>>();
    let high = pixels.iter().map(|p| rgb::RGB::new(u16::from(p.r) * 4 + 1, u16::from(p.g) * 4 + 2, u16::from(p.b) * 4 + 3)).collect::<Vec<_>>();
    let frames = [20, 30].map(|duration_ms| AnimFrame { rgb: Img::new(pixels.as_slice(), 65, 67), duration_ms });
    let high_frames = [20, 30].map(|duration_ms| AnimFrame16 { rgb: Img::new(high.as_slice(), 65, 67), duration_ms });
    for high_depth in [false, true] {
        let mut files = Vec::new();
        for range in [None, Some((4, 4))] {
            let mut params = zenravif::expert::InternalParams::default();
            params.partition_range = range;
            let enc = Encoder::new().with_speed(10).with_quality(35.0).with_num_threads(Some(1)).with_internal_params(params);
            let result = if high_depth { enc.encode_animation_rgb16(&high_frames) } else { enc.encode_animation_rgb(&frames) }.unwrap();
            files.push(result.avif_file);
        }
        assert!(files[0] != files[1], "partition override ignored: high_depth={high_depth}");
    }
}
