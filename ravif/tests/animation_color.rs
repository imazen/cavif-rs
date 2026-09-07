#![cfg(feature = "imazen")]
use imgref::Img;
use rgb::{RGB8, RGBA8};
use zenravif::{Encoder, TimedAnimFrame, ChromaSubsampling, ColorModel, PixelRange};

#[test]
fn animation_color_formats_are_encoded_and_exported() {
    let rgb: Vec<_> = (0..65 * 67).map(|i| RGB8::new((i * 17) as u8, (i * 31) as u8, (i * 7) as u8)).collect();
    let rgba: Vec<_> = rgb.iter().map(|p| RGBA8::new(p.r, p.g, p.b, 128)).collect();
    let high: Vec<_> = rgb.iter().map(|p| rgb::RGB::new(u16::from(p.r) * 4, u16::from(p.g) * 4 + 1, u16::from(p.b) * 4 + 2)).collect();
    let high_alpha: Vec<_> = high.iter().map(|p| rgb::RGBA::new(p.r, p.g, p.b, 511)).collect();
    for (name, chroma, model, range) in [
        ("420full", ChromaSubsampling::Yuv420, ColorModel::YCbCr, PixelRange::Full),
        ("420limited", ChromaSubsampling::Yuv420, ColorModel::YCbCr, PixelRange::Limited),
        ("444full", ChromaSubsampling::Yuv444, ColorModel::YCbCr, PixelRange::Full),
        ("444limited", ChromaSubsampling::Yuv444, ColorModel::YCbCr, PixelRange::Limited),
        ("rgb", ChromaSubsampling::Yuv444, ColorModel::RGB, PixelRange::Full),
    ] {
        let enc = Encoder::new().with_speed(10).with_num_threads(Some(1)).with_lossless(true)
            .with_chroma_subsampling(chroma).with_internal_color_model(model).with_pixel_range(range);
        let a = [1001, 2002].map(|duration_ticks| TimedAnimFrame { pixels: Img::new(rgb.as_slice(), 65, 67), duration_ticks });
        let b = [1001, 2002].map(|duration_ticks| TimedAnimFrame { pixels: Img::new(rgba.as_slice(), 65, 67), duration_ticks });
        let c = [1001, 2002].map(|duration_ticks| TimedAnimFrame { pixels: Img::new(high.as_slice(), 65, 67), duration_ticks });
        let d = [1001, 2002].map(|duration_ticks| TimedAnimFrame { pixels: Img::new(high_alpha.as_slice(), 65, 67), duration_ticks });
        let results = [enc.encode_animation_rgb_timed(&a, 30000), enc.encode_animation_rgba_timed(&b, 30000), enc.encode_animation_rgb16_timed(&c, 30000), enc.encode_animation_rgba16_timed(&d, 30000)];
        for (kind, result) in results.into_iter().enumerate() {
            let result = result.unwrap();
            let parser = zenavif_parse::AvifParser::from_bytes(&result.avif_file).unwrap();
            let config = parser.av1_config().unwrap();
            let subsampled = u8::from(chroma == ChromaSubsampling::Yuv420);
            assert_eq!(config.profile, 1 - subsampled, "{name}/{kind}");
            assert_eq!((config.chroma_subsampling_x, config.chroma_subsampling_y), (subsampled, subsampled), "{name}/{kind}");
            assert_eq!(config.bit_depth, if kind < 2 { 8 } else { 10 });
            assert!(!config.monochrome);
            let Some(zenavif_parse::ColorInformation::Nclx { matrix_coefficients, full_range, .. }) = parser.color_info() else { panic!("missing color signaling: {name}/{kind}") };
            assert_eq!(*matrix_coefficients, if model == ColorModel::RGB { 0 } else { 6 });
            assert_eq!(*full_range, range == PixelRange::Full);
            let frames: Vec<_> = parser.frames().map(Result::unwrap).collect();
            assert_eq!(frames.len(), 2);
            if let Some(dir) = std::env::var_os("ZENRAVIF_COLOR_ARTIFACTS") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let stem = format!("color-{name}-{kind}");
                std::fs::write(dir.join(format!("{stem}.avif")), &result.avif_file).unwrap();
                let color: Vec<_> = frames.iter().flat_map(|f| f.data.iter().copied()).collect();
                std::fs::write(dir.join(format!("{stem}.obu")), color).unwrap();
                if name == "rgb" {
                    let mut source = Vec::new();
                    for _ in 0..2 { for plane in 0..3 { for i in 0..65 * 67 {
                        let value = if kind < 2 { let p = rgb[i]; [u16::from(p.g), u16::from(p.b), u16::from(p.r)][plane] }
                            else { let p = high[i]; [p.g, p.b, p.r][plane] };
                        if kind < 2 { source.push(value as u8); } else { source.extend_from_slice(&value.to_le_bytes()); }
                    } } }
                    std::fs::write(dir.join(format!("{stem}.source.yuv")), source).unwrap();
                }
            }
        }
    }
}
