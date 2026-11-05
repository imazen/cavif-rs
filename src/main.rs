use clap::{value_parser, Arg, ArgAction, Command};
use imgref::{ImgRef, ImgVec};
use rgb::RGBA;
use zenravif::{AlphaColorMode, BitDepth, ChromaSubsampling, ColorModel, EncodedImage, Encoder, MatrixCoefficients, PixelRange, RGBA8};
use rayon::prelude::*;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        let mut source = e.source();
        while let Some(e) = source {
            eprintln!("  because: {e}");
            source = e.source();
        }
        std::process::exit(1);
    }
}

enum MaybePath {
    Stdio,
    Path(PathBuf),
}

impl MaybePath {
    fn display(&self) -> std::path::Display<'_> {
        match self {
            Self::Stdio => Path::new("stdin").display(),
            Self::Path(path) => path.display(),
        }
    }
}

/// Decoded input. Samples deeper than 8 bits stay deep until the output depth
/// is known, so a narrowing is always the output's and never the loader's.
enum Source {
    Rgba8(ImgVec<RGBA8>),
    Rgba16(ImgVec<RGBA<u16>>),
}

fn parse_quality(arg: &str) -> Result<f32, String> {
    let q = arg.parse::<f32>().map_err(|e| e.to_string())?;
    if !(1. ..=100.).contains(&q) {
        return Err("quality must be in 1-100 range".into());
    }
    Ok(q)
}

fn parse_speed(arg: &str) -> Result<u8, String> {
    let s = arg.parse::<u8>().map_err(|e| e.to_string())?;
    if !(1..=100).contains(&s) {
        return Err("speed must be in 1-10 range".into());
    }
    Ok(s)
}

fn run() -> Result<(), BoxError> {
    let args = Command::new("cavif-rs")
        .version(clap::crate_version!())
        .author("Kornel Lesiński <kornel@imageoptim.com>")
        .about("Convert JPEG/PNG images to AVIF image format (based on AV1/rav1e)")
        .arg(Arg::new("quality")
            .short('Q')
            .long("quality")
            .value_name("n")
            .value_parser(parse_quality)
            .default_value("80")
            .help("Quality from 1 (worst) to 100 (best)"))
        .arg(Arg::new("speed")
            .short('s')
            .long("speed")
            .value_name("n")
            .default_value("4")
            .value_parser(parse_speed)
            .help("Encoding speed from 1 (best) to 10 (fast but ugly)"))
        .arg(Arg::new("threads")
            .short('j')
            .long("threads")
            .value_name("n")
            .default_value("0")
            .value_parser(value_parser!(u8))
            .help("Maximum threads to use (0 = one thread per host core)"))
        .arg(Arg::new("overwrite")
            .alias("force")
            .short('f')
            .long("overwrite")
            .action(ArgAction::SetTrue)
            .num_args(0)
            .help("Replace files if there's .avif already"))
        .arg(Arg::new("output")
            .short('o')
            .long("output")
            .value_parser(value_parser!(PathBuf))
            .value_name("path")
            .help("Write output to this path instead of same_file.avif. It may be a file or a directory."))
        .arg(Arg::new("quiet")
            .short('q')
            .long("quiet")
            .action(ArgAction::SetTrue)
            .num_args(0)
            .help("Don't print anything"))
        .arg(Arg::new("dirty-alpha")
            .long("dirty-alpha")
            .action(ArgAction::SetTrue)
            .num_args(0)
            .help("Keep RGB data of fully-transparent pixels (makes larger, lower quality files)"))
        .arg(Arg::new("color")
            .long("color")
            .default_value("ycbcr")
            .value_parser(["ycbcr", "rgb"])
            .help("Internal AVIF color model. YCbCr works better for human eyes."))
        .arg(Arg::new("depth")
            .long("depth")
            .default_value("auto")
            .value_parser(["8", "10", "auto"])
            .help("Write 8-bit (more compatible) or 10-bit (better quality) images. Sources deeper than 8 bits keep 10 bits unless this says 8"))
        .arg(Arg::new("yuv")
            .long("yuv")
            .default_value("444")
            .value_parser(["444", "420"])
            .help("Chroma subsampling: 444 (full resolution) or 420 (half, smaller files)"))
        .arg(Arg::new("libavif-scale")
            .long("libavif-scale")
            .action(ArgAction::SetTrue)
            .num_args(0)
            .help("Interpret --quality using libavif/avifenc's linear scale instead of zenravif's curve"))
        .arg(Arg::new("IMAGES")
            .index(1)
            .num_args(1..)
            .value_parser(value_parser!(PathBuf))
            .help("One or more JPEG or PNG files to convert. \"-\" is interpreted as stdin/stdout."))
        .get_matches();

    let output = args.get_one::<PathBuf>("output").map(|s| match s {
        s if s.as_os_str() == "-" => MaybePath::Stdio,
        s => MaybePath::Path(PathBuf::from(s)),
    });
    let quality = *args.get_one::<f32>("quality").expect("default");
    let alpha_quality = ((quality + 100.) / 2.).min(quality + quality / 4. + 2.);
    let speed: u8 = *args.get_one::<u8>("speed").expect("default");
    let overwrite = args.get_flag("overwrite");
    let quiet = args.get_flag("quiet");
    let threads = args.get_one::<u8>("threads").copied();
    let dirty_alpha = args.get_flag("dirty-alpha");
    let libavif_scale = args.get_flag("libavif-scale");

    let color_model = match args.get_one::<String>("color").expect("default").as_str() {
        "ycbcr" => ColorModel::YCbCr,
        "rgb" => ColorModel::RGB,
        x => Err(format!("bad color type: {x}"))?,
    };

    let depth = match args.get_one::<String>("depth").expect("default").as_str() {
        "8" => BitDepth::Eight,
        "10" => BitDepth::Ten,
        _ => BitDepth::Auto,
    };

    let chroma_subsampling = match args.get_one::<String>("yuv").expect("default").as_str() {
        "420" => ChromaSubsampling::Yuv420,
        _ => ChromaSubsampling::Yuv444,
    };

    let files = args.get_many::<PathBuf>("IMAGES").ok_or("Please specify image paths to convert")?;
    let files: Vec<_> = files
        .filter(|pathstr| {
            let path = Path::new(&pathstr);
            if let Some(s) = path.to_str().filter(|_| quiet)
                && s.parse::<u8>().is_ok() && !path.exists()
            {
                eprintln!("warning: -q is not for quality, so '{s}' is misinterpreted as a file. Use -Q {s}");
            }
            path.extension().is_none_or(|e| if e == "avif" {
                if !quiet {
                    if path.exists() {
                        eprintln!("warning: ignoring {}, because it's already an AVIF", path.display());
                    } else {
                        eprintln!("warning: Did you mean to use -o {p}?", p = path.display());
                        return true;
                    }
                }
                false
            } else {
                true
            })
        })
        .map(|p| if p.as_os_str() == "-" {
            MaybePath::Stdio
        } else {
            MaybePath::Path(PathBuf::from(p))
        })
        .collect();

    if files.is_empty() {
        return Err("No PNG/JPEG files specified".into());
    }

    let use_dir = match output {
        Some(MaybePath::Path(ref path)) => {
            if files.len() > 1 {
                let _ = fs::create_dir_all(path);
            }
            files.len() > 1 || path.is_dir()
        },
        _ => false,
    };

    let process = move |data: Vec<u8>, input_path: &MaybePath| -> Result<(), BoxError> {
        let img = load_source(&data)?;
        drop(data);
        let out_path = match (&output, input_path) {
            (None, MaybePath::Path(input)) => MaybePath::Path(input.with_extension("avif")),
            (Some(MaybePath::Path(output)), MaybePath::Path(input)) => MaybePath::Path({
                if use_dir {
                    output.join(Path::new(input.file_name().unwrap()).with_extension("avif"))
                } else {
                    output.clone()
                }
            }),
            (None, MaybePath::Stdio) |
            (Some(MaybePath::Stdio), _) => MaybePath::Stdio,
            (Some(MaybePath::Path(output)), MaybePath::Stdio) => MaybePath::Path(output.clone()),
        };
        match out_path {
            MaybePath::Path(ref p) if !overwrite && p.exists() => {
                return Err(format!("{} already exists; skipping", p.display()).into());
            },
            _ => {},
        }
        let enc = Encoder::new()
            .with_bit_depth(depth)
            .with_speed(speed)
            .with_alpha_quality(alpha_quality)
            .with_internal_color_model(color_model)
            .with_alpha_color_mode(if dirty_alpha { AlphaColorMode::UnassociatedDirty } else { AlphaColorMode::UnassociatedClean })
            .with_num_threads(threads.filter(|&n| n > 0).map(usize::from))
            .with_chroma_subsampling(chroma_subsampling);
        let enc = if libavif_scale {
            enc.with_libavif_quality(quality)
        } else {
            enc.with_quality(quality)
        };
        // zenravif 0.2 returns `At<Error>` (a traced error). Surface the
        // `file:line` trace in the CLI's error output so server-side failures
        // are debuggable, then box it for the rest of the pipeline.
        let encoded = match img {
            Source::Rgba8(img) => enc.encode_rgba(img.as_ref()),
            Source::Rgba16(img) => {
                let output_bits = if depth == BitDepth::Eight { 8 } else { 10 };
                if !quiet {
                    eprintln!("{}: 16-bit source narrowed to {output_bits}-bit output", input_path.display());
                }
                if output_bits == 8 {
                    enc.encode_rgba(narrow_to_8_bit(img.as_ref()).as_ref())
                } else {
                    encode_10_bit(&enc, img, color_model, dirty_alpha)
                }
            },
        };
        let EncodedImage { avif_file, color_byte_size, alpha_byte_size , .. } = encoded
            .map_err(|e| -> BoxError { format!("{}", e.full_trace()).into() })?;
        match out_path {
            MaybePath::Path(ref p) => {
                if !quiet {
                    println!("{}: {}KB ({color_byte_size}B color, {alpha_byte_size}B alpha, {}B HEIF)", p.display(), avif_file.len().div_ceil(1000), avif_file.len() - color_byte_size - alpha_byte_size);
                }
                fs::write(p, avif_file)
            },
            MaybePath::Stdio => std::io::stdout().write_all(&avif_file),
        }
        .map_err(|e| format!("Unable to write output image: {e}"))?;
        Ok(())
    };

    let failures = files.into_par_iter().map(|path| {
            let tmp;
            let (data, path_str): (_, &dyn std::fmt::Display) = match path {
                MaybePath::Stdio => {
                    let mut data = Vec::new();
                    std::io::stdin().read_to_end(&mut data)?;
                    (data, &"stdin")
                },
                MaybePath::Path(ref path) => {
                    let data = fs::read(path).map_err(|e| format!("Unable to read input image {}: {e}", path.display()))?;
                    tmp = path.display();
                    (data, &tmp)
                },
            };
            process(data, &path)
                .map_err(|e| BoxError::from(format!("{path_str}: error: {e}")))
        })
        .filter_map(|res| res.err())
        .collect::<Vec<BoxError>>();

    if !failures.is_empty() {
        if !quiet {
            for f in failures {
                eprintln!("error: {f}");
            }
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(feature = "cocoa_image"))]
fn load_source(data: &[u8]) -> Result<Source, BoxError> {
    use load_image::export::imgref::ImgVecKind;

    let deep: ImgVec<RGBA<u16>> = match load_image::load_data(data)?.into_imgvec() {
        ImgVecKind::RGB8(img) => return Ok(Source::Rgba8(img.map_buf(|buf| buf.into_iter().map(|px| px.with_alpha(255)).collect()))),
        ImgVecKind::RGBA8(img) => return Ok(Source::Rgba8(img)),
        ImgVecKind::GRAY8(img) => return Ok(Source::Rgba8(img.map_buf(|buf| buf.into_iter().map(|g| { let c = g.value(); RGBA8::new(c,c,c,255) }).collect()))),
        ImgVecKind::GRAYA8(img) => return Ok(Source::Rgba8(img.map_buf(|buf| buf.into_iter().map(|g| { let c = g.value(); RGBA8::new(c,c,c,g.a) }).collect()))),
        ImgVecKind::RGB16(img) => img.map_buf(|buf| buf.into_iter().map(|px| px.with_alpha(u16::MAX)).collect()),
        ImgVecKind::RGBA16(img) => img,
        ImgVecKind::GRAY16(img) => img.map_buf(|buf| buf.into_iter().map(|g| { let c = g.value(); RGBA::new(c,c,c,u16::MAX) }).collect()),
        ImgVecKind::GRAYA16(img) => img.map_buf(|buf| buf.into_iter().map(|g| { let c = g.value(); RGBA::new(c,c,c,g.a) }).collect()),
    };
    // Eight-bit samples in 16-bit storage lose nothing here, and then encode
    // exactly as the same image stored in 8 bits does.
    if deep.pixels().all(|px| [px.r, px.g, px.b, px.a].iter().all(|c| c % 257 == 0)) {
        return Ok(Source::Rgba8(narrow_to_8_bit(deep.as_ref())));
    }
    Ok(Source::Rgba16(deep))
}

#[cfg(feature = "cocoa_image")]
fn load_source(data: &[u8]) -> Result<Source, BoxError> {
    Ok(Source::Rgba8(cocoa_image::decode_image_as_rgba(data)?))
}

fn narrow_to_8_bit(img: ImgRef<'_, RGBA<u16>>) -> ImgVec<RGBA8> {
    let narrow = |c: u16| ((u32::from(c) * 255 + 32767) / 65535) as u8;
    let pixels = img.pixels().map(|px| RGBA8::new(narrow(px.r), narrow(px.g), narrow(px.b), narrow(px.a))).collect();
    ImgVec::new(pixels, img.width(), img.height())
}

/// Encodes 16-bit samples through 10-bit planes without an 8-bit stage.
///
/// Uses the matrix and the full range the 8-bit entry points use.
fn encode_10_bit(enc: &Encoder, mut img: ImgVec<RGBA<u16>>, color_model: ColorModel, dirty_alpha: bool) -> zenravif::Result<EncodedImage> {
    let (width, height) = (img.width(), img.height());
    if !dirty_alpha {
        clear_transparent_colors(&mut img);
    }
    let to_ten = |c: u16| ((u32::from(c) * 1023 + 32767) / 65535) as u16;
    let planes = img.pixels().map(|px| match color_model {
        ColorModel::YCbCr => rgb16_to_10_bit_ycbcr(px),
        ColorModel::RGB => [to_ten(px.g), to_ten(px.b), to_ten(px.r)],
    });
    let matrix = match color_model {
        ColorModel::YCbCr => MatrixCoefficients::BT601,
        ColorModel::RGB => MatrixCoefficients::Identity,
    };
    if img.pixels().all(|px| px.a == u16::MAX) {
        return enc.encode_raw_planes_10_bit(width, height, planes, None::<[u16; 0]>, PixelRange::Full, matrix);
    }
    let alpha = img.pixels().map(|px| to_ten(px.a));
    enc.encode_raw_planes_10_bit(width, height, planes, Some(alpha), PixelRange::Full, matrix)
}

fn rgb16_to_10_bit_ycbcr(px: RGBA<u16>) -> [u16; 3] {
    const KR: f64 = 0.2990;
    const KG: f64 = 0.5870;
    const KB: f64 = 0.1140;
    const MAX: f64 = 1023.;
    const NEUTRAL: f64 = 512.;
    let scale = MAX / 65535.;
    let (r, g, b) = (f64::from(px.r) * scale, f64::from(px.g) * scale, f64::from(px.b) * scale);
    let y = KR * r + KG * g + KB * b;
    let cb = (b - y) * 0.5 / (1. - KB) + NEUTRAL;
    let cr = (r - y) * 0.5 / (1. - KR) + NEUTRAL;
    [y, cb, cr].map(|v| v.round().clamp(0., MAX) as u16)
}

/// Gives fully transparent pixels the average color of their visible neighbors,
/// or of the visible image, because their own color is never seen.
///
/// Simplified from the 8-bit `blurred_dirty_alpha`, which also blurs the result:
/// the output depends only on visible samples, but this can cost more bytes.
fn clear_transparent_colors(img: &mut ImgVec<RGBA<u16>>) {
    let (width, height) = (img.width(), img.height());
    let mut image_sum = [0u64; 3];
    let mut image_count = 0u64;
    for px in img.pixels().filter(|px| px.a != 0) {
        image_sum = [image_sum[0] + u64::from(px.r), image_sum[1] + u64::from(px.g), image_sum[2] + u64::from(px.b)];
        image_count += 1;
    }
    let original = img.clone();
    let average = |sum: [u64; 3], count: u64| sum.map(|c| (c / count.max(1)) as u16);
    let background = average(image_sum, image_count);
    for y in 0..height {
        for x in 0..width {
            if original[(x, y)].a != 0 {
                continue;
            }
            let mut sum = [0u64; 3];
            let mut count = 0u64;
            for ny in y.saturating_sub(1)..(y + 2).min(height) {
                for nx in x.saturating_sub(1)..(x + 2).min(width) {
                    let n = original[(nx, ny)];
                    if n.a != 0 {
                        sum = [sum[0] + u64::from(n.r), sum[1] + u64::from(n.g), sum[2] + u64::from(n.b)];
                        count += 1;
                    }
                }
            }
            let [r, g, b] = if count == 0 { background } else { average(sum, count) };
            img[(x, y)] = RGBA::new(r, g, b, 0);
        }
    }
}
