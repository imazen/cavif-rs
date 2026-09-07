use imgref::Img;
use rgb::{RGB8, RGBA8};
use zenravif::{AnimFrame, AnimFrameRgba, AnimFrame16, AnimFrameRgba16, Encoder, TimedAnimFrame};

fn boxes<'a>(data: &'a [u8], wanted: &[u8; 4], found: &mut Vec<&'a [u8]>) {
    let mut offset = 0;
    while offset < data.len() {
        assert!(data.len() - offset >= 8);
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && size <= data.len() - offset);
        let kind = &data[offset + 4..offset + 8];
        let payload = &data[offset + 8..offset + size];
        if kind == wanted { found.push(payload); }
        if [b"moov", b"trak", b"mdia", b"minf", b"stbl"].iter().any(|k| kind == *k) {
            boxes(payload, wanted, found);
        }
        offset += size;
    }
}

fn verify(data: &[u8], timescale: u32, durations: &[u32], alpha: bool) {
    let mut media = Vec::new();
    boxes(data, b"mdhd", &mut media);
    assert_eq!(media.len(), if alpha { 2 } else { 1 });
    for payload in media {
        assert_eq!(payload[0], 1);
        assert_eq!(u32::from_be_bytes(payload[20..24].try_into().unwrap()), timescale);
        assert_eq!(u64::from_be_bytes(payload[24..32].try_into().unwrap()), durations.iter().map(|&d| u64::from(d)).sum::<u64>());
    }
    let mut tables = Vec::new();
    boxes(data, b"stts", &mut tables);
    assert_eq!(tables.len(), if alpha { 2 } else { 1 });
    for payload in tables {
        let entries = u32::from_be_bytes(payload[4..8].try_into().unwrap()) as usize;
        assert_eq!(payload.len(), 8 + entries * 8);
        let mut actual = Vec::new();
        for entry in payload[8..].as_chunks::<8>().0 {
            let count = u32::from_be_bytes(entry[..4].try_into().unwrap());
            let delta = u32::from_be_bytes(entry[4..].try_into().unwrap());
            actual.extend(std::iter::repeat_n(delta, count as usize));
        }
        assert_eq!(actual, durations);
    }
}

#[test]
fn exact_ticks_survive_all_animation_input_types() {
    let rgb: Vec<_> = (0..65 * 67).map(|i| RGB8::new((i * 17) as u8, (i * 31) as u8, (i * 7) as u8)).collect();
    let rgba: Vec<_> = rgb.iter().map(|p| RGBA8::new(p.r, p.g, p.b, 128)).collect();
    let high: Vec<_> = rgb.iter().map(|p| rgb::RGB::new(u16::from(p.r) * 4, u16::from(p.g) * 4 + 1, u16::from(p.b) * 4 + 2)).collect();
    let high_alpha: Vec<_> = high.iter().map(|p| rgb::RGBA::new(p.r, p.g, p.b, 511)).collect();
    let enc = Encoder::new().with_speed(10).with_num_threads(Some(1));
    for (timescale, durations) in [(30000, [1001, 2002]), (1_000_000, [1, 7]), (u32::MAX, [u32::MAX, u32::MAX]), (1000, [20, 30])] {
        let a = durations.map(|duration_ticks| TimedAnimFrame { pixels: Img::new(rgb.as_slice(), 65, 67), duration_ticks });
        let b = durations.map(|duration_ticks| TimedAnimFrame { pixels: Img::new(rgba.as_slice(), 65, 67), duration_ticks });
        let c = durations.map(|duration_ticks| TimedAnimFrame { pixels: Img::new(high.as_slice(), 65, 67), duration_ticks });
        let d = durations.map(|duration_ticks| TimedAnimFrame { pixels: Img::new(high_alpha.as_slice(), 65, 67), duration_ticks });
        let results = [enc.encode_animation_rgb_timed(&a, timescale), enc.encode_animation_rgba_timed(&b, timescale), enc.encode_animation_rgb16_timed(&c, timescale), enc.encode_animation_rgba16_timed(&d, timescale)];
        for (index, result) in results.into_iter().enumerate() {
            let result = result.unwrap();
            let total = durations.iter().map(|&v| u64::from(v)).sum::<u64>();
            assert_eq!(result.timescale, timescale);
            assert_eq!(result.total_duration_ticks, total);
            assert_eq!(result.total_duration_ms, (u128::from(total) * 1000 / u128::from(timescale)) as u64);
            verify(&result.avif_file, timescale, &durations, index % 2 != 0);
            if timescale == 1000 {
                let old = match index {
                    0 => enc.encode_animation_rgb(&durations.map(|duration_ms| AnimFrame { rgb: Img::new(rgb.as_slice(), 65, 67), duration_ms })),
                    1 => enc.encode_animation_rgba(&durations.map(|duration_ms| AnimFrameRgba { rgba: Img::new(rgba.as_slice(), 65, 67), duration_ms })),
                    2 => enc.encode_animation_rgb16(&durations.map(|duration_ms| AnimFrame16 { rgb: Img::new(high.as_slice(), 65, 67), duration_ms })),
                    _ => enc.encode_animation_rgba16(&durations.map(|duration_ms| AnimFrameRgba16 { rgba: Img::new(high_alpha.as_slice(), 65, 67), duration_ms })),
                };
                assert_eq!(old.unwrap().avif_file, result.avif_file);
            }
            if let Some(dir) = std::env::var_os("ZENRAVIF_TIMING_ARTIFACTS") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("timing-{timescale}-{index}.avif")), result.avif_file).unwrap();
            }
        }
    }
    let frame = TimedAnimFrame { pixels: Img::new(rgb.as_slice(), 65, 67), duration_ticks: 1 };
    assert!(enc.encode_animation_rgb_timed(std::slice::from_ref(&frame), 0).is_err());
    assert!(enc.encode_animation_rgb_timed(&[TimedAnimFrame { duration_ticks: 0, ..frame }], 1000).is_err());
}
