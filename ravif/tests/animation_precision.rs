#![cfg(feature = "imazen")]
use imgref::Img;
use zenravif::{BitDepth, ChromaSubsampling, ColorModel, Encoder, PixelRange, TimedAnimFrame};

#[test]
fn twelve_bit_animation_signals_profile_two_in_both_tracks() {
    for (width,height) in [(1,1),(17,13)] {
        let rgb:Vec<_>=(0..width*height).map(|i|rgb::RGB::new(((i*73+2053)&4095) as u16,((i*113+1027)&4095) as u16,((i*191+3)&4095) as u16)).collect();
        let rgba:Vec<_>=rgb.iter().enumerate().map(|(i,p)|rgb::RGBA::new(p.r,p.g,p.b,((i*31+2049)&4095) as u16)).collect();
        for (name,chroma,model,range) in [
            ("rgb",ChromaSubsampling::Yuv444,ColorModel::RGB,PixelRange::Full),
            ("444full",ChromaSubsampling::Yuv444,ColorModel::YCbCr,PixelRange::Full),
            ("420full",ChromaSubsampling::Yuv420,ColorModel::YCbCr,PixelRange::Full),
            ("420limited",ChromaSubsampling::Yuv420,ColorModel::YCbCr,PixelRange::Limited),
        ] {
            let enc=Encoder::new().with_speed(10).with_num_threads(Some(1)).with_lossless(true)
                .with_bit_depth(BitDepth::Twelve).with_chroma_subsampling(chroma)
                .with_internal_color_model(model).with_pixel_range(range);
            let frames=[1001,2002].map(|duration_ticks|TimedAnimFrame { pixels:Img::new(rgb.as_slice(),width,height),duration_ticks });
            let alpha_frames=[1001,2002].map(|duration_ticks|TimedAnimFrame { pixels:Img::new(rgba.as_slice(),width,height),duration_ticks });
            for (alpha,encoded) in [(false,enc.encode_animation_rgb16_timed(&frames,30000)),(true,enc.encode_animation_rgba16_timed(&alpha_frames,30000))] {
                let output=encoded.unwrap();
                let parser=zenavif_parse::AvifParser::from_bytes(&output.avif_file).unwrap();
                let config=parser.av1_config().unwrap();
                assert_eq!(config.bit_depth,12,"{width}x{height}/{name}/alpha={alpha}");
                assert_eq!(config.profile,2);
                let decoded:Vec<_>=parser.frames().map(Result::unwrap).collect();
                assert_eq!(decoded.len(),2);
                assert_eq!(output.timescale,30000);
                assert_eq!(output.total_duration_ticks,3003);
                if let Some(directory)=std::env::var_os("ZENRAVIF_PRECISION_ARTIFACTS") {
                    let directory=std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let stem=format!("{width}x{height}-{name}-alpha{alpha}");
                    std::fs::write(directory.join(format!("{stem}.avif")),&output.avif_file).unwrap();
                    let color:Vec<_>=decoded.iter().flat_map(|f|f.data.iter().copied()).collect();
                    std::fs::write(directory.join(format!("{stem}.obu")),color).unwrap();
                }
            }
        }
    }
}

#[test]
fn native_animation_rejects_codes_outside_the_declared_precision() {
    let rgb=[rgb::RGB::new(4096u16,1,2)];
    let rgba=[rgb::RGBA::new(1u16,2,3,4096)];
    let enc=Encoder::new().with_speed(10).with_bit_depth(BitDepth::Twelve);
    assert!(enc.encode_animation_rgb16_timed(&[TimedAnimFrame { pixels:Img::new(rgb.as_slice(),1,1),duration_ticks:1 }],30).is_err());
    assert!(enc.encode_animation_rgba16_timed(&[TimedAnimFrame { pixels:Img::new(rgba.as_slice(),1,1),duration_ticks:1 }],30).is_err());
}
