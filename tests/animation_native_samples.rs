use imgref::Img;
use rav1d_safe::{Decoder, Frame, Packet, Planes, ReceiveStatus, SendStatus, Settings};
use zenravif::{BitDepth, ChromaSubsampling, ColorModel, Encoder, TimedAnimFrame};

fn decode(packets: &[Vec<u8>]) -> Vec<Frame> {
    let mut settings = Settings::default();
    settings.threads = 1;
    settings.max_frame_delay = 1;
    settings.apply_grain = false;
    let mut decoder = Decoder::with_settings(settings).unwrap();
    let mut frames = Vec::new();
    for bytes in packets {
        let mut packet = Packet::new(bytes.clone()).unwrap();
        for attempt in 0..16 {
            let status = decoder.send_packet(&mut packet).unwrap();
            while let ReceiveStatus::Frame(frame) = decoder.receive().unwrap() {
                frames.push(frame);
            }
            if matches!(status, SendStatus::Accepted) { break; }
            assert!(attempt < 15, "decoder made no input progress");
        }
    }
    decoder.end_input();
    for _ in 0..16 {
        match decoder.receive().unwrap() {
            ReceiveStatus::Frame(frame) => frames.push(frame),
            ReceiveStatus::EndOfStream => return frames,
            _ => panic!("unexpected drain state"),
        }
    }
    panic!("decoder made no drain progress");
}

#[test]
fn native_animation_preserves_twelve_bit_rgb_and_alpha_codes() {
    for (w,h) in [(1,1),(17,13)] {
        for depth in [10u8,12] {
            let max = (1u16 << depth)-1;
            let originals:Vec<Vec<rgb::RGBA<u16>>>=(0..3).map(|frame| (0..w*h)
                .map(|i| rgb::RGBA::new(
                    (i as u16 * 73 + 517 + frame*11)&max,
                    (i as u16 * 113 + 259 + frame*17)&max,
                    ((i as u32 * 191 + 3 + u32::from(frame)*23)&u32::from(max)) as u16,
                    (i as u16 * 31 + 513 + frame*7)&max)).collect()).collect();
            let frames:Vec<_>=originals.iter().map(|p| TimedAnimFrame { pixels:Img::new(p.as_slice(),w,h), duration_ticks:1001 }).collect();
            let encoded=Encoder::new().with_speed(10).with_num_threads(Some(1))
                .with_lossless(true).with_bit_depth(if depth==12 { BitDepth::Twelve } else { BitDepth::Ten })
                .with_internal_color_model(ColorModel::RGB).with_chroma_subsampling(ChromaSubsampling::Yuv444)
                .encode_animation_rgba16_timed(&frames,30000).unwrap();
            let parser=zenavif_parse::AvifParser::from_bytes(&encoded.avif_file).unwrap();
            let packets:Vec<_>=parser.frames().map(Result::unwrap).collect();
            let color=decode(&packets.iter().map(|p|p.data.to_vec()).collect::<Vec<_>>());
            let alpha=decode(&packets.iter().map(|p|p.alpha_data.as_ref().unwrap().to_vec()).collect::<Vec<_>>());
            assert_eq!(color.len(),3);
            assert_eq!(alpha.len(),3);
            for (index,(color,alpha)) in color.iter().zip(&alpha).enumerate() {
                assert_eq!(color.bit_depth(),depth);
                assert_eq!(alpha.bit_depth(),depth);
                let Planes::Depth16(c)=color.planes() else { panic!("color storage") };
                let Planes::Depth16(a)=alpha.planes() else { panic!("alpha storage") };
                for (component,plane) in [c.v().unwrap(),c.y(),c.u().unwrap(),a.y()].iter().enumerate() {
                    for (y,row) in plane.rows().enumerate() {
                        for (x,&code) in row.iter().enumerate() {
                            let p=originals[index][y*w+x];
                            assert_eq!(code,[p.r,p.g,p.b,p.a][component],"{w}x{h} depth{depth} frame{index} channel{component} ({x},{y})");
                        }
                    }
                }
            }
        }
    }
}
