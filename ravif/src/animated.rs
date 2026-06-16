//! Animated AVIF encoding
//!
//! Encodes a sequence of frames into an animated AVIF file using
//! rav1e's video encoding mode and a minimal ISOBMFF muxer.

use crate::av1encoder::SpeedTweaks;
use crate::error::Error;
use zenrav1e::prelude::*;
use zenavif_serialize::animated::{AnimFrame as SerializeFrame, AnimatedImage};
use zenavif_serialize::Av1CBox;
use rgb::{RGB8, RGBA8};
use imgref::ImgRef;

/// A single frame in an animated sequence (8-bit RGB)
#[derive(Clone)]
pub struct AnimFrame<'a> {
    /// Frame pixel data (RGB8)
    pub rgb: ImgRef<'a, RGB8>,
    /// Duration of this frame in milliseconds
    pub duration_ms: u32,
}

/// A single frame with alpha in an animated sequence (8-bit RGBA)
#[derive(Clone)]
pub struct AnimFrameRgba<'a> {
    /// Frame pixel data (RGBA8)
    pub rgba: ImgRef<'a, RGBA8>,
    /// Duration of this frame in milliseconds
    pub duration_ms: u32,
}

/// A single frame in an animated sequence (16-bit RGB, 10-bit values 0–1023)
#[derive(Clone)]
pub struct AnimFrame16<'a> {
    /// Frame pixel data (RGB16, 10-bit values)
    pub rgb: ImgRef<'a, rgb::RGB<u16>>,
    /// Duration of this frame in milliseconds
    pub duration_ms: u32,
}

/// A single frame with alpha in an animated sequence (16-bit RGBA, 10-bit values 0–1023)
#[derive(Clone)]
pub struct AnimFrameRgba16<'a> {
    /// Frame pixel data (RGBA16, 10-bit values)
    pub rgba: ImgRef<'a, rgb::RGBA<u16>>,
    /// Duration of this frame in milliseconds
    pub duration_ms: u32,
}

/// Result of animated AVIF encoding
#[non_exhaustive]
#[derive(Clone)]
pub struct EncodedAnimation {
    /// Complete AVIF file bytes
    pub avif_file: Vec<u8>,
    /// Number of frames encoded
    pub frame_count: usize,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
}

const BT601: [f32; 3] = [0.2990, 0.5870, 0.1140];

impl crate::Encoder<'_> {
    /// Encode a sequence of 8-bit RGB frames into an animated AVIF.
    ///
    /// Each frame has its own duration in milliseconds. All frames must have
    /// the same dimensions.
    pub fn encode_animation_rgb(&self, frames: &[AnimFrame<'_>]) -> Result<EncodedAnimation, Error> {
        if frames.is_empty() {
            return Err(Error::Unsupported("empty frame sequence"));
        }

        let width = frames[0].rgb.width();
        let height = frames[0].rgb.height();

        for f in frames {
            if f.rgb.width() != width || f.rgb.height() != height {
                return Err(Error::Unsupported("all frames must have the same dimensions"));
            }
            if f.duration_ms == 0 {
                return Err(Error::Unsupported("frame duration must be > 0"));
            }
        }

        let durations_ms: Vec<u32> = frames.iter().map(|f| f.duration_ms).collect();

        let encoded_frames = encode_sequence_av1::<u8>(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgb8_420(rav1e_frame, width, height, f.rgb)?;
                Ok(())
            },
            false,
            8,
        )?;

        assemble_animation(self, width, height, &encoded_frames, &durations_ms, None, 8)
    }

    /// Encode a sequence of 8-bit RGBA frames into an animated AVIF.
    ///
    /// If any frame has non-opaque alpha, an alpha track is included.
    pub fn encode_animation_rgba(&self, frames: &[AnimFrameRgba<'_>]) -> Result<EncodedAnimation, Error> {
        if frames.is_empty() {
            return Err(Error::Unsupported("empty frame sequence"));
        }

        let width = frames[0].rgba.width();
        let height = frames[0].rgba.height();

        for f in frames {
            if f.rgba.width() != width || f.rgba.height() != height {
                return Err(Error::Unsupported("all frames must have the same dimensions"));
            }
            if f.duration_ms == 0 {
                return Err(Error::Unsupported("frame duration must be > 0"));
            }
        }

        let has_alpha = frames.iter().any(|f| f.rgba.pixels().any(|px| px.a != 255));
        let durations_ms: Vec<u32> = frames.iter().map(|f| f.duration_ms).collect();

        // Encode color track
        let color_frames = encode_sequence_av1::<u8>(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgba8_color_420(rav1e_frame, width, height, f.rgba)?;
                Ok(())
            },
            false,
            8,
        )?;

        // Encode alpha track if needed
        let alpha_frames = if has_alpha {
            Some(encode_sequence_av1::<u8>(
                self, width, height,
                frames.len(),
                |frame_idx, rav1e_frame| {
                    let f = &frames[frame_idx];
                    fill_frame_alpha8(rav1e_frame, width, height, f.rgba)?;
                    Ok(())
                },
                true,
                8,
            )?)
        } else {
            None
        };

        assemble_animation(self, width, height, &color_frames, &durations_ms, alpha_frames.as_deref(), 8)
    }

    /// Encode a sequence of 16-bit RGB frames into an animated AVIF (10-bit AV1).
    ///
    /// Input values should be in 10-bit range (0–1023). All frames must have
    /// the same dimensions.
    pub fn encode_animation_rgb16(&self, frames: &[AnimFrame16<'_>]) -> Result<EncodedAnimation, Error> {
        if frames.is_empty() {
            return Err(Error::Unsupported("empty frame sequence"));
        }

        let width = frames[0].rgb.width();
        let height = frames[0].rgb.height();

        for f in frames {
            if f.rgb.width() != width || f.rgb.height() != height {
                return Err(Error::Unsupported("all frames must have the same dimensions"));
            }
            if f.duration_ms == 0 {
                return Err(Error::Unsupported("frame duration must be > 0"));
            }
        }

        let durations_ms: Vec<u32> = frames.iter().map(|f| f.duration_ms).collect();

        let encoded_frames = encode_sequence_av1::<u16>(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgb16_420(rav1e_frame, width, height, f.rgb)?;
                Ok(())
            },
            false,
            10,
        )?;

        assemble_animation(self, width, height, &encoded_frames, &durations_ms, None, 10)
    }

    /// Encode a sequence of 16-bit RGBA frames into an animated AVIF (10-bit AV1).
    ///
    /// Input values should be in 10-bit range (0–1023). If any frame has
    /// non-opaque alpha, an alpha track is included.
    pub fn encode_animation_rgba16(&self, frames: &[AnimFrameRgba16<'_>]) -> Result<EncodedAnimation, Error> {
        if frames.is_empty() {
            return Err(Error::Unsupported("empty frame sequence"));
        }

        let width = frames[0].rgba.width();
        let height = frames[0].rgba.height();

        for f in frames {
            if f.rgba.width() != width || f.rgba.height() != height {
                return Err(Error::Unsupported("all frames must have the same dimensions"));
            }
            if f.duration_ms == 0 {
                return Err(Error::Unsupported("frame duration must be > 0"));
            }
        }

        let has_alpha = frames.iter().any(|f| f.rgba.pixels().any(|px| px.a != 1023));
        let durations_ms: Vec<u32> = frames.iter().map(|f| f.duration_ms).collect();

        // Encode color track
        let color_frames = encode_sequence_av1::<u16>(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgba16_color_420(rav1e_frame, width, height, f.rgba)?;
                Ok(())
            },
            false,
            10,
        )?;

        // Encode alpha track if needed
        let alpha_frames = if has_alpha {
            Some(encode_sequence_av1::<u16>(
                self, width, height,
                frames.len(),
                |frame_idx, rav1e_frame| {
                    let f = &frames[frame_idx];
                    fill_frame_alpha16(rav1e_frame, width, height, f.rgba)?;
                    Ok(())
                },
                true,
                10,
            )?)
        } else {
            None
        };

        assemble_animation(self, width, height, &color_frames, &durations_ms, alpha_frames.as_deref(), 10)
    }
}

// ---- Encoding helpers ----

fn encode_sequence_av1<P: Pixel + Default>(
    enc: &crate::Encoder<'_>,
    width: usize,
    height: usize,
    num_frames: usize,
    init_frame: impl Fn(usize, &mut Frame<P>) -> Result<(), Error>,
    is_alpha: bool,
    bit_depth: u8,
) -> Result<Vec<Vec<u8>>, Error> {
    // Pre-flight: reject oversized frames before building the rav1e context.
    // This is the shared chokepoint for every animation encode entry point.
    enc.check_pixel_limit(width, height)?;

    let (quantizer, chroma_sampling) = if is_alpha {
        (enc.alpha_quantizer, ChromaSampling::Cs400)
    } else {
        (enc.quantizer, ChromaSampling::Cs420)
    };

    let speed = SpeedTweaks::from_my_preset(enc.speed, quantizer);

    let color_description = if is_alpha {
        None
    } else {
        Some(ColorDescription {
            transfer_characteristics: enc.transfer_characteristics
                .unwrap_or(TransferCharacteristics::SRGB),
            color_primaries: enc.color_primaries
                .unwrap_or(ColorPrimaries::BT709),
            matrix_coefficients: MatrixCoefficients::BT601,
        })
    };

    let config = EncoderConfig {
        width,
        height,
        time_base: Rational::new(1, 1000),
        sample_aspect_ratio: Rational::new(1, 1),
        bit_depth: bit_depth as usize,
        chroma_sampling,
        chroma_sample_position: ChromaSamplePosition::Unknown,
        pixel_range: PixelRange::Full,
        color_description,
        mastering_display: if is_alpha { None } else { enc.mastering_display },
        content_light: if is_alpha { None } else { enc.content_light },
        enable_timing_info: false,
        still_picture: false,
        error_resilient: false,
        switch_frame_interval: 0,
        min_key_frame_interval: 0,
        max_key_frame_interval: num_frames as u64,
        reservoir_frame_delay: None,
        low_latency: true,
        quantizer: quantizer as usize,
        min_quantizer: quantizer as _,
        bitrate: 0,
        tune: Tune::Psychovisual,
        tile_cols: 0,
        tile_rows: 0,
        tiles: 0,
        film_grain_params: None,
        level_idx: None,
        enable_qm: {
            #[cfg(feature = "imazen")]
            { if is_alpha { false } else { enc.enable_qm } }
            #[cfg(not(feature = "imazen"))]
            { false }
        },
        enable_vaq: false,
        vaq_strength: 1.0,
        seg_boost: 1.0,
        enable_trellis: false,
        max_pixel_count: enc.max_pixels,
        speed_settings: speed.speed_settings(),
    };

    let cfg = Config::new().with_encoder_config(config);
    let mut ctx: Context<P> = cfg.new_context()?;

    for i in 0..num_frames {
        let mut frame = ctx.new_frame();
        init_frame(i, &mut frame)?;
        ctx.send_frame(frame)?;
    }
    ctx.flush();

    let mut packets: Vec<Option<Vec<u8>>> = (0..num_frames).map(|_| None).collect();

    loop {
        match ctx.receive_packet() {
            Ok(packet) => {
                let idx = packet.input_frameno as usize;
                if idx < num_frames {
                    packets[idx] = Some(packet.data);
                }
            }
            Err(EncoderStatus::Encoded) => continue,
            Err(EncoderStatus::NeedMoreData) => continue,
            Err(EncoderStatus::LimitReached) => break,
            Err(err) => return Err(err.into()),
        }
    }

    let mut result = Vec::with_capacity(num_frames);
    for p in packets {
        result.push(p.ok_or(Error::Unsupported("frame was not encoded"))?);
    }
    Ok(result)
}

fn make_sequence_header<P: Pixel + Default>(
    enc: &crate::Encoder<'_>,
    width: usize,
    height: usize,
    is_alpha: bool,
    bit_depth: u8,
) -> Result<Vec<u8>, Error> {
    // Pre-flight: guard the standalone context built here too.
    enc.check_pixel_limit(width, height)?;

    let (quantizer, chroma_sampling) = if is_alpha {
        (enc.alpha_quantizer, ChromaSampling::Cs400)
    } else {
        (enc.quantizer, ChromaSampling::Cs420)
    };

    let speed = SpeedTweaks::from_my_preset(enc.speed, quantizer);

    let config = EncoderConfig {
        width,
        height,
        time_base: Rational::new(1, 1000),
        sample_aspect_ratio: Rational::new(1, 1),
        bit_depth: bit_depth as usize,
        chroma_sampling,
        chroma_sample_position: ChromaSamplePosition::Unknown,
        pixel_range: PixelRange::Full,
        color_description: if is_alpha {
            None
        } else {
            Some(ColorDescription {
                transfer_characteristics: enc.transfer_characteristics
                    .unwrap_or(TransferCharacteristics::SRGB),
                color_primaries: enc.color_primaries
                    .unwrap_or(ColorPrimaries::BT709),
                matrix_coefficients: MatrixCoefficients::BT601,
            })
        },
        mastering_display: None,
        content_light: None,
        enable_timing_info: false,
        still_picture: false,
        error_resilient: false,
        switch_frame_interval: 0,
        min_key_frame_interval: 0,
        max_key_frame_interval: 1,
        reservoir_frame_delay: None,
        low_latency: true,
        quantizer: quantizer as usize,
        min_quantizer: quantizer as _,
        bitrate: 0,
        tune: Tune::Psychovisual,
        tile_cols: 0,
        tile_rows: 0,
        tiles: 0,
        film_grain_params: None,
        level_idx: None,
        enable_qm: false,
        enable_vaq: false,
        vaq_strength: 1.0,
        seg_boost: 1.0,
        enable_trellis: false,
        max_pixel_count: enc.max_pixels,
        speed_settings: speed.speed_settings(),
    };
    let cfg = Config::new().with_encoder_config(config);
    let ctx: Context<P> = cfg.new_context()?;
    Ok(ctx.container_sequence_header())
}

/// Assemble encoded frames into an animated AVIF container.
fn assemble_animation(
    enc: &crate::Encoder<'_>,
    width: usize,
    height: usize,
    color_frames: &[Vec<u8>],
    durations_ms: &[u32],
    alpha_frames: Option<&[Vec<u8>]>,
    bit_depth: u8,
) -> Result<EncodedAnimation, Error> {
    let total_duration_ms: u64 = durations_ms.iter().map(|d| u64::from(*d)).sum();
    let frame_count = color_frames.len();

    let (color_seq_header, alpha_seq_header) = match bit_depth {
        10 | 12 => {
            let color = make_sequence_header::<u16>(enc, width, height, false, bit_depth)?;
            let alpha = if alpha_frames.is_some() {
                Some(make_sequence_header::<u16>(enc, width, height, true, bit_depth)?)
            } else {
                None
            };
            (color, alpha)
        }
        _ => {
            let color = make_sequence_header::<u8>(enc, width, height, false, bit_depth)?;
            let alpha = if alpha_frames.is_some() {
                Some(make_sequence_header::<u8>(enc, width, height, true, bit_depth)?)
            } else {
                None
            };
            (color, alpha)
        }
    };

    let frames: Vec<SerializeFrame<'_>> = color_frames.iter()
        .zip(durations_ms.iter())
        .enumerate()
        .map(|(i, (color_data, &dur))| {
            let alpha = alpha_frames.and_then(|af| af.get(i).map(|a| a.as_slice()));
            let frame = SerializeFrame::new(color_data, dur).with_sync(i == 0);
            if let Some(a) = alpha { frame.with_alpha(a) } else { frame }
        }).collect();

    let mut anim = AnimatedImage::new();
    anim.set_color_config(make_av1c_config(false, bit_depth));
    if alpha_frames.is_some() {
        anim.set_alpha_config(make_av1c_config(true, bit_depth));
    }
    let avif_file = anim.serialize(width as u32, height as u32, &frames, &color_seq_header, alpha_seq_header.as_deref());

    Ok(EncodedAnimation {
        avif_file,
        frame_count,
        total_duration_ms,
    })
}

// ---- Frame fill helpers (8-bit) ----

fn fill_frame_rgb8_420(
    frame: &mut Frame<u8>,
    width: usize,
    height: usize,
    img: ImgRef<'_, RGB8>,
) -> Result<(), Error> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let mut f = frame.planes.iter_mut();
    let mut y_plane = f.next().unwrap().mut_slice(Default::default());
    let mut u_plane = f.next().unwrap().mut_slice(Default::default());
    let mut v_plane = f.next().unwrap().mut_slice(Default::default());

    let mut y_rows = y_plane.rows_iter_mut();
    let mut u_rows = u_plane.rows_iter_mut();
    let mut v_rows = v_plane.rows_iter_mut();

    let mut u_acc: Vec<u32> = vec![0; chroma_width];
    let mut v_acc: Vec<u32> = vec![0; chroma_width];
    let mut count: Vec<u8> = vec![0; chroma_width];

    for row_idx in 0..height {
        let y_row = &mut y_rows.next().unwrap()[..width];

        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            let px = img[(col_idx, row_idx)];
            let yv = BT601[0] * f32::from(px.r) + BT601[1] * f32::from(px.g) + BT601[2] * f32::from(px.b);
            *y_out = yv.round().clamp(0.0, 255.0) as u8;

            let cx = col_idx / 2;
            let cb = (f32::from(px.b) - yv) * 0.5 / (1.0 - BT601[2]) + 128.0;
            let cr = (f32::from(px.r) - yv) * 0.5 / (1.0 - BT601[0]) + 128.0;

            u_acc[cx] += cb.round().clamp(0.0, 255.0) as u32;
            v_acc[cx] += cr.round().clamp(0.0, 255.0) as u32;
            if row_idx % 2 == 0 && col_idx % 2 == 0 {
                count[cx] = 1;
            } else {
                count[cx] += 1;
            }
        }

        if row_idx % 2 == 1 || row_idx == height - 1 {
            let chroma_row_idx = row_idx / 2;
            if chroma_row_idx < chroma_height {
                let u_row = &mut u_rows.next().unwrap()[..chroma_width];
                let v_row = &mut v_rows.next().unwrap()[..chroma_width];
                for cx in 0..chroma_width {
                    let c = u32::from(count[cx]);
                    u_row[cx] = ((u_acc[cx] + c / 2) / c) as u8;
                    v_row[cx] = ((v_acc[cx] + c / 2) / c) as u8;
                }
                u_acc.iter_mut().for_each(|v| *v = 0);
                v_acc.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(())
}

fn fill_frame_rgba8_color_420(
    frame: &mut Frame<u8>,
    width: usize,
    height: usize,
    img: ImgRef<'_, RGBA8>,
) -> Result<(), Error> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let mut f = frame.planes.iter_mut();
    let mut y_plane = f.next().unwrap().mut_slice(Default::default());
    let mut u_plane = f.next().unwrap().mut_slice(Default::default());
    let mut v_plane = f.next().unwrap().mut_slice(Default::default());

    let mut y_rows = y_plane.rows_iter_mut();
    let mut u_rows = u_plane.rows_iter_mut();
    let mut v_rows = v_plane.rows_iter_mut();

    let mut u_acc: Vec<u32> = vec![0; chroma_width];
    let mut v_acc: Vec<u32> = vec![0; chroma_width];
    let mut count: Vec<u8> = vec![0; chroma_width];

    for row_idx in 0..height {
        let y_row = &mut y_rows.next().unwrap()[..width];

        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            let px = img[(col_idx, row_idx)];
            let yv = BT601[0] * f32::from(px.r) + BT601[1] * f32::from(px.g) + BT601[2] * f32::from(px.b);
            *y_out = yv.round().clamp(0.0, 255.0) as u8;

            let cx = col_idx / 2;
            let cb = (f32::from(px.b) - yv) * 0.5 / (1.0 - BT601[2]) + 128.0;
            let cr = (f32::from(px.r) - yv) * 0.5 / (1.0 - BT601[0]) + 128.0;

            u_acc[cx] += cb.round().clamp(0.0, 255.0) as u32;
            v_acc[cx] += cr.round().clamp(0.0, 255.0) as u32;
            if row_idx % 2 == 0 && col_idx % 2 == 0 {
                count[cx] = 1;
            } else {
                count[cx] += 1;
            }
        }

        if row_idx % 2 == 1 || row_idx == height - 1 {
            let chroma_row_idx = row_idx / 2;
            if chroma_row_idx < chroma_height {
                let u_row = &mut u_rows.next().unwrap()[..chroma_width];
                let v_row = &mut v_rows.next().unwrap()[..chroma_width];
                for cx in 0..chroma_width {
                    let c = u32::from(count[cx]);
                    u_row[cx] = ((u_acc[cx] + c / 2) / c) as u8;
                    v_row[cx] = ((v_acc[cx] + c / 2) / c) as u8;
                }
                u_acc.iter_mut().for_each(|v| *v = 0);
                v_acc.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(())
}

fn fill_frame_alpha8(
    frame: &mut Frame<u8>,
    width: usize,
    height: usize,
    img: ImgRef<'_, RGBA8>,
) -> Result<(), Error> {
    let mut y_plane = frame.planes[0].mut_slice(Default::default());
    for (row_idx, y_row) in y_plane.rows_iter_mut().take(height).enumerate() {
        let y_row = &mut y_row[..width];
        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            *y_out = img[(col_idx, row_idx)].a;
        }
    }
    Ok(())
}

// ---- Frame fill helpers (16-bit / 10-bit) ----

/// Convert 10-bit RGB to 10-bit YCbCr 4:2:0 using BT.601 matrix.
fn fill_frame_rgb16_420(
    frame: &mut Frame<u16>,
    width: usize,
    height: usize,
    img: ImgRef<'_, rgb::RGB<u16>>,
) -> Result<(), Error> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let mut f = frame.planes.iter_mut();
    let mut y_plane = f.next().unwrap().mut_slice(Default::default());
    let mut u_plane = f.next().unwrap().mut_slice(Default::default());
    let mut v_plane = f.next().unwrap().mut_slice(Default::default());

    let mut y_rows = y_plane.rows_iter_mut();
    let mut u_rows = u_plane.rows_iter_mut();
    let mut v_rows = v_plane.rows_iter_mut();

    let mut u_acc: Vec<u64> = vec![0; chroma_width];
    let mut v_acc: Vec<u64> = vec![0; chroma_width];
    let mut count: Vec<u8> = vec![0; chroma_width];

    for row_idx in 0..height {
        let y_row = &mut y_rows.next().unwrap()[..width];

        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            let px = img[(col_idx, row_idx)];
            let r = f64::from(px.r);
            let g = f64::from(px.g);
            let b = f64::from(px.b);
            let yv = BT601[0] as f64 * r + BT601[1] as f64 * g + BT601[2] as f64 * b;
            *y_out = yv.round().clamp(0.0, 1023.0) as u16;

            let cx = col_idx / 2;
            let cb = (b - yv) * 0.5 / (1.0 - BT601[2] as f64) + 512.0;
            let cr = (r - yv) * 0.5 / (1.0 - BT601[0] as f64) + 512.0;

            u_acc[cx] += cb.round().clamp(0.0, 1023.0) as u64;
            v_acc[cx] += cr.round().clamp(0.0, 1023.0) as u64;
            if row_idx % 2 == 0 && col_idx % 2 == 0 {
                count[cx] = 1;
            } else {
                count[cx] += 1;
            }
        }

        if row_idx % 2 == 1 || row_idx == height - 1 {
            let chroma_row_idx = row_idx / 2;
            if chroma_row_idx < chroma_height {
                let u_row = &mut u_rows.next().unwrap()[..chroma_width];
                let v_row = &mut v_rows.next().unwrap()[..chroma_width];
                for cx in 0..chroma_width {
                    let c = u64::from(count[cx]);
                    u_row[cx] = ((u_acc[cx] + c / 2) / c) as u16;
                    v_row[cx] = ((v_acc[cx] + c / 2) / c) as u16;
                }
                u_acc.iter_mut().for_each(|v| *v = 0);
                v_acc.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(())
}

/// Convert 10-bit RGBA color channels to 10-bit YCbCr 4:2:0 (alpha ignored).
fn fill_frame_rgba16_color_420(
    frame: &mut Frame<u16>,
    width: usize,
    height: usize,
    img: ImgRef<'_, rgb::RGBA<u16>>,
) -> Result<(), Error> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let mut f = frame.planes.iter_mut();
    let mut y_plane = f.next().unwrap().mut_slice(Default::default());
    let mut u_plane = f.next().unwrap().mut_slice(Default::default());
    let mut v_plane = f.next().unwrap().mut_slice(Default::default());

    let mut y_rows = y_plane.rows_iter_mut();
    let mut u_rows = u_plane.rows_iter_mut();
    let mut v_rows = v_plane.rows_iter_mut();

    let mut u_acc: Vec<u64> = vec![0; chroma_width];
    let mut v_acc: Vec<u64> = vec![0; chroma_width];
    let mut count: Vec<u8> = vec![0; chroma_width];

    for row_idx in 0..height {
        let y_row = &mut y_rows.next().unwrap()[..width];

        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            let px = img[(col_idx, row_idx)];
            let r = f64::from(px.r);
            let g = f64::from(px.g);
            let b = f64::from(px.b);
            let yv = BT601[0] as f64 * r + BT601[1] as f64 * g + BT601[2] as f64 * b;
            *y_out = yv.round().clamp(0.0, 1023.0) as u16;

            let cx = col_idx / 2;
            let cb = (b - yv) * 0.5 / (1.0 - BT601[2] as f64) + 512.0;
            let cr = (r - yv) * 0.5 / (1.0 - BT601[0] as f64) + 512.0;

            u_acc[cx] += cb.round().clamp(0.0, 1023.0) as u64;
            v_acc[cx] += cr.round().clamp(0.0, 1023.0) as u64;
            if row_idx % 2 == 0 && col_idx % 2 == 0 {
                count[cx] = 1;
            } else {
                count[cx] += 1;
            }
        }

        if row_idx % 2 == 1 || row_idx == height - 1 {
            let chroma_row_idx = row_idx / 2;
            if chroma_row_idx < chroma_height {
                let u_row = &mut u_rows.next().unwrap()[..chroma_width];
                let v_row = &mut v_rows.next().unwrap()[..chroma_width];
                for cx in 0..chroma_width {
                    let c = u64::from(count[cx]);
                    u_row[cx] = ((u_acc[cx] + c / 2) / c) as u16;
                    v_row[cx] = ((v_acc[cx] + c / 2) / c) as u16;
                }
                u_acc.iter_mut().for_each(|v| *v = 0);
                v_acc.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(())
}

/// Fill alpha plane from 10-bit RGBA alpha channel (monochrome track).
fn fill_frame_alpha16(
    frame: &mut Frame<u16>,
    width: usize,
    height: usize,
    img: ImgRef<'_, rgb::RGBA<u16>>,
) -> Result<(), Error> {
    let mut y_plane = frame.planes[0].mut_slice(Default::default());
    for (row_idx, y_row) in y_plane.rows_iter_mut().take(height).enumerate() {
        let y_row = &mut y_row[..width];
        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            *y_out = img[(col_idx, row_idx)].a;
        }
    }
    Ok(())
}

/// Construct an Av1CBox configuration for the given bit depth.
fn make_av1c_config(is_alpha: bool, bit_depth: u8) -> Av1CBox {
    let mut config = Av1CBox::default();
    config.monochrome = is_alpha;
    if bit_depth > 8 {
        config.high_bitdepth = true;
        if bit_depth > 10 {
            config.twelve_bit = true;
        }
    }
    config
}
