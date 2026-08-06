//! Reproduces zenavif `tests/hdr_roundtrip.rs::pq10_pixel_fidelity_within_bounds`
//! inside this repo, so a speed-table change can be attributed to a specific
//! arm without building the downstream crate.
//!
//! Same fixture (64x48 HDR-ish u16), same encode path (`encode_raw_planes_10_bit`
//! with `MatrixCoefficients::Identity`, GBR plane order, full range, q95/s8),
//! same reconstruction (16->10 is `v >> 6`, 10->16 is `(v << 6) | (v >> 4)`),
//! same statistic (max and mean |delta| in 16-bit units over all three
//! channels).
//!
//! `--dump` additionally prints where the tail lives: the worst cells, their
//! source values, and a per-row / per-region breakdown — so "the tail moved"
//! can be told apart from "a region is structurally wrong".

use rav1d_safe::{Decoder, Planes, Settings};
use zenravif::{Encoder, MatrixCoefficients, PixelRange};

const W: usize = 64;
const H: usize = 48;

/// The zenavif fixture, verbatim.
fn make_hdr16() -> Vec<[u16; 3]> {
    let mut px = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let p: [u16; 3] = if y < 16 {
                let v = (x * 65535 / (W - 1)) as u16;
                [v, v, v]
            } else if y < 32 {
                match x / 16 {
                    0 => [60000, 4000, 4000],
                    1 => [4000, 60000, 4000],
                    2 => [4000, 4000, 60000],
                    _ => [62000, 62000, 62000],
                }
            } else if x % 13 == 0 && y % 5 == 0 {
                [65535, 65535, 65535]
            } else {
                let v = 1200 + ((x * 7 + y * 11) % 64) as u16 * 8;
                [v, v / 2, v / 3]
            };
            px.push(p);
        }
    }
    px
}

fn scale_from_u16(v: u16) -> u16 {
    v >> 6
}
fn scale_to_u16(v: u16) -> u16 {
    (v << 6) | (v >> 4)
}

fn region(y: usize) -> &'static str {
    if y < 16 {
        "ramp"
    } else if y < 32 {
        "patches"
    } else {
        "dark+specular"
    }
}

fn main() {
    let dump = std::env::args().any(|a| a == "--dump");
    let quality: f32 = std::env::args()
        .position(|a| a == "--quality")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(95.0);
    let speed: u8 = std::env::args()
        .position(|a| a == "--speed")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let src = make_hdr16();
    // GBR plane order for MC=0 identity, exactly as zenavif::encode_rgb16 does.
    let planes: Vec<[u16; 3]> = src
        .iter()
        .map(|p| [scale_from_u16(p[1]), scale_from_u16(p[2]), scale_from_u16(p[0])])
        .collect();

    let enc = Encoder::new()
        .with_quality(quality)
        .with_speed(speed)
        .with_num_threads(Some(1));
    let out = enc
        .encode_raw_planes_10_bit(
            W,
            H,
            planes,
            None::<std::iter::Empty<u16>>,
            PixelRange::Full,
            MatrixCoefficients::Identity,
        )
        .expect("PQ10 encode");

    let parsed = avif_parse::read_avif(&mut &out.avif_file[..]).expect("parse own output");
    let mut settings = Settings::default();
    settings.threads = 1;
    settings.max_frame_delay = 1;
    let mut dec = Decoder::with_settings(settings).expect("decoder");
    let frame = dec
        .decode(&parsed.primary_item[..])
        .expect("decode")
        .expect("a frame");
    assert_eq!(frame.bit_depth(), 10, "must be a 10-bit stream");

    let Planes::Depth16(p) = frame.planes() else {
        panic!("expected 10-bit planes")
    };
    let (g, b, r) = (p.y(), p.u().expect("U plane"), p.v().expect("V plane"));

    let mut max_diff = 0u32;
    let mut sum_diff = 0u64;
    // (diff, x, y, channel, src, decoded)
    let mut worst: Vec<(u32, usize, usize, char, u16, u16)> = Vec::new();
    let mut per_region: [(u32, u64, u64); 3] = [(0, 0, 0); 3];

    for y in 0..H {
        let (gr, br, rr) = (g.row(y), b.row(y), r.row(y));
        for x in 0..W {
            let s = src[y * W + x];
            let dec_rgb = [scale_to_u16(rr[x]), scale_to_u16(gr[x]), scale_to_u16(br[x])];
            for (c, (&sv, &dv)) in ['r', 'g', 'b'].iter().zip(s.iter().zip(dec_rgb.iter())) {
                let d = (i32::from(sv) - i32::from(dv)).unsigned_abs();
                max_diff = max_diff.max(d);
                sum_diff += u64::from(d);
                let ri = if y < 16 {
                    0
                } else if y < 32 {
                    1
                } else {
                    2
                };
                per_region[ri].0 = per_region[ri].0.max(d);
                per_region[ri].1 += u64::from(d);
                per_region[ri].2 += 1;
                worst.push((d, x, y, *c, sv, dv));
            }
        }
    }
    let n = (W * H * 3) as u64;
    let mean = sum_diff / n;
    println!(
        "q{quality} s{speed}: max |D| = {max_diff}, mean |D| = {mean} (16-bit units), {} bytes",
        out.avif_file.len()
    );

    if dump {
        worst.sort_unstable_by_key(|w| std::cmp::Reverse(w.0));
        println!("\n  worst 24 cells (diff, x, y, chan, src -> decoded, region):");
        for &(d, x, y, c, sv, dv) in worst.iter().take(24) {
            println!(
                "    {d:6}  ({x:2},{y:2}) {c}  {sv:5} -> {dv:5}   {}",
                region(y)
            );
        }
        println!("\n  per region (max, mean, n):");
        for (i, name) in ["ramp (y 0..16)", "patches (y 16..32)", "dark+spec (y 32..48)"]
            .iter()
            .enumerate()
        {
            let (mx, sm, cnt) = per_region[i];
            println!("    {name:24} max {mx:6}  mean {:5}  n {cnt}", sm / cnt);
        }
        // How many cells exceed the test's 900 budget, and are they the
        // specular lattice?
        let over: Vec<_> = worst.iter().filter(|w| w.0 > 900).collect();
        println!("\n  cells over the 900 budget: {}", over.len());
        let specular = over
            .iter()
            .filter(|w| w.1 % 13 == 0 && w.2 % 5 == 0 && w.2 >= 32)
            .count();
        println!("    of those, on the specular lattice (x%13==0, y%5==0, y>=32): {specular}");
    }
}
