//! Composed-arm A/B sweep harness for the ravif speed-table release gates.
//!
//! Encodes a fixed content×size×quality×speed grid with the *current* build of
//! `zenravif`, decodes each result back through `rav1d-safe`, and scores it with
//! SSIMULACRA2. Emits one TSV row per cell. Run it once on the pre-flip build
//! and once on the post-flip build, then join the two TSVs on the cell key:
//! byte-identical rows prove a gate's apply block is still dead, and the
//! bytes/ssim2 deltas are the measured effect of the arms that did fire.
//!
//! Sweep discipline (workspace CLAUDE.md): four size tiers (tiny / small /
//! medium / large), quality 5..=100 step 5 with *equal* density at low and high
//! q, five content classes, and every speed row a gate touches plus an
//! untouched control row.
//!
//! ```text
//! cargo run --release --example gate_sweep -- \
//!     --label pre --out ~/tmp/pre.tsv \
//!     --sizes 64,256,1024,2048 --speeds 1,2,4,6,8,9,10 --qstep 5
//! ```
//!
//! Sources come from the local `codec-corpus` checkout (`--corpus`, default
//! `../codec-corpus`). Sizes above a source's native long edge are skipped —
//! upscaled sources are synthetic detail and would poison the RD numbers.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use imgref::{Img, ImgVec};
use rav1d_safe::{Decoder, Planes, Settings};
use rgb::RGB8;
use zenravif::Encoder;
use zenresize::{Filter, PixelDescriptor, ResizeConfig, Resizer};

/// One source per content class. Long edges: 2048 / 2048 / 2940 / 2560 / 2560.
const SOURCES: &[(&str, &str)] = &[
    ("photo_a", "clic2025/training/0c49a5cce349020bbba2f97ae41e90ba.png"),
    ("photo_b", "clic2025/training/100a02c269c5948392f283b2aa3bb4da.png"),
    ("screen", "gb82-sc/imac_dark.png"),
    ("text_ui", "gb82-sc/codec_wiki.png"),
    ("lineart_ui", "gb82-sc/windows.png"),
];

struct Args {
    label: String,
    out: PathBuf,
    corpus: PathBuf,
    sizes: Vec<u32>,
    speeds: Vec<u8>,
    qmin: u32,
    qmax: u32,
    qstep: u32,
    threads: usize,
    only: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        label: "run".into(),
        out: PathBuf::from("gate_sweep.tsv"),
        corpus: PathBuf::from("../codec-corpus"),
        sizes: vec![64, 256, 1024, 2048],
        speeds: vec![1, 2, 4, 6, 8, 9, 10],
        qmin: 5,
        qmax: 100,
        qstep: 5,
        threads: 1,
        only: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().expect("missing value");
        match k.as_str() {
            "--label" => a.label = v(),
            "--out" => a.out = PathBuf::from(v()),
            "--corpus" => a.corpus = PathBuf::from(v()),
            "--sizes" => a.sizes = v().split(',').map(|s| s.parse().unwrap()).collect(),
            "--speeds" => a.speeds = v().split(',').map(|s| s.parse().unwrap()).collect(),
            "--qmin" => a.qmin = v().parse().unwrap(),
            "--qmax" => a.qmax = v().parse().unwrap(),
            "--qstep" => a.qstep = v().parse().unwrap(),
            "--threads" => a.threads = v().parse().unwrap(),
            "--only" => a.only = Some(v()),
            other => panic!("unknown arg {other}"),
        }
    }
    a
}

/// Decode a source file to packed RGB8 (alpha composited away is not needed —
/// every corpus source used here is opaque).
fn load_source(path: &Path) -> ImgVec<RGB8> {
    let loaded = load_image::load_path(path).expect("load source image");
    let (w, h) = (loaded.width, loaded.height);
    let px: Vec<RGB8> = match loaded.bitmap {
        load_image::ImageData::RGB8(v) => v.iter().map(|p| RGB8::new(p.r, p.g, p.b)).collect(),
        load_image::ImageData::RGBA8(v) => v.iter().map(|p| RGB8::new(p.r, p.g, p.b)).collect(),
        load_image::ImageData::RGB16(v) => v
            .iter()
            .map(|p| RGB8::new((p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8))
            .collect(),
        load_image::ImageData::RGBA16(v) => v
            .iter()
            .map(|p| RGB8::new((p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8))
            .collect(),
        _ => panic!("{}: grayscale sources are not part of this grid", path.display()),
    };
    Img::new(px, w, h)
}

/// Mitchell-Netravali downscale to a target long edge. Never upscales (callers
/// filter those cells out first).
fn downscale(src: &ImgVec<RGB8>, target_long_edge: u32) -> ImgVec<RGB8> {
    let (sw, sh) = (src.width() as u32, src.height() as u32);
    let long = sw.max(sh);
    if long == target_long_edge {
        return src.clone();
    }
    let scale = f64::from(target_long_edge) / f64::from(long);
    let dw = ((f64::from(sw) * scale).round() as u32).max(1);
    let dh = ((f64::from(sh) * scale).round() as u32).max(1);

    // zenresize works on packed buffers; RGB8 is 3 bytes/px.
    let flat: Vec<u8> = src.pixels().flat_map(|p| [p.r, p.g, p.b]).collect();
    let cfg = ResizeConfig::builder(sw, sh, dw, dh)
        .filter(Filter::Mitchell)
        .format(PixelDescriptor::RGB8_SRGB)
        .build();
    let out = Resizer::new(&cfg).resize(&flat);
    let px: Vec<RGB8> = out.chunks_exact(3).map(|c| RGB8::new(c[0], c[1], c[2])).collect();
    Img::new(px, dw as usize, dh as usize)
}

/// Full-range BT.601 inverse — the exact inverse of ravif's `rgb_to_ycbcr`
/// (`BT601 = [0.2990, 0.5870, 0.1140]`, `PixelRange::Full`, 4:4:4 so there is
/// no chroma resampling step to disagree about).
#[inline]
fn ycbcr_to_rgb8(y: f32, cb: f32, cr: f32, max: f32) -> RGB8 {
    let s = 255.0 / max;
    let half = (max * 0.5).round();
    let yv = y * s;
    let r = (cr - half) * (2.0 * (1.0 - 0.2990)) * s + yv;
    let b = (cb - half) * (2.0 * (1.0 - 0.1140)) * s + yv;
    let g = (yv - 0.2990 * r - 0.1140 * b) / 0.5870;
    RGB8::new(
        r.round().clamp(0.0, 255.0) as u8,
        g.round().clamp(0.0, 255.0) as u8,
        b.round().clamp(0.0, 255.0) as u8,
    )
}

fn decode_avif_rgb8(avif: &[u8]) -> Option<ImgVec<RGB8>> {
    let parsed = avif_parse::read_avif(&mut &avif[..]).ok()?;
    // Single-threaded on purpose: the tile-threading DisjointMut wedge on
    // rav1d-safe 0.5.x (zenavif#30) only bites with worker threads, and a
    // hung measurement is worse than a slow one.
    let mut settings = Settings::default();
    settings.threads = 1;
    settings.max_frame_delay = 1;
    let mut dec = Decoder::with_settings(settings).ok()?;
    let frame = dec.decode(&parsed.primary_item[..]).ok()??;
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let max = ((1u32 << frame.bit_depth()) - 1) as f32;
    let mut px = Vec::with_capacity(w * h);
    match frame.planes() {
        Planes::Depth8(p) => {
            let (yv, uv, vv) = (p.y(), p.u()?, p.v()?);
            for row in 0..h {
                let (yr, ur, vr) = (yv.row(row), uv.row(row), vv.row(row));
                for x in 0..w {
                    px.push(ycbcr_to_rgb8(f32::from(yr[x]), f32::from(ur[x]), f32::from(vr[x]), max));
                }
            }
        }
        Planes::Depth16(p) => {
            let (yv, uv, vv) = (p.y(), p.u()?, p.v()?);
            for row in 0..h {
                let (yr, ur, vr) = (yv.row(row), uv.row(row), vv.row(row));
                for x in 0..w {
                    px.push(ycbcr_to_rgb8(f32::from(yr[x]), f32::from(ur[x]), f32::from(vr[x]), max));
                }
            }
        }
    }
    Some(Img::new(px, w, h))
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Tiny in-tree SHA-256 so the harness needs no extra dependency; the hash
    // is only a cell-identity fingerprint for the pre/post join.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = bytes.to_vec();
    let bitlen = (bytes.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    let mut w = [0u32; 64];
    for chunk in msg.chunks_exact(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1);
            d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].into_iter().enumerate() {
            h[i] = h[i].wrapping_add(v);
        }
    }
    let mut s = String::new();
    for v in h {
        let _ = write!(s, "{v:08x}");
    }
    s
}

fn main() {
    let args = parse_args();
    let mut out = String::new();
    out.push_str("label\tsource\tclass\tsize\twidth\theight\tspeed\tquality\tbytes\tbpp\tssim2\tsha256\tenc_ms\n");

    let qualities: Vec<u32> = (args.qmin..=args.qmax).step_by(args.qstep as usize).collect();
    let total_start = Instant::now();
    let mut cells = 0usize;

    for (class, rel) in SOURCES {
        if let Some(only) = &args.only
            && !class.contains(only.as_str())
        {
            continue;
        }
        let path = args.corpus.join(rel);
        let src = load_source(&path);
        let native_long = src.width().max(src.height()) as u32;
        for &size in &args.sizes {
            if size > native_long {
                eprintln!("skip {class}@{size}: source long edge {native_long} < target (no upscaling)");
                continue;
            }
            let scaled = downscale(&src, size);
            let (w, h) = (scaled.width(), scaled.height());
            for &speed in &args.speeds {
                for &q in &qualities {
                    let enc = Encoder::new()
                        .with_quality(q as f32)
                        .with_speed(speed)
                        .with_num_threads(Some(args.threads));
                    let t0 = Instant::now();
                    let res = match enc.encode_rgb(scaled.as_ref()) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("ENCFAIL {class} {size} s{speed} q{q}: {e}");
                            continue;
                        }
                    };
                    let enc_ms = t0.elapsed().as_secs_f64() * 1000.0;
                    let bytes = res.avif_file.len();
                    let sha = sha256_hex(&res.avif_file);
                    let ssim2 = match decode_avif_rgb8(&res.avif_file) {
                        Some(dec) => {
                            let a: Vec<[u8; 3]> = scaled.pixels().map(|p| [p.r, p.g, p.b]).collect();
                            let b: Vec<[u8; 3]> = dec.pixels().map(|p| [p.r, p.g, p.b]).collect();
                            let ai = Img::new(a, w, h);
                            let bi = Img::new(b, dec.width(), dec.height());
                            fast_ssim2::compute_ssimulacra2(ai.as_ref(), bi.as_ref()).unwrap_or(f64::NAN)
                        }
                        None => {
                            eprintln!("DECFAIL {class} {size} s{speed} q{q}");
                            f64::NAN
                        }
                    };
                    let bpp = (bytes as f64 * 8.0) / (w as f64 * h as f64);
                    let _ = writeln!(
                        out,
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.6}\t{:.4}\t{}\t{:.1}",
                        args.label, rel, class, size, w, h, speed, q, bytes, bpp, ssim2, sha, enc_ms
                    );
                    cells += 1;
                    if cells.is_multiple_of(20) {
                        std::fs::write(&args.out, &out).expect("write tsv");
                        eprintln!(
                            "[{:6.1}s] {cells} cells :: {class} {size} s{speed} q{q} -> {bytes} B ssim2 {ssim2:.2}",
                            total_start.elapsed().as_secs_f64()
                        );
                    }
                }
            }
        }
    }
    std::fs::write(&args.out, &out).expect("write tsv");
    eprintln!(
        "done: {cells} cells in {:.1}s -> {}",
        total_start.elapsed().as_secs_f64(),
        args.out.display()
    );
}
