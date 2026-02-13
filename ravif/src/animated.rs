//! Animated AVIF encoding
//!
//! Encodes a sequence of frames into an animated AVIF file using
//! rav1e's video encoding mode and a minimal ISOBMFF muxer.

use crate::av1encoder::SpeedTweaks;
use crate::error::Error;
use zenrav1e::prelude::*;
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

        let avif_file = mux_animated_avif(
            width as u32,
            height as u32,
            &encoded_frames,
            &durations_ms,
            seq_header,
        );

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

        let avif_file = if let Some(ref alpha) = alpha_frames {
            let alpha_seq_header = make_sequence_header(self, width, height, true)?;
            mux_animated_avif_with_alpha(
                width as u32,
                height as u32,
                &color_frames,
                alpha,
                &durations_ms,
                color_seq_header,
                alpha_seq_header,
            )
        } else {
            mux_animated_avif(
                width as u32,
                height as u32,
                &color_frames,
                &durations_ms,
                color_seq_header,
            )
        };

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

// ---- Minimal ISOBMFF muxer for animated AVIF ----

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Start a box, return position for later size patching
fn begin_box(out: &mut Vec<u8>, box_type: &[u8; 4]) -> usize {
    let pos = out.len();
    write_u32(out, 0); // placeholder
    out.extend_from_slice(box_type);
    pos
}

/// Patch box size
fn end_box(out: &mut Vec<u8>, pos: usize) {
    let size = (out.len() - pos) as u32;
    out[pos..pos + 4].copy_from_slice(&size.to_be_bytes());
}

fn write_fullbox(out: &mut Vec<u8>, version: u8, flags: u32) {
    out.push(version);
    out.push((flags >> 16) as u8);
    out.push((flags >> 8) as u8);
    out.push(flags as u8);
}

const STCO_PLACEHOLDER: u32 = 0xDEAD_BEEF;
const ILOC_PLACEHOLDER: u32 = 0xDEAD_BEE0;

fn mux_animated_avif(
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations_ms: &[u32],
    seq_header: Vec<u8>,
) -> Vec<u8> {
    let timescale: u32 = 1000;
    let total_duration: u64 = durations_ms.iter().map(|d| u64::from(*d)).sum();

    let mut out = Vec::new();

    write_ftyp(&mut out);
    write_meta(&mut out, width, height, &seq_header, frames[0].len() as u32, false);

    let moov_pos = begin_box(&mut out, b"moov");
    write_mvhd(&mut out, timescale, total_duration, 2);
    write_track(&mut out, 1, width, height, timescale, total_duration, frames, durations_ms, &seq_header, false);
    end_box(&mut out, moov_pos);

    let mdat_pos = begin_box(&mut out, b"mdat");
    let mdat_data_start = out.len();
    for frame in frames {
        out.extend_from_slice(frame);
    }
    end_box(&mut out, mdat_pos);

    patch_offset_placeholders(&mut out, &[mdat_data_start as u32], mdat_data_start as u32);

    out
}

fn mux_animated_avif_with_alpha(
    width: u32,
    height: u32,
    color_frames: &[Vec<u8>],
    alpha_frames: &[Vec<u8>],
    durations_ms: &[u32],
    color_seq_header: Vec<u8>,
    alpha_seq_header: Vec<u8>,
) -> Vec<u8> {
    let timescale: u32 = 1000;
    let total_duration: u64 = durations_ms.iter().map(|d| u64::from(*d)).sum();

    let mut out = Vec::new();

    write_ftyp(&mut out);
    write_meta(&mut out, width, height, &color_seq_header, color_frames[0].len() as u32, false);

    let moov_pos = begin_box(&mut out, b"moov");
    write_mvhd(&mut out, timescale, total_duration, 3);
    write_track(&mut out, 1, width, height, timescale, total_duration, color_frames, durations_ms, &color_seq_header, false);
    write_track(&mut out, 2, width, height, timescale, total_duration, alpha_frames, durations_ms, &alpha_seq_header, true);
    end_box(&mut out, moov_pos);

    let mdat_pos = begin_box(&mut out, b"mdat");
    let mdat_data_start = out.len();
    for frame in color_frames {
        out.extend_from_slice(frame);
    }
    let alpha_data_start = out.len();
    for frame in alpha_frames {
        out.extend_from_slice(frame);
    }
    end_box(&mut out, mdat_pos);

    patch_offset_placeholders(&mut out, &[mdat_data_start as u32, alpha_data_start as u32], mdat_data_start as u32);

    out
}

fn write_ftyp(out: &mut Vec<u8>) {
    let pos = begin_box(out, b"ftyp");
    out.extend_from_slice(b"avis");
    write_u32(out, 0);
    out.extend_from_slice(b"avisavifmif1miafiso8");
    end_box(out, pos);
}

/// Write a minimal `meta` box for AVIF sequence interoperability.
///
/// Declares item 1 as the primary item (av01) with ispe + av1C properties.
/// The iloc extent offset uses a placeholder patched after mdat is written.
fn write_meta(
    out: &mut Vec<u8>,
    width: u32,
    height: u32,
    seq_header: &[u8],
    first_frame_len: u32,
    is_alpha: bool,
) {
    let meta_pos = begin_box(out, b"meta");
    write_fullbox(out, 0, 0);

    // hdlr: handler = "pict"
    {
        let pos = begin_box(out, b"hdlr");
        write_fullbox(out, 0, 0);
        write_u32(out, 0); // pre_defined
        out.extend_from_slice(b"pict");
        out.extend_from_slice(&[0u8; 12]); // reserved
        out.push(0); // name (null-terminated empty string)
        end_box(out, pos);
    }

    // pitm: primary item ID = 1
    {
        let pos = begin_box(out, b"pitm");
        write_fullbox(out, 0, 0);
        write_u16(out, 1); // item_id
        end_box(out, pos);
    }

    // iloc: item 1 location (offset placeholder, patched after mdat)
    {
        let pos = begin_box(out, b"iloc");
        write_fullbox(out, 0, 0);
        // offset_size=4, length_size=4, base_offset_size=0, reserved=0
        out.push(0x44);
        out.push(0x00);
        write_u16(out, 1); // item_count
        write_u16(out, 1); // item_id
        write_u16(out, 0); // data_reference_index
        write_u16(out, 1); // extent_count
        write_u32(out, ILOC_PLACEHOLDER); // extent_offset (patched later)
        write_u32(out, first_frame_len); // extent_length
        end_box(out, pos);
    }

    // iinf: one infe entry for item 1
    {
        let iinf_pos = begin_box(out, b"iinf");
        write_fullbox(out, 0, 0);
        write_u16(out, 1); // entry_count

        let infe_pos = begin_box(out, b"infe");
        write_fullbox(out, 2, 0); // version 2 for item_type
        write_u16(out, 1); // item_id
        write_u16(out, 0); // item_protection_index
        out.extend_from_slice(b"av01"); // item_type
        out.push(0); // item_name (null-terminated empty)
        end_box(out, infe_pos);

        end_box(out, iinf_pos);
    }

    // iprp: item properties (ispe + av1C) associated with item 1
    {
        let iprp_pos = begin_box(out, b"iprp");

        // ipco: property container
        {
            let ipco_pos = begin_box(out, b"ipco");

            // Property 1: ispe (image spatial extents)
            {
                let pos = begin_box(out, b"ispe");
                write_fullbox(out, 0, 0);
                write_u32(out, width);
                write_u32(out, height);
                end_box(out, pos);
            }

            // Property 2: av1C (AV1 codec configuration)
            {
                let pos = begin_box(out, b"av1C");
                out.push(0x81); // marker=1, version=1
                out.push(0x04); // seq_profile=0, seq_level_idx=4
                if is_alpha {
                    out.push(0b0001_0110); // monochrome=1, chroma 4:2:0
                } else {
                    out.push(0b0000_1100); // monochrome=0, chroma 4:2:0
                }
                out.push(0x00); // no initial_presentation_delay
                out.extend_from_slice(seq_header);
                end_box(out, pos);
            }

            end_box(out, ipco_pos);
        }

        // ipma: associate properties with item 1
        {
            let pos = begin_box(out, b"ipma");
            write_fullbox(out, 0, 0);
            write_u32(out, 1); // entry_count
            write_u16(out, 1); // item_id
            out.push(2); // association_count
            out.push(0x01); // essential=0, property_index=1 (ispe)
            out.push(0x82); // essential=1, property_index=2 (av1C)
            end_box(out, pos);
        }

        end_box(out, iprp_pos);
    }

    end_box(out, meta_pos);
}

fn write_mvhd(out: &mut Vec<u8>, timescale: u32, duration: u64, next_track_id: u32) {
    let pos = begin_box(out, b"mvhd");
    write_fullbox(out, 1, 0);
    write_u64(out, 0); // creation_time
    write_u64(out, 0); // modification_time
    write_u32(out, timescale);
    write_u64(out, duration);
    write_u32(out, 0x0001_0000); // rate 1.0
    write_u16(out, 0x0100); // volume 1.0
    out.extend_from_slice(&[0u8; 10]); // reserved
    // Identity matrix
    for &v in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        write_u32(out, v);
    }
    out.extend_from_slice(&[0u8; 24]); // pre_defined
    write_u32(out, next_track_id);
    end_box(out, pos);
}

fn write_track(
    out: &mut Vec<u8>,
    track_id: u32,
    width: u32,
    height: u32,
    timescale: u32,
    duration: u64,
    frames: &[Vec<u8>],
    durations_ms: &[u32],
    seq_header: &[u8],
    is_alpha: bool,
) {
    let trak_pos = begin_box(out, b"trak");

    // tkhd
    {
        let pos = begin_box(out, b"tkhd");
        let flags = if is_alpha { 1 } else { 3 }; // enabled | in_movie
        write_fullbox(out, 1, flags);
        write_u64(out, 0); // creation_time
        write_u64(out, 0); // modification_time
        write_u32(out, track_id);
        write_u32(out, 0); // reserved
        write_u64(out, duration);
        out.extend_from_slice(&[0u8; 8]); // reserved
        write_u16(out, 0); // layer
        write_u16(out, 0); // alternate_group
        write_u16(out, 0); // volume
        write_u16(out, 0); // reserved
        for &v in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
            write_u32(out, v);
        }
        write_u32(out, width << 16);
        write_u32(out, height << 16);
        end_box(out, pos);
    }

    // mdia
    {
        let mdia_pos = begin_box(out, b"mdia");

        // mdhd
        {
            let pos = begin_box(out, b"mdhd");
            write_fullbox(out, 1, 0);
            write_u64(out, 0); // creation_time
            write_u64(out, 0); // modification_time
            write_u32(out, timescale);
            write_u64(out, duration);
            write_u16(out, 0x55C4); // language = "und"
            write_u16(out, 0);
            end_box(out, pos);
        }

        // hdlr
        {
            let pos = begin_box(out, b"hdlr");
            write_fullbox(out, 0, 0);
            write_u32(out, 0); // pre_defined
            if is_alpha {
                out.extend_from_slice(b"auxv");
            } else {
                out.extend_from_slice(b"pict");
            }
            out.extend_from_slice(&[0u8; 12]); // reserved
            out.extend_from_slice(if is_alpha { b"Alpha\0" } else { b"Color\0" });
            end_box(out, pos);
        }

        // minf
        {
            let minf_pos = begin_box(out, b"minf");

            // vmhd
            {
                let pos = begin_box(out, b"vmhd");
                write_fullbox(out, 0, 1);
                out.extend_from_slice(&[0u8; 8]); // graphicsmode + opcolor
                end_box(out, pos);
            }

            // dinf + dref
            {
                let dinf_pos = begin_box(out, b"dinf");
                let dref_pos = begin_box(out, b"dref");
                write_fullbox(out, 0, 0);
                write_u32(out, 1);
                let url_pos = begin_box(out, b"url ");
                write_fullbox(out, 0, 1); // self-contained
                end_box(out, url_pos);
                end_box(out, dref_pos);
                end_box(out, dinf_pos);
            }

            // stbl
            {
                let stbl_pos = begin_box(out, b"stbl");

                // stsd with av01 + av1C
                {
                    let pos = begin_box(out, b"stsd");
                    write_fullbox(out, 0, 0);
                    write_u32(out, 1);

                    let av01_pos = begin_box(out, b"av01");
                    out.extend_from_slice(&[0u8; 6]); // reserved
                    write_u16(out, 1); // data_reference_index
                    write_u16(out, 0); // pre_defined
                    write_u16(out, 0); // reserved
                    out.extend_from_slice(&[0u8; 12]); // pre_defined
                    write_u16(out, width as u16);
                    write_u16(out, height as u16);
                    write_u32(out, 0x0048_0000); // horiz resolution 72dpi
                    write_u32(out, 0x0048_0000); // vert resolution 72dpi
                    write_u32(out, 0); // reserved
                    write_u16(out, 1); // frame_count
                    out.extend_from_slice(&[0u8; 32]); // compressorname
                    write_u16(out, 0x0018); // depth = 24
                    out.extend_from_slice(&0xFFFFu16.to_be_bytes()); // pre_defined = -1

                    // av1C box
                    {
                        let av1c_pos = begin_box(out, b"av1C");
                        out.push(0x81); // marker=1, version=1
                        // seq_profile=0, seq_level_idx=4 (Level 2.0)
                        out.push(0x04);
                        if is_alpha {
                            // monochrome=1, chroma_sub_x=1, chroma_sub_y=1
                            out.push(0b0001_0110);
                        } else {
                            // monochrome=0, chroma_sub_x=1, chroma_sub_y=1
                            out.push(0b0000_1100);
                        }
                        out.push(0x00); // no initial_presentation_delay
                        out.extend_from_slice(seq_header);
                        end_box(out, av1c_pos);
                    }

                    end_box(out, av01_pos);
                    end_box(out, pos);
                }

                // stts (time-to-sample)
                {
                    let pos = begin_box(out, b"stts");
                    write_fullbox(out, 0, 0);
                    // Run-length encode
                    let mut entries: Vec<(u32, u32)> = Vec::new();
                    for &d in durations_ms {
                        if let Some(last) = entries.last_mut() {
                            if last.1 == d {
                                last.0 += 1;
                                continue;
                            }
                        }
                        entries.push((1, d));
                    }
                    write_u32(out, entries.len() as u32);
                    for (count, delta) in &entries {
                        write_u32(out, *count);
                        write_u32(out, *delta);
                    }
                    end_box(out, pos);
                }

                // stsc (sample-to-chunk: all in one chunk)
                {
                    let pos = begin_box(out, b"stsc");
                    write_fullbox(out, 0, 0);
                    write_u32(out, 1);
                    write_u32(out, 1); // first_chunk
                    write_u32(out, frames.len() as u32); // samples_per_chunk
                    write_u32(out, 1); // sample_description_index
                    end_box(out, pos);
                }

                // stsz (sample sizes)
                {
                    let pos = begin_box(out, b"stsz");
                    write_fullbox(out, 0, 0);
                    write_u32(out, 0); // variable size
                    write_u32(out, frames.len() as u32);
                    for frame in frames {
                        write_u32(out, frame.len() as u32);
                    }
                    end_box(out, pos);
                }

                // stco (chunk offset — placeholder, patched after mdat is written)
                {
                    let pos = begin_box(out, b"stco");
                    write_fullbox(out, 0, 0);
                    write_u32(out, 1);
                    write_u32(out, STCO_PLACEHOLDER);
                    end_box(out, pos);
                }

                // stss (sync samples — first frame is keyframe)
                {
                    let pos = begin_box(out, b"stss");
                    write_fullbox(out, 0, 0);
                    write_u32(out, 1);
                    write_u32(out, 1); // 1-indexed
                    end_box(out, pos);
                }

                end_box(out, stbl_pos);
            }

            end_box(out, minf_pos);
        }

        end_box(out, mdia_pos);
    }

    // tref for alpha track
    if is_alpha {
        let tref_pos = begin_box(out, b"tref");
        let auxl_pos = begin_box(out, b"auxl");
        write_u32(out, 1); // references track 1 (color)
        end_box(out, auxl_pos);
        end_box(out, tref_pos);
    }

    end_box(out, trak_pos);
}

/// Find and replace placeholder values with actual offsets.
/// Patches both STCO (track chunk offsets) and ILOC (item extent offsets).
fn patch_offset_placeholders(out: &mut Vec<u8>, stco_offsets: &[u32], iloc_offset: u32) {
    let stco_placeholder = STCO_PLACEHOLDER.to_be_bytes();
    let iloc_placeholder = ILOC_PLACEHOLDER.to_be_bytes();
    let mut stco_idx = 0;
    let mut i = 0;
    while i + 4 <= out.len() {
        if stco_idx < stco_offsets.len() && out[i..i + 4] == stco_placeholder {
            out[i..i + 4].copy_from_slice(&stco_offsets[stco_idx].to_be_bytes());
            stco_idx += 1;
            i += 4;
        } else if out[i..i + 4] == iloc_placeholder {
            out[i..i + 4].copy_from_slice(&iloc_offset.to_be_bytes());
            i += 4;
        } else {
            i += 1;
        }
    }
}
