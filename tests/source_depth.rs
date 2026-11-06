//! cavif source depth: 16-bit samples reach 10-bit AV1 without an 8-bit stage,
//! and narrowing to 8 bits happens only when asked for.
use rav1d_safe::{Decoder, Frame, Planes, Settings};
use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

const WIDTH: usize = 1024;
const HEIGHT: usize = 16;

fn cavif(png: &[u8], args: &[&str]) -> Vec<u8> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cavif"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("-")
        .arg("--speed=10")
        .args(args)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = png.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn decode_obu(obu: &[u8]) -> Frame {
    let mut settings = Settings::default();
    settings.threads = 1;
    settings.max_frame_delay = 1;
    let mut decoder = Decoder::with_settings(settings).unwrap();
    decoder.decode(obu).unwrap().expect("a frame")
}

fn plane_16(frame: &Frame, plane: usize) -> Vec<u16> {
    let Planes::Depth16(p) = frame.planes() else {
        panic!("expected 16-bit storage, got depth {}", frame.bit_depth());
    };
    let view = match plane {
        0 => p.y(),
        1 => p.u().unwrap(),
        _ => p.v().unwrap(),
    };
    (0..HEIGHT)
        .flat_map(|y| view.row(y)[..WIDTH].to_vec())
        .collect()
}

/// A full-scale ramp holding 1024 levels, far more than 8 bits can carry.
fn ramp(x: usize) -> u16 {
    (x * 65535 / (WIDTH - 1)) as u16
}

fn nearest_ten(sample: u16) -> u16 {
    ((u32::from(sample) * 1023 + 32767) / 65535) as u16
}

fn rgb16_png_of(pixel: impl Fn(usize) -> [u16; 3]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(WIDTH * HEIGHT * 6);
    for _ in 0..HEIGHT {
        for x in 0..WIDTH {
            for c in pixel(x) {
                bytes.extend_from_slice(&c.to_be_bytes());
            }
        }
    }
    lodepng::encode_memory(&bytes, WIDTH, HEIGHT, lodepng::ColorType::RGB, 16).unwrap()
}

fn rgb16_png() -> Vec<u8> {
    rgb16_png_of(|x| [ramp(x), ramp(WIDTH - 1 - x), ramp(x) / 2])
}

fn distinct(values: &[u16]) -> usize {
    values.iter().copied().collect::<BTreeSet<_>>().len()
}

#[test]
fn sixteen_bit_rgb_keeps_ten_bits_by_default() {
    let png = rgb16_png();
    let avif = cavif(&png, &["--quality=100", "--color=rgb"]);
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    assert!(parsed.alpha_item.is_none(), "opaque input grew alpha");
    let frame = decode_obu(&parsed.primary_item);
    assert_eq!(frame.bit_depth(), 10);
    // GBR order. An 8-bit stage would leave at most 256 levels per plane.
    let (g, b, r) = (plane_16(&frame, 0), plane_16(&frame, 1), plane_16(&frame, 2));
    assert!(distinct(&r) > 256, "{} red levels", distinct(&r));
    assert!(distinct(&g) > 256, "{} green levels", distinct(&g));
    assert!(distinct(&b) > 256, "{} blue levels", distinct(&b));
    let mut max_error = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = y * WIDTH + x;
            max_error = max_error.max(r[i].abs_diff(nearest_ten(ramp(x))));
            max_error = max_error.max(g[i].abs_diff(nearest_ten(ramp(WIDTH - 1 - x))));
            max_error = max_error.max(b[i].abs_diff(nearest_ten(ramp(x) / 2)));
        }
    }
    assert!(max_error <= 4, "max 10-bit error {max_error}");
}

#[test]
fn sixteen_bit_ycbcr_keeps_ten_bits_by_default() {
    // Neutral, so luma spans the whole ramp.
    let png = rgb16_png_of(|x| [ramp(x), ramp(x), ramp(x)]);
    let avif = cavif(&png, &["--quality=100"]);
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    let frame = decode_obu(&parsed.primary_item);
    assert_eq!(frame.bit_depth(), 10);
    let luma = plane_16(&frame, 0);
    assert!(distinct(&luma) > 256, "{} luma levels", distinct(&luma));
}

#[test]
fn sixteen_bit_gray_and_alpha_keep_ten_bits() {
    let mut bytes = Vec::new();
    for _ in 0..HEIGHT {
        for x in 0..WIDTH {
            for c in [ramp(x), ramp(WIDTH - 1 - x) | 1] {
                bytes.extend_from_slice(&c.to_be_bytes());
            }
        }
    }
    let png =
        lodepng::encode_memory(&bytes, WIDTH, HEIGHT, lodepng::ColorType::GREY_ALPHA, 16).unwrap();
    let avif = cavif(&png, &["--quality=100"]);
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    let alpha = parsed.alpha_item.as_ref().expect("translucent input has alpha");
    let color = decode_obu(&parsed.primary_item);
    assert_eq!(color.bit_depth(), 10);
    let luma = plane_16(&color, 0);
    assert!(distinct(&luma) > 256, "{} luma levels", distinct(&luma));
    let alpha = decode_obu(alpha);
    assert_eq!(alpha.bit_depth(), 10);
    let coverage = plane_16(&alpha, 0);
    assert!(distinct(&coverage) > 256, "{} alpha levels", distinct(&coverage));
}

#[test]
fn eight_bits_only_when_asked() {
    let avif = cavif(&rgb16_png(), &["--quality=100", "--color=rgb", "--depth=8"]);
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    assert_eq!(decode_obu(&parsed.primary_item).bit_depth(), 8);
}

#[test]
fn eight_bit_samples_in_16_bit_storage_encode_like_the_8_bit_image() {
    let eight: Vec<u8> = (0..HEIGHT)
        .flat_map(|_| (0..WIDTH).map(|x| (x / 4) as u8))
        .collect();
    let sixteen: Vec<u8> = eight
        .iter()
        .flat_map(|&v| (u16::from(v) * 257).to_be_bytes())
        .collect();
    let png8 = lodepng::encode_memory(&eight, WIDTH, HEIGHT, lodepng::ColorType::GREY, 8).unwrap();
    let png16 =
        lodepng::encode_memory(&sixteen, WIDTH, HEIGHT, lodepng::ColorType::GREY, 16).unwrap();
    assert_eq!(cavif(&png8, &[]), cavif(&png16, &[]));
}

#[test]
#[ignore = "requires ffmpeg on PATH as an independent decoder"]
fn oracle_ffmpeg_reads_ten_bit_samples() {
    let avif = cavif(&rgb16_png(), &["--quality=100", "--color=rgb"]);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("source_depth_oracle.avif");
    std::fs::write(&path, &avif).unwrap();
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "gbrp10le", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let words: Vec<u16> = output
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    assert_eq!(words.len(), WIDTH * HEIGHT * 3);
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    let frame = decode_obu(&parsed.primary_item);
    let (g, b, r) = (plane_16(&frame, 0), plane_16(&frame, 1), plane_16(&frame, 2));
    let n = WIDTH * HEIGHT;
    assert_eq!(&words[..n], &g[..], "green plane");
    assert_eq!(&words[n..2 * n], &b[..], "blue plane");
    assert_eq!(&words[2 * n..3 * n], &r[..], "red plane");
}

/// Permuted 12-bit codes exercise precision without a smooth-ramp predictor.
fn twelve_bit_sample(x: usize, channel: usize) -> u16 {
    let code = ((x * 4051 + channel * 977 + 17) & 4095) as u32;
    ((code * 65535 + 2047) / 4095) as u16
}

fn twelve_bit_png(source_bits: u32, alpha: bool) -> Vec<u8> {
    let channels = if alpha { 4 } else { 3 };
    let mut bytes = Vec::new();
    for _ in 0..HEIGHT {
        for x in 0..WIDTH {
            for channel in 0..channels {
                let sample = twelve_bit_sample(x, channel);
                if source_bits == 16 {
                    bytes.extend_from_slice(&sample.to_be_bytes());
                } else {
                    bytes.push((sample >> 8) as u8);
                }
            }
        }
    }
    let color = if alpha { lodepng::ColorType::RGBA } else { lodepng::ColorType::RGB };
    lodepng::encode_memory(&bytes, WIDTH, HEIGHT, color, source_bits).unwrap()
}

#[test]
fn twelve_bits_are_explicit_for_both_source_depths_and_color_models() {
    for source_bits in [8, 16] {
        for color in ["rgb", "ycbcr"] {
            let avif = cavif(&twelve_bit_png(source_bits, true),
                &["--quality=100", "--depth=12", "--dirty-alpha", "--color", color]);
            let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
            let frame = decode_obu(&parsed.primary_item);
            assert_eq!(frame.bit_depth(), 12, "{source_bits}-bit {color}");
            let alpha = decode_obu(parsed.alpha_item.as_ref().expect("alpha retained"));
            assert_eq!(alpha.bit_depth(), 12, "alpha {source_bits}-bit {color}");
            for plane in 0..3 {
                assert!(plane_16(&frame, plane).iter().all(|&v| v <= 4095));
            }
            if source_bits == 16 {
                assert!(distinct(&plane_16(&alpha, 0)) > 256);
                if color == "rgb" {
                    for (plane, channel) in [(0, 1), (1, 2), (2, 0)] {
                        let samples = plane_16(&frame, plane);
                        assert!(distinct(&samples) > 256);
                        for (i, &got) in samples.iter().enumerate() {
                            let expected = ((u32::from(twelve_bit_sample(i % WIDTH, channel)) * 4095 + 32767) / 65535) as u16;
                            assert!(got.abs_diff(expected) <= 4, "channel {channel} sample {i}: {got} != {expected}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires ffmpeg on PATH as an independent decoder"]
fn oracle_ffmpeg_reads_twelve_bit_samples() {
    let avif = cavif(&twelve_bit_png(16, false), &["--quality=100", "--color=rgb", "--depth=12"]);
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("source_depth_twelve_oracle.avif");
    std::fs::write(&path, &avif).unwrap();
    let output = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "gbrp12le", "-"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout.len(), WIDTH * HEIGHT * 3 * 2);
    let words: Vec<u16> = output.stdout.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let parsed = avif_parse::read_avif(&mut avif.as_slice()).unwrap();
    let frame = decode_obu(&parsed.primary_item);
    assert_eq!(frame.bit_depth(), 12);
    for plane in 0..3 {
        assert_eq!(&words[plane * WIDTH * HEIGHT..(plane + 1) * WIDTH * HEIGHT], &plane_16(&frame, plane));
    }
}
