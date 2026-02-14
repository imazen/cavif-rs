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

/// A single frame in an animated sequence
#[derive(Clone)]
pub struct AnimFrame<'a> {
    /// Frame pixel data (RGB8)
    pub rgb: ImgRef<'a, RGB8>,
    /// Duration of this frame in milliseconds
    pub duration_ms: u32,
}

/// A single frame with alpha in an animated sequence
#[derive(Clone)]
pub struct AnimFrameRgba<'a> {
    /// Frame pixel data (RGBA8)
    pub rgba: ImgRef<'a, RGBA8>,
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
    /// Encode a sequence of RGB frames into an animated AVIF.
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

        let encoded_frames = encode_sequence_av1(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgb_420(rav1e_frame, width, height, f.rgb)?;
                Ok(())
            },
            false,
        )?;

        let total_duration_ms: u64 = durations_ms.iter().map(|d| u64::from(*d)).sum();
        let frame_count = encoded_frames.len();

        let seq_header = make_sequence_header(self, width, height, false)?;

        let frames: Vec<SerializeFrame<'_>> = encoded_frames.iter().zip(durations_ms.iter()).enumerate().map(|(i, (data, &dur))| {
            SerializeFrame::new(data, dur).with_sync(i == 0)
        }).collect();

        let mut anim = AnimatedImage::new();
        anim.set_color_config(make_av1c_config(false));
        let avif_file = anim.serialize(width as u32, height as u32, &frames, &seq_header, None);

        Ok(EncodedAnimation {
            avif_file,
            frame_count,
            total_duration_ms,
        })
    }

    /// Encode a sequence of RGBA frames into an animated AVIF.
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
        let color_frames = encode_sequence_av1(
            self, width, height,
            frames.len(),
            |frame_idx, rav1e_frame| {
                let f = &frames[frame_idx];
                fill_frame_rgba_color_420(rav1e_frame, width, height, f.rgba)?;
                Ok(())
            },
            false,
        )?;

        // Encode alpha track if needed
        let alpha_frames = if has_alpha {
            Some(encode_sequence_av1(
                self, width, height,
                frames.len(),
                |frame_idx, rav1e_frame| {
                    let f = &frames[frame_idx];
                    fill_frame_alpha(rav1e_frame, width, height, f.rgba)?;
                    Ok(())
                },
                true,
            )?)
        } else {
            None
        };

        let total_duration_ms: u64 = durations_ms.iter().map(|d| u64::from(*d)).sum();
        let frame_count = color_frames.len();

        let color_seq_header = make_sequence_header(self, width, height, false)?;
        let alpha_seq_header = if alpha_frames.is_some() {
            Some(make_sequence_header(self, width, height, true)?)
        } else {
            None
        };

        let frames: Vec<SerializeFrame<'_>> = color_frames.iter()
            .zip(durations_ms.iter())
            .enumerate()
            .map(|(i, (color_data, &dur))| {
                let alpha = alpha_frames.as_ref().and_then(|af| af.get(i).map(|a| a.as_slice()));
                let frame = SerializeFrame::new(color_data, dur).with_sync(i == 0);
                if let Some(a) = alpha { frame.with_alpha(a) } else { frame }
            }).collect();

        let mut anim = AnimatedImage::new();
        anim.set_color_config(make_av1c_config(false));
        if alpha_frames.is_some() {
            anim.set_alpha_config(make_av1c_config(true));
        }
        let avif_file = anim.serialize(width as u32, height as u32, &frames, &color_seq_header, alpha_seq_header.as_deref());

        Ok(EncodedAnimation {
            avif_file,
            frame_count,
            total_duration_ms,
        })
    }
}

// ---- Encoding helpers ----

fn encode_sequence_av1(
    enc: &crate::Encoder<'_>,
    width: usize,
    height: usize,
    num_frames: usize,
    init_frame: impl Fn(usize, &mut Frame<u8>) -> Result<(), Error>,
    is_alpha: bool,
) -> Result<Vec<Vec<u8>>, Error> {
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
        bit_depth: 8,
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
        max_pixel_count: u64::MAX,
        speed_settings: speed.speed_settings(),
    };

    let cfg = Config::new().with_encoder_config(config);
    let mut ctx: Context<u8> = cfg.new_context()?;

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
        result.push(p.ok_or_else(|| Error::Unsupported("frame was not encoded"))?);
    }
    Ok(result)
}

fn make_sequence_header(
    enc: &crate::Encoder<'_>,
    width: usize,
    height: usize,
    is_alpha: bool,
) -> Result<Vec<u8>, Error> {
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
        bit_depth: 8,
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
        max_pixel_count: u64::MAX,
        speed_settings: speed.speed_settings(),
    };
    let cfg = Config::new().with_encoder_config(config);
    let ctx: Context<u8> = cfg.new_context()?;
    Ok(ctx.container_sequence_header())
}

// ---- Frame fill helpers ----

fn fill_frame_rgb_420(
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

fn fill_frame_rgba_color_420(
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

fn fill_frame_alpha(
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

/// Construct an Av1CBox for 8-bit 4:2:0 (color) or monochrome (alpha).
fn make_av1c_config(is_alpha: bool) -> Av1CBox {
    let mut config = Av1CBox::default();
    config.monochrome = is_alpha;
    config
}


