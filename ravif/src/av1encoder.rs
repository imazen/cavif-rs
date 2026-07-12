#![allow(deprecated)]
use std::borrow::Cow;
use crate::cancel::CancellationToken;
use crate::dirtyalpha::blurred_dirty_alpha;
use crate::error::Error;
use crate::Result;
use whereat::{at, ResultAtExt as _};
#[cfg(not(feature = "threading"))]
use crate::rayoff as rayon;
use imgref::{Img, ImgVec};
use zenrav1e::prelude::*;
use rgb::{RGB8, RGBA8};

/// Helper to check cancellation with minimal overhead
/// Returns Error::Cancelled if cancellation is requested
#[inline(always)]
fn check_cancellation(
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> core::result::Result<(), Error> {
    if cancel_token.is_some_and(|t| t.is_cancelled()) {
        return Err(Error::Cancelled);
    }
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// For [`Encoder::with_internal_color_model`]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ColorModel {
    /// Standard color model for photographic content. Usually the best choice.
    /// This library always uses full-resolution color (4:4:4).
    /// This library will automatically choose between BT.601 or BT.709.
    YCbCr,
    /// RGB channels are encoded without color space transformation.
    /// Usually results in larger file sizes, and is less compatible than `YCbCr`.
    /// Use only if the content really makes use of RGB, e.g. anaglyph images or RGB subpixel anti-aliasing.
    RGB,
}

/// Chroma subsampling mode. For [`Encoder::with_chroma_subsampling`]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum ChromaSubsampling {
    /// Full-resolution chroma (4:4:4). Best quality, larger files.
    /// This is the default and generally recommended for AVIF.
    #[default]
    Yuv444,
    /// Half-resolution chroma in both dimensions (4:2:0).
    /// Reduces file size by ~25-35% with minimal quality loss on photographic content.
    /// Not recommended for text, sharp edges, or synthetic images.
    /// Cannot be used with [`ColorModel::RGB`].
    Yuv420,
}

/// Handling of color channels in transparent images. For [`Encoder::with_alpha_color_mode`]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AlphaColorMode {
    /// Use unassociated alpha channel and leave color channels unchanged, even if there's redundant color data in transparent areas.
    UnassociatedDirty,
    /// Use unassociated alpha channel, but set color channels of transparent areas to a solid color to eliminate invisible data and improve compression.
    UnassociatedClean,
    /// Store color channels of transparent images in premultiplied form.
    /// This requires support for premultiplied alpha in AVIF decoders.
    ///
    /// It may reduce file sizes due to clearing of fully-transparent pixels, but
    /// may also increase file sizes due to creation of new edges in the color channels.
    ///
    /// Note that this is only internal detail for the AVIF file.
    /// It does not change meaning of `RGBA` in this library — it's always unassociated.
    Premultiplied,
}

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
pub enum BitDepth {
    Eight,
    Ten,
    Twelve,
    /// Same as `Ten`
    #[default]
    Auto,
}

/// Pre-encoded gain map data for embedding in an AVIF file.
///
/// Contains an already-encoded AV1 bitstream of the gain map image
/// plus the ISO 21496-1 binary metadata describing how to apply it.
///
/// The gain map is used for SDR/HDR tone mapping: the SDR base image
/// is stored as the primary item, and the gain map allows reconstruction
/// of the HDR rendition.
#[derive(Debug, Clone)]
pub struct GainMapData {
    /// Pre-encoded AV1 bitstream of the gain map image.
    pub av1_data: Vec<u8>,
    /// Width of the gain map image in pixels.
    pub width: u32,
    /// Height of the gain map image in pixels.
    pub height: u32,
    /// Bit depth of the gain map AV1 data (typically 8 or 10).
    pub bit_depth: u8,
    /// ISO 21496-1 binary metadata blob.
    pub metadata: Vec<u8>,
    /// CICP color description of the **alternate rendition**, written as a
    /// `colr` (nclx) property on the `tmap` item: `(color_primaries,
    /// transfer_characteristics, matrix_coefficients, full_range)` as raw
    /// ITU-T H.273 code points. Code points outside the muxer's supported
    /// set fail the encode with [`Error::Unsupported`] rather than being
    /// silently dropped.
    pub alt_colr_cicp: Option<(u8, u8, u8, bool)>,
    /// Chroma subsampling `(horizontal, vertical)` of the gain-map AV1
    /// payload — written into the gain-map item's `av1C`, which must
    /// describe the actual bitstream. `(false, false)` = 4:4:4,
    /// `(true, true)` = 4:2:0.
    pub chroma_subsampling: (bool, bool),
    /// Whether the gain-map AV1 payload is monochrome (av1C flag).
    pub monochrome: bool,
    /// ICC profile of the **alternate rendition**, written as a `colr` box
    /// of type `prof` on the `tmap` item. May be combined with
    /// `alt_colr_cicp` (ISOBMFF allows one `colr` of each type per item).
    pub alt_icc: Option<Vec<u8>>,
}

/// Map raw H.273 CICP code points onto the muxer's `ColrBox` enums,
/// erroring honestly on code points the muxer cannot represent.
fn gain_map_alt_colr_box(
    p: u8,
    t: u8,
    m: u8,
    full_range: bool,
) -> Result<zenavif_serialize::ColrBox> {
    use zenavif_serialize::constants::{
        ColorPrimaries as CP, MatrixCoefficients as MC, TransferCharacteristics as TC,
    };
    let color_primaries = match p {
        1 => CP::Bt709,
        2 => CP::Unspecified,
        6 => CP::Bt601,
        9 => CP::Bt2020,
        11 => CP::DciP3,
        12 => CP::DisplayP3,
        _ => return Err(at!(Error::Unsupported("gain map alt colr: color primaries code point"))),
    };
    let transfer_characteristics = match t {
        1 => TC::Bt709,
        2 => TC::Unspecified,
        6 => TC::Bt601,
        7 => TC::Smpte240,
        8 => TC::Linear,
        9 => TC::Log,
        10 => TC::LogSqrt,
        11 => TC::Iec61966,
        13 => TC::Srgb,
        14 => TC::Bt2020_10,
        15 => TC::Bt2020_12,
        16 => TC::Smpte2084,
        17 => TC::Smpte428,
        18 => TC::Hlg,
        _ => {
            return Err(at!(Error::Unsupported(
                "gain map alt colr: transfer characteristics code point",
            )));
        }
    };
    let matrix_coefficients = match m {
        0 => MC::Rgb,
        1 => MC::Bt709,
        2 => MC::Unspecified,
        6 => MC::Bt601,
        8 => MC::Ycgco,
        9 => MC::Bt2020Ncl,
        10 => MC::Bt2020Cl,
        _ => return Err(at!(Error::Unsupported("gain map alt colr: matrix coefficients code point"))),
    };
    // `#[non_exhaustive]` prohibits literal construction outside the
    // defining crate — build via Default + field assignment.
    let mut colr = zenavif_serialize::ColrBox::default();
    colr.color_primaries = color_primaries;
    colr.transfer_characteristics = transfer_characteristics;
    colr.matrix_coefficients = matrix_coefficients;
    colr.full_range_flag = full_range;
    Ok(colr)
}

/// The newly-created image file + extra info FYI
#[non_exhaustive]
#[derive(Clone)]
pub struct EncodedImage {
    /// AVIF (HEIF+AV1) encoded image data
    pub avif_file: Vec<u8>,
    /// FYI: number of bytes of AV1 payload used for the color
    pub color_byte_size: usize,
    /// FYI: number of bytes of AV1 payload used for the alpha channel
    pub alpha_byte_size: usize,
}

/// Encoder config builder
///
/// The lifetime is relevant only for [`Encoder::with_exif()`]. Use `Encoder<'static>` if Rust complains.
#[derive(Debug, Clone)]
pub struct Encoder<'exif_slice> {
    /// 0-255 scale
    pub(crate) quantizer: u8,
    /// 0-255 scale
    pub(crate) alpha_quantizer: u8,
    /// User-supplied quality value, retained verbatim for [`Encoder::validate`].
    /// `None` means the setter was never called (i.e. default).
    pub(crate) quality_input: Option<f32>,
    /// User-supplied alpha quality, retained for validation.
    pub(crate) alpha_quality_input: Option<f32>,
    /// User-supplied libavif quality, retained for validation.
    pub(crate) libavif_quality_input: Option<f32>,
    /// rav1e preset 1 (slow) 10 (fast but crappy)
    pub(crate) speed: u8,
    /// True if RGBA input has already been premultiplied. It inserts appropriate metadata.
    premultiplied_alpha: bool,
    /// Which pixel format to use in AVIF file. RGB tends to give larger files.
    color_model: ColorModel,
    /// How many threads should be used (0 = match core count), None - use global rayon thread pool
    threads: Option<usize>,
    /// [`AlphaColorMode`]
    alpha_color_mode: AlphaColorMode,
    /// 8 or 10
    output_depth: BitDepth,
    /// [`ChromaSubsampling`]
    chroma_subsampling: ChromaSubsampling,
    /// Dropped into MPEG infe BOX
    exif: Option<Cow<'exif_slice, [u8]>>,
    /// Optional cancellation token for interrupting encoding
    cancellation_token: Option<CancellationToken>,
    /// Optional cooperative stop token (from zencodec/enough).
    /// When the `stop` feature is enabled, this is forwarded to zenrav1e's
    /// per-superblock cancellation via `Context::set_stop()`.
    #[cfg(feature = "stop")]
    stop_token: Option<almost_enough::StopToken>,
    /// Optional timeout duration for encoding
    timeout: Option<std::time::Duration>,
    /// Override color primaries (default: BT709 for sRGB)
    pub(crate) color_primaries: Option<ColorPrimaries>,
    /// Override transfer characteristics (default: SRGB)
    pub(crate) transfer_characteristics: Option<TransferCharacteristics>,
    /// Override pixel range (default: Full)
    pixel_range: Option<PixelRange>,
    /// HDR mastering display metadata (SMPTE ST 2086)
    pub(crate) mastering_display: Option<MasteringDisplay>,
    /// HDR content light level metadata (CEA-861.3)
    pub(crate) content_light: Option<ContentLight>,
    /// Image rotation (counter-clockwise degrees: 0, 90, 180, 270)
    rotation: Option<u8>,
    /// Image mirror axis (0 = vertical/left-right, 1 = horizontal/top-bottom)
    mirror: Option<u8>,
    /// ICC color profile
    icc_profile: Option<Vec<u8>>,
    /// XMP metadata
    xmp: Option<Vec<u8>>,
    /// Pre-encoded gain map for UltraHDR / ISO 21496-1
    gain_map: Option<GainMapData>,
    /// Enable AV1 quantization matrices (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    pub(crate) enable_qm: bool,
    /// Enable variance adaptive quantization (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    enable_vaq: bool,
    /// VAQ strength 0.0–4.0 (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    vaq_strength: f64,
    /// Use Tune::StillImage instead of Tune::Psychovisual (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    tune_still_image: bool,
    /// Mathematically lossless encoding (quantizer=0) (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    lossless: bool,
    /// Segmentation boost power (1.0 = off, >1.0 = wider QP deltas)
    #[cfg(feature = "imazen")]
    seg_boost: f64,
    /// Override CDEF on/off (None = use speed preset default)
    #[cfg(feature = "imazen")]
    override_cdef: Option<bool>,
    /// Override rdo_tx_decision on/off (None = use speed preset default)
    #[cfg(feature = "imazen")]
    override_rdo_tx_decision: Option<bool>,
    /// Override SGR complexity to Full (all 16 parameter sets vs 8 at speed ≥5)
    #[cfg(feature = "imazen")]
    override_sgr_complexity: Option<bool>,
    /// Override LRU on skip (search loop restoration on skip blocks)
    #[cfg(feature = "imazen")]
    override_lru_on_skip: Option<bool>,
    /// Override segmentation to Complex (k-means vs Simple at speed ≥3)
    #[cfg(feature = "imazen")]
    override_segmentation_complex: Option<bool>,
    /// Override bottom-up partition search (vs top-down at speed ≥4)
    #[cfg(feature = "imazen")]
    override_encode_bottomup: Option<bool>,
    /// Override partition block-size range (min, max) in pixels.
    /// Valid sizes: 4, 8, 16, 32, 64, 128. Smaller mins help screen/text;
    /// larger maxes help smooth photo content.
    #[cfg(feature = "imazen")]
    override_partition_range: Option<(u8, u8)>,
    /// Override prediction-modes setting. `Some(true)` = ComplexAll (slowest,
    /// all intra modes). `Some(false)` = Simple (fastest). `None` = preset.
    /// Note: ComplexAll currently disabled by default for stills (zenrav1e#5).
    #[cfg(feature = "imazen")]
    override_complex_prediction_modes: Option<bool>,
    /// Override loop restoration filter (Wiener / SGR). Helps smooth/noisy
    /// content; can soften line art.
    #[cfg(feature = "imazen")]
    override_lrf: Option<bool>,
    /// Override fast deblock vs full deblock. Off (full) preserves edges
    /// better; on (fast) reduces blocking artifacts faster.
    #[cfg(feature = "imazen")]
    override_fast_deblock: Option<bool>,
    /// Per-superblock AC quantizer scale map for the color encode
    /// (closed-loop second pass; see `expert::InternalParams::sb_q_scale`).
    #[cfg(feature = "imazen")]
    override_sb_q_scale: Option<Box<[f32]>>,
    /// Fast-tier budget passthroughs (see the matching
    /// `expert::InternalParams` fields): tx-size/type/reduced-set/num-modes
    /// overrides applied onto `SpeedTweaks` after the preset.
    #[cfg(feature = "imazen")]
    override_rdo_tx_size: Option<bool>,
    #[cfg(feature = "imazen")]
    override_rdo_tx_size_depth: Option<u8>,
    #[cfg(feature = "imazen")]
    override_rdo_tx_type: Option<bool>,
    #[cfg(feature = "imazen")]
    override_reduced_tx_set: Option<bool>,
    #[cfg(feature = "imazen")]
    override_num_modes_rdo: Option<u8>,
    /// Rect-partition liveness threshold in pixels (mapped to `BlockSize`
    /// at apply time).
    #[cfg(feature = "imazen")]
    override_non_square_max_px: Option<u8>,
    /// The topdown-prune gate quartet, overriding AS A UNIT when
    /// `Some` (a `None` inside the unit clears that gate) — see
    /// `expert::InternalParams::prune_none_breakout`.
    #[cfg(feature = "imazen")]
    override_prune: Option<PruneQuartet>,
    /// Screen-content palette mode for the color/gray streams (zenrav1e
    /// `PaletteMode`; the alpha stream never receives it). See
    /// [`Encoder::with_palette`].
    palette_mode: Option<PaletteMode>,
    /// Enable trellis quantization (Viterbi DP coefficient optimization)
    #[cfg(feature = "imazen")]
    enable_trellis: bool,
    /// Pre-flight pixel cap: `width * height` must not exceed this before
    /// encoding starts. Defaults to 120 megapixels. `0` disables the cap
    /// (unlimited). See [`Encoder::with_max_pixels`]. Also forwarded to
    /// zenrav1e's own `max_pixel_count` guard so it isn't nulled.
    pub(crate) max_pixels: u64,
}

impl<'exif_slice> Default for Encoder<'exif_slice> {
    fn default() -> Self {
        Self {
            quantizer: quality_to_quantizer(80.),
            alpha_quantizer: quality_to_quantizer(80.),
            quality_input: None,
            alpha_quality_input: None,
            libavif_quality_input: None,
            speed: 5,
            output_depth: BitDepth::default(),
            chroma_subsampling: ChromaSubsampling::default(),
            premultiplied_alpha: false,
            color_model: ColorModel::YCbCr,
            threads: None,
            exif: None,
            alpha_color_mode: AlphaColorMode::UnassociatedClean,
            cancellation_token: None,
            #[cfg(feature = "stop")]
            stop_token: None,
            timeout: None,
            color_primaries: None,
            transfer_characteristics: None,
            pixel_range: None,
            mastering_display: None,
            content_light: None,
            rotation: None,
            mirror: None,
            icc_profile: None,
            xmp: None,
            gain_map: None,
            #[cfg(feature = "imazen")]
            enable_qm: true,
            #[cfg(feature = "imazen")]
            enable_vaq: false,
            #[cfg(feature = "imazen")]
            vaq_strength: 1.0,
            #[cfg(feature = "imazen")]
            tune_still_image: false,
            #[cfg(feature = "imazen")]
            lossless: false,
            #[cfg(feature = "imazen")]
            seg_boost: 1.0,
            #[cfg(feature = "imazen")]
            override_cdef: None,
            #[cfg(feature = "imazen")]
            override_rdo_tx_decision: None,
            #[cfg(feature = "imazen")]
            override_sgr_complexity: None,
            #[cfg(feature = "imazen")]
            override_lru_on_skip: None,
            #[cfg(feature = "imazen")]
            override_segmentation_complex: None,
            #[cfg(feature = "imazen")]
            override_encode_bottomup: None,
            #[cfg(feature = "imazen")]
            override_partition_range: None,
            #[cfg(feature = "imazen")]
            override_complex_prediction_modes: None,
            #[cfg(feature = "imazen")]
            override_lrf: None,
            #[cfg(feature = "imazen")]
            override_fast_deblock: None,
            #[cfg(feature = "imazen")]
            override_sb_q_scale: None,
            #[cfg(feature = "imazen")]
            override_rdo_tx_size: None,
            #[cfg(feature = "imazen")]
            override_rdo_tx_size_depth: None,
            #[cfg(feature = "imazen")]
            override_rdo_tx_type: None,
            #[cfg(feature = "imazen")]
            override_reduced_tx_set: None,
            #[cfg(feature = "imazen")]
            override_num_modes_rdo: None,
            #[cfg(feature = "imazen")]
            override_non_square_max_px: None,
            #[cfg(feature = "imazen")]
            override_prune: None,
            palette_mode: None,
            #[cfg(feature = "imazen")]
            enable_trellis: false,
            max_pixels: DEFAULT_MAX_PIXELS,
        }
    }
}

/// Default pre-flight pixel cap: 120 megapixels.
///
/// Large enough to admit current high-resolution phone/camera stills
/// (e.g. 108 MP sensors) while rejecting attacker-controlled dimensions
/// that would otherwise allocate unbounded memory before any encoding
/// guard fires. Matches zenrav1e's own default `max_pixel_count`.
pub const DEFAULT_MAX_PIXELS: u64 = 120_000_000;

/// Builder methods
impl<'exif_slice> Encoder<'exif_slice> {
    /// Start here
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Quality `1..=100`. Out-of-range values are silently clamped to the
    /// valid range during encoding; call [`Encoder::validate`] before encoding
    /// for fail-fast behaviour.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_quality(mut self, quality: f32) -> Self {
        self.quality_input = Some(quality);
        self.quantizer = quality_to_quantizer(quality);
        self
    }

    #[doc(hidden)]
    #[deprecated(note = "Renamed to with_bit_depth")]
    #[must_use]
    pub fn with_depth(self, depth: Option<u8>) -> Self {
        self.with_bit_depth(depth.map(|d| if d >= 10 { BitDepth::Ten } else { BitDepth::Eight }).unwrap_or(BitDepth::Auto))
    }

    /// Internal precision to use in the encoded AV1 data, for both color and alpha. 10-bit depth works best, even for 8-bit inputs/outputs.
    ///
    /// Use 8-bit depth only as a workaround for decoders that need it.
    ///
    /// This setting does not affect pixel inputs for this library.
    #[inline(always)]
    #[must_use]
    pub fn with_bit_depth(mut self, depth: BitDepth) -> Self {
        self.output_depth = depth;
        self
    }

    /// Quality for the alpha channel only. `1..=100`. Out-of-range values are
    /// silently clamped during encoding; call [`Encoder::validate`] before
    /// encoding for fail-fast behaviour.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_alpha_quality(mut self, quality: f32) -> Self {
        self.alpha_quality_input = Some(quality);
        self.alpha_quantizer = quality_to_quantizer(quality);
        self
    }

    /// Quality `1..=100` using libavif-compatible linear mapping.
    ///
    /// Use this when you want Q numbers to match avifenc behavior.
    /// At the same Q, this produces similar quality to avifenc but
    /// with ~7% smaller files due to rav1e's efficiency.
    /// Quality `1..=100` using libavif-compatible linear mapping.
    ///
    /// Produces the same perceived visual quality (Butteraugli error) as avifenc at the
    /// same Q number. Use this for fair comparisons against libavif/avifenc.
    ///
    /// Note: At the same perceived quality, zenravif typically produces smaller files
    /// due to rav1e's superior encoding efficiency. This advantage appears when
    /// comparing images with matched visual quality scores, not matched Q numbers.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_libavif_quality(mut self, quality: f32) -> Self {
        self.libavif_quality_input = Some(quality);
        let q = quality.clamp(1., 100.);
        // Use exact libavif mapping: qindex = (100 - q) * 255 / 100
        self.quantizer = ((100. - q) * 255. / 100.).round() as u8;
        self
    }

    /// * 1 = very very slow, but max compression.
    /// * 10 = quick, but larger file sizes and lower quality.
    ///
    /// Values outside `1..=10` are silently accepted (the encoder treats
    /// `speed > 10` as the fastest preset and `speed = 0` as the slowest).
    /// Call [`Encoder::validate`] before encoding for fail-fast behaviour.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_speed(mut self, speed: u8) -> Self {
        self.speed = speed;
        self
    }

    /// Maximum number of pixels (`width * height`) any single encode is
    /// allowed to process. The encode functions reject larger inputs
    /// **pre-flight** — before allocating planes or building the rav1e
    /// context — returning [`Error::TooManyPixels`].
    ///
    /// The default is [`DEFAULT_MAX_PIXELS`] (120 megapixels), which admits
    /// current high-resolution stills while bounding memory for
    /// attacker-controlled dimensions on a server. This value is also
    /// forwarded to zenrav1e's own `max_pixel_count` guard.
    ///
    /// Pass `0` to **disable** the cap (unlimited). Only do this when the
    /// dimensions are already trusted/bounded by the caller.
    ///
    /// ```
    /// use zenravif::Encoder;
    /// // Allow up to 200 megapixels:
    /// let enc = Encoder::new().with_max_pixels(200_000_000);
    /// // Or remove the cap entirely (trusted input only):
    /// let unlimited = Encoder::new().with_max_pixels(0);
    /// ```
    #[inline(always)]
    #[must_use]
    pub fn with_max_pixels(mut self, max_pixels: u64) -> Self {
        self.max_pixels = max_pixels;
        self
    }

    /// Changes how color channels are stored in the image. The default is YCbCr.
    ///
    /// Note that this is only internal detail for the AVIF file, and doesn't
    /// change color model of inputs to encode functions.
    #[inline(always)]
    #[must_use]
    pub fn with_internal_color_model(mut self, color_model: ColorModel) -> Self {
        self.color_model = color_model;
        self
    }

    #[doc(hidden)]
    #[deprecated = "Renamed to `with_internal_color_model()`"]
    #[must_use]
    pub fn with_internal_color_space(self, color_model: ColorModel) -> Self {
        self.with_internal_color_model(color_model)
    }

    /// Configures `rayon` thread pool size.
    /// The default `None` is to use all threads in the default `rayon` thread pool.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_num_threads(mut self, num_threads: Option<usize>) -> Self {
        self.threads = num_threads;
        self
    }

    /// Configure handling of color channels in transparent images
    ///
    /// Note that this doesn't affect input format for this library,
    /// which for RGBA is always uncorrelated alpha.
    #[inline(always)]
    #[must_use]
    pub fn with_alpha_color_mode(mut self, mode: AlphaColorMode) -> Self {
        self.alpha_color_mode = mode;
        self.premultiplied_alpha = mode == AlphaColorMode::Premultiplied;
        self
    }

    /// Set chroma subsampling mode.
    ///
    /// [`ChromaSubsampling::Yuv444`] (default) keeps full-resolution chroma for best quality.
    /// [`ChromaSubsampling::Yuv420`] halves chroma resolution in both dimensions,
    /// reducing file size by ~25-35% with minimal quality loss on photographic content.
    ///
    /// Cannot be combined with [`ColorModel::RGB`].
    #[inline(always)]
    #[must_use]
    pub fn with_chroma_subsampling(mut self, subsampling: ChromaSubsampling) -> Self {
        self.chroma_subsampling = subsampling;
        self
    }

    /// Embedded into AVIF file as-is
    ///
    /// The data can be `Vec<u8>`, or `&[u8]` if the encoder instance doesn't leave its scope.
    pub fn with_exif(mut self, exif_data: impl Into<Cow<'exif_slice, [u8]>>) -> Self {
        self.set_exif(exif_data);
        self
    }

    /// Embedded into AVIF file as-is
    ///
    /// The data can be `Vec<u8>`, or `&[u8]` if the encoder instance doesn't leave its scope.
    pub fn set_exif(&mut self, exif_data: impl Into<Cow<'exif_slice, [u8]>>) {
        self.exif = Some(exif_data.into());
    }

    /// Set a cancellation token for interrupting encoding
    ///
    /// The encoder checks the token on every packet iteration (~5-15ns overhead per check)
    /// and returns `Error::Cancelled` if cancellation is requested.
    ///
    /// The cancellation token can be cloned and cancelled from another thread.
    /// Actual response time depends on encoding speed (10-200ms at typical speeds).
    #[inline(always)]
    #[must_use]
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Set a cooperative stop token for per-superblock cancellation.
    ///
    /// When the `stop` feature is enabled, this token is forwarded to
    /// zenrav1e's `Context::set_stop()`, enabling cancellation during
    /// encoding (not just between packets). The encoder checks the token
    /// once per superblock (~64x64 pixels), providing sub-millisecond
    /// cancellation response.
    ///
    /// This is the preferred cancellation mechanism for integration with
    /// the `enough` crate's cooperative cancellation framework.
    #[cfg(feature = "stop")]
    #[inline(always)]
    #[must_use]
    pub fn with_stop(mut self, stop: almost_enough::StopToken) -> Self {
        self.stop_token = Some(stop);
        self
    }

    /// Set a timeout for encoding
    ///
    /// If encoding takes longer than the specified duration, it will be cancelled
    /// and return `Error::Cancelled`.
    ///
    /// The timeout is checked every 10ms or every 10 packets (whichever comes first),
    /// providing responsive cancellation with minimal overhead (~20-50ns per check).
    ///
    /// # Example
    ///
    /// ```
    /// use zenravif::*;
    /// use std::time::Duration;
    /// # fn example(pixels: &[RGBA8], width: usize, height: usize) {
    ///
    /// let encoder = Encoder::new()
    ///     .with_quality(70.0)
    ///     .with_timeout(Duration::from_millis(100));
    ///
    /// match encoder.encode_rgba(Img::new(pixels, width, height)) {
    ///     Ok(result) => println!("Encoded successfully"),
    ///     // The encode error is `At<Error>`; borrow the inner error to match it.
    ///     Err(e) if matches!(e.error(), Error::Cancelled) => println!("Encoding timed out"),
    ///     Err(e) => eprintln!("Error: {:?} at {}", e.error(), e.full_trace()),
    /// }
    /// # }
    /// ```
    #[inline(always)]
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set color primaries for the encoded image.
    ///
    /// Default is [`ColorPrimaries::BT709`] (sRGB). Use [`ColorPrimaries::BT2020`] for
    /// wide gamut / HDR content, or [`ColorPrimaries::SMPTE432`] for Display P3.
    ///
    /// This must match the actual color space of the input pixels.
    #[inline(always)]
    #[must_use]
    pub fn with_color_primaries(mut self, cp: ColorPrimaries) -> Self {
        self.color_primaries = Some(cp);
        self
    }

    /// Set transfer characteristics for the encoded image.
    ///
    /// Default is [`TransferCharacteristics::SRGB`]. Use [`TransferCharacteristics::SMPTE2084`]
    /// for PQ (HDR10) or [`TransferCharacteristics::HLG`] for HLG.
    ///
    /// This must match the actual transfer function of the input pixels.
    #[inline(always)]
    #[must_use]
    pub fn with_transfer_characteristics(mut self, tc: TransferCharacteristics) -> Self {
        self.transfer_characteristics = Some(tc);
        self
    }

    /// Set pixel value range.
    ///
    /// Default is [`PixelRange::Full`]. Use [`PixelRange::Limited`] for broadcast/studio
    /// content with limited (narrow) range values.
    #[inline(always)]
    #[must_use]
    pub fn with_pixel_range(mut self, range: PixelRange) -> Self {
        self.pixel_range = Some(range);
        self
    }

    /// Set HDR mastering display color volume metadata (SMPTE ST 2086).
    ///
    /// This metadata describes the display on which the content was mastered.
    /// It is embedded in both the AV1 bitstream and the AVIF container.
    #[inline(always)]
    #[must_use]
    pub fn with_mastering_display(mut self, md: MasteringDisplay) -> Self {
        self.mastering_display = Some(md);
        self
    }

    /// Set HDR content light level metadata (CEA-861.3).
    ///
    /// Describes the maximum and average light levels of the content.
    /// It is embedded in both the AV1 bitstream and the AVIF container.
    #[inline(always)]
    #[must_use]
    pub fn with_content_light(mut self, cl: ContentLight) -> Self {
        self.content_light = Some(cl);
        self
    }

    /// Set image rotation in the AVIF container.
    ///
    /// Angle is counter-clockwise in degrees: 0, 90, 180, or 270.
    #[inline(always)]
    #[must_use]
    pub fn with_rotation(mut self, angle: u8) -> Self {
        self.rotation = Some(angle);
        self
    }

    /// Set image mirror axis in the AVIF container.
    ///
    /// `0` = vertical axis (left-right flip), `1` = horizontal axis (top-bottom flip).
    #[inline(always)]
    #[must_use]
    pub fn with_mirror(mut self, axis: u8) -> Self {
        self.mirror = Some(axis);
        self
    }

    /// Set ICC color profile to embed in the AVIF container.
    #[inline(always)]
    #[must_use]
    pub fn with_icc_profile(mut self, profile: Vec<u8>) -> Self {
        self.icc_profile = Some(profile);
        self
    }

    /// Set XMP metadata to embed in the AVIF container.
    #[inline(always)]
    #[must_use]
    pub fn with_xmp(mut self, xmp: Vec<u8>) -> Self {
        self.xmp = Some(xmp);
        self
    }

    /// Embed a pre-encoded gain map for UltraHDR / ISO 21496-1.
    ///
    /// The gain map enables SDR/HDR tone mapping: the primary image is the SDR
    /// base, and the gain map allows reconstruction of the HDR rendition.
    ///
    /// * `gain_map` - Pre-encoded AV1 gain map data with ISO 21496-1 metadata.
    #[inline(always)]
    #[must_use]
    pub fn with_gain_map(mut self, gain_map: GainMapData) -> Self {
        self.gain_map = Some(gain_map);
        self
    }

    /// Enable/disable AV1 quantization matrices (imazen/rav1e fork).
    ///
    /// QM applies frequency-dependent quantization weights based on contrast
    /// sensitivity, giving ~10% BD-rate improvement for photographic content.
    /// Default: enabled.
    #[cfg(feature = "imazen")]
    #[inline(always)]
    #[must_use]
    pub fn with_qm(mut self, enable: bool) -> Self {
        self.enable_qm = enable;
        self
    }

    /// Enable/disable variance adaptive quantization (imazen/rav1e fork).
    ///
    /// VAQ allocates more bits to smooth regions where artifacts are visible
    /// and fewer bits to textured regions. Default: disabled; strength 1.0 when enabled.
    #[cfg(feature = "imazen")]
    #[inline(always)]
    #[must_use]
    pub fn with_vaq(mut self, enable: bool, strength: f64) -> Self {
        self.enable_vaq = enable;
        self.vaq_strength = strength;
        self
    }

    /// Enable/disable still-image tuning (imazen/rav1e fork).
    ///
    /// Uses `Tune::StillImage` which applies perceptual distortion metric with
    /// activity masking plus reduced CDEF/deblock for detail preservation.
    /// Default: disabled.
    #[cfg(feature = "imazen")]
    #[inline(always)]
    #[must_use]
    pub fn with_still_image_tuning(mut self, enable: bool) -> Self {
        self.tune_still_image = enable;
        self
    }

    /// Enable/disable mathematically lossless encoding (imazen/rav1e fork).
    ///
    /// Sets quantizer to 0 for lossless output. Default: disabled.
    #[cfg(feature = "imazen")]
    #[inline(always)]
    #[must_use]
    pub fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = lossless;
        self
    }

    /// Set segmentation boost power (1.0 = off, >1.0 = wider QP deltas).
    /// Amplifies the dynamic range of segmentation independently of RDO.
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_seg_boost(mut self, boost: f64) -> Self {
        self.seg_boost = boost;
        self
    }

    /// Override CDEF enable/disable (None = use speed preset default).
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_cdef(mut self, enable: Option<bool>) -> Self {
        self.override_cdef = enable;
        self
    }

    /// Override rdo_tx_decision enable/disable (None = use speed preset default).
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_rdo_tx_decision(mut self, enable: Option<bool>) -> Self {
        self.override_rdo_tx_decision = enable;
        self
    }

    /// Override SGR complexity to Full (all 16 parameter sets).
    /// At speed ≥5, rav1e uses Reduced (8 sets). Full searches all 16.
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_sgr_full(mut self, enable: Option<bool>) -> Self {
        self.override_sgr_complexity = enable;
        self
    }

    /// Override LRU on skip (search loop restoration on blocks with no coefficients).
    /// Off by default at speed ≥1.
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_lru_on_skip(mut self, enable: Option<bool>) -> Self {
        self.override_lru_on_skip = enable;
        self
    }

    /// Override segmentation to Complex (k-means, vs Simple at speed ≥3).
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_segmentation_complex(mut self, enable: Option<bool>) -> Self {
        self.override_segmentation_complex = enable;
        self
    }

    /// Override bottom-up partition search (off at speed ≥4).
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_encode_bottomup(mut self, enable: Option<bool>) -> Self {
        self.override_encode_bottomup = enable;
        self
    }

    /// Apply expert-only [`crate::expert::InternalParams`].
    ///
    /// **Unstable surface** — may change in any patch release; see
    /// [`crate::expert`] module docs for the contract. Each `Some(_)`
    /// field overrides a speed-preset default; each `None` leaves the
    /// preset's value untouched. Calling this multiple times overwrites
    /// previously-set fields wholesale (the struct is the unit of
    /// configuration, not the individual fields).
    #[cfg(feature = "__expert")]
    #[must_use]
    pub fn with_internal_params(mut self, params: crate::expert::InternalParams) -> Self {
        self.override_partition_range = params.partition_range;
        self.override_complex_prediction_modes = params.complex_prediction_modes;
        self.override_lrf = params.lrf;
        self.override_fast_deblock = params.fast_deblock;
        self.override_sb_q_scale = params.sb_q_scale;
        self.override_rdo_tx_size = params.rdo_tx_size_override;
        self.override_rdo_tx_size_depth = params.rdo_tx_size_depth;
        self.override_rdo_tx_type = params.rdo_tx_type_override;
        self.override_reduced_tx_set = params.reduced_tx_set;
        self.override_num_modes_rdo = params.num_modes_rdo_override;
        self.override_non_square_max_px = params.non_square_partition_max_threshold;
        // The prune quartet overrides as a UNIT (a None inside the unit
        // clears that gate) — see expert::InternalParams::prune_none_breakout.
        self.override_prune = if params.prune_none_breakout.is_some()
            || params.prune_rect_margin.is_some()
            || params.prune_four_way_margin.is_some()
            || params.prune_homogeneity_gate.is_some()
        {
            Some((
                params.prune_none_breakout,
                params.prune_rect_margin,
                params.prune_four_way_margin,
                params.prune_homogeneity_gate,
            ))
        } else {
            None
        };
        self
    }

    /// Enable trellis quantization (Viterbi DP coefficient optimization).
    /// Optimizes coefficient levels jointly using rate-distortion cost.
    #[cfg(feature = "imazen")]
    #[must_use]
    pub fn with_trellis(mut self, enable: bool) -> Self {
        self.enable_trellis = enable;
        self
    }

    /// Screen-content palette mode for the color/gray streams (the alpha
    /// stream never receives it). `PaletteMode::Off` is zenrav1e's default;
    /// `Auto` ports libaom's screen-content detection; `Always` forces the
    /// tool on. Content-gating belongs to the caller (zenavif's
    /// `palette_gate` drives this from `patch_fraction`).
    #[must_use]
    pub fn with_palette(mut self, mode: PaletteMode) -> Self {
        self.palette_mode = Some(mode);
        self
    }

    /// Validate the configuration without encoding.
    ///
    /// Returns `Err(ValidationError)` for the **first** failure found, in
    /// roughly the order parameters were configured. Returns `Ok(())` when
    /// every parameter is in its accepted range and no cross-parameter
    /// invariant is violated.
    ///
    /// The existing `encode_*` methods clamp / mask out-of-range values
    /// silently. Use `validate()` before encoding when you need fail-fast
    /// behaviour for batch jobs that should not spend compute on
    /// configurations that won't be respected anyway.
    ///
    /// # Example
    ///
    /// ```
    /// use zenravif::Encoder;
    /// let enc = Encoder::new().with_quality(150.0); // clamped at encode time
    /// assert!(enc.validate().is_err());
    /// ```
    // `validate()` is a config-shape check, not an encode call, so it keeps
    // returning the bare `ValidationError` (the crate's `Result` alias is the
    // `At<Error>` encode result; spell out `core::result::Result` here).
    pub fn validate(&self) -> core::result::Result<(), crate::ValidationError> {
        use crate::validate as v;
        use crate::ValidationError as E;

        if let Some(q) = self.quality_input
            && !v::QUALITY_RANGE.contains(&q)
        {
            return Err(E::QualityOutOfRange {
                value: q,
                valid: v::QUALITY_RANGE,
            });
        }
        if let Some(q) = self.alpha_quality_input
            && !v::QUALITY_RANGE.contains(&q)
        {
            return Err(E::AlphaQualityOutOfRange {
                value: q,
                valid: v::QUALITY_RANGE,
            });
        }
        if let Some(q) = self.libavif_quality_input
            && !v::QUALITY_RANGE.contains(&q)
        {
            return Err(E::LibavifQualityOutOfRange {
                value: q,
                valid: v::QUALITY_RANGE,
            });
        }
        if !v::SPEED_RANGE.contains(&self.speed) {
            return Err(E::SpeedOutOfRange {
                value: self.speed,
                valid: v::SPEED_RANGE,
            });
        }
        if let Some(0) = self.threads {
            return Err(E::NumThreadsZero);
        }
        if let Some(angle) = self.rotation
            && !v::ROTATION_RANGE.contains(&angle)
        {
            return Err(E::RotationOutOfRange {
                value: angle,
                valid: v::ROTATION_RANGE,
            });
        }
        if let Some(axis) = self.mirror
            && !v::MIRROR_RANGE.contains(&axis)
        {
            return Err(E::MirrorOutOfRange {
                value: axis,
                valid: v::MIRROR_RANGE,
            });
        }

        // Cross-parameter: 4:2:0 chroma subsampling is rejected at encode
        // time when combined with the RGB internal color model
        // (encode_raw_planes_internal: `Error::Unsupported(..)`).
        if matches!(self.chroma_subsampling, ChromaSubsampling::Yuv420)
            && matches!(self.color_model, ColorModel::RGB)
        {
            return Err(E::MutuallyExclusive {
                a: "chroma_subsampling=Yuv420",
                b: "color_model=RGB",
            });
        }

        #[cfg(feature = "imazen")]
        {
            // VAQ strength is clamped to 0.0..=4.0 in zenrav1e
            // (encoder.rs apply_vaq_strength). Only meaningful when VAQ is
            // enabled, but we still validate the stored value so that
            // `with_vaq(false, weird_value)` is also rejected.
            if !v::VAQ_STRENGTH_RANGE.contains(&self.vaq_strength) {
                return Err(E::VaqStrengthOutOfRange {
                    value: self.vaq_strength,
                    valid: v::VAQ_STRENGTH_RANGE,
                });
            }
            // seg_boost: 1.0 is the documented no-op; outside that we
            // require zenrav1e's clamp range (encoder.rs:913).
            if (self.seg_boost - 1.0).abs() > f64::EPSILON
                && !v::SEG_BOOST_RANGE.contains(&self.seg_boost)
            {
                return Err(E::SegBoostOutOfRange {
                    value: self.seg_boost,
                    valid: v::SEG_BOOST_RANGE,
                });
            }
            if let Some((min, max)) = self.override_partition_range
                && (!v::is_valid_partition_size(min)
                    || !v::is_valid_partition_size(max)
                    || min > max)
            {
                return Err(E::PartitionRangeInvalid { min, max });
            }
        }

        Ok(())
    }
}

/// Once done with config, call one of the `encode_*` functions
impl Encoder<'_> {
    /// Pre-flight dimension guard. Rejects `width * height` greater than
    /// the configured [`Encoder::with_max_pixels`] limit **before** any
    /// plane allocation or rav1e context construction. A limit of `0`
    /// means unlimited (the cap is disabled). Uses a saturating multiply
    /// so an overflowing `width * height` cannot wrap past the limit.
    #[inline]
    pub(crate) fn check_pixel_limit(&self, width: usize, height: usize) -> Result<()> {
        if self.max_pixels == 0 {
            return Ok(());
        }
        let pixels = (width as u64).saturating_mul(height as u64);
        if pixels > self.max_pixels {
            // Pre-flight rejection: trace the origin so server logs show the
            // exact dimensions and the call site that rejected them.
            return Err(at!(Error::TooManyPixels {
                width,
                height,
                max_pixels: self.max_pixels,
            }));
        }
        Ok(())
    }

    /// Make a new AVIF image from RGBA pixels (non-premultiplied, alpha last)
    ///
    /// Make the `Img` for the `buffer` like this:
    ///
    /// ```rust,ignore
    /// Img::new(&pixels_rgba[..], width, height)
    /// ```
    ///
    /// If you have pixels as `u8` slice, then use the `rgb` crate, and do:
    ///
    /// ```rust,ignore
    /// use rgb::ComponentSlice;
    /// let pixels_rgba = pixels_u8.as_rgba();
    /// ```
    ///
    /// If all pixels are opaque, the alpha channel will be left out automatically.
    ///
    /// This function takes 8-bit inputs, but will generate an AVIF file using 10-bit depth.
    ///
    /// returns AVIF file with info about sizes about AV1 payload.
    pub fn encode_rgba(&self, in_buffer: Img<&[rgb::RGBA<u8>]>) -> Result<EncodedImage> {
        // Pre-flight: reject oversized dimensions before any pixel work
        // (convert_alpha_8bit below scans the whole buffer).
        self.check_pixel_limit(in_buffer.width(), in_buffer.height()).at()?;
        let new_alpha = self.convert_alpha_8bit(in_buffer);
        let buffer = new_alpha.as_ref().map(|b| b.as_ref()).unwrap_or(in_buffer);
        let use_alpha = buffer.pixels().any(|px| px.a != 255);
        if !use_alpha {
            return self.encode_rgb_internal_from_8bit(buffer.width(), buffer.height(), buffer.pixels().map(|px| px.rgb())).at();
        }

        let width = buffer.width();
        let height = buffer.height();
        let matrix_coefficients = match self.color_model {
            ColorModel::YCbCr => MatrixCoefficients::BT601,
            ColorModel::RGB => MatrixCoefficients::Identity,
        };
        let pixel_range = self.pixel_range.unwrap_or(PixelRange::Full);
        match self.output_depth {
            BitDepth::Eight => {
                let planes = buffer.pixels().map(|px| match self.color_model {
                    ColorModel::YCbCr => rgb_to_8_bit_ycbcr(px.rgb(), BT601).into(),
                    ColorModel::RGB => rgb_to_8_bit_gbr(px.rgb()).into(),
                });
                let alpha = buffer.pixels().map(|px| px.a);
                self.encode_raw_planes_8_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients).at()
            },
            BitDepth::Ten | BitDepth::Auto => {
                let planes = buffer.pixels().map(|px| match self.color_model {
                    ColorModel::YCbCr => rgb_to_10_bit_ycbcr(px.rgb(), BT601).into(),
                    ColorModel::RGB => rgb_to_10_bit_gbr(px.rgb()).into(),
                });
                let alpha = buffer.pixels().map(|px| to_ten(px.a));
                self.encode_raw_planes_10_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients).at()
            },
            BitDepth::Twelve => {
                let planes = buffer.pixels().map(|px| match self.color_model {
                    ColorModel::YCbCr => rgb_to_12_bit_ycbcr(px.rgb(), BT601).into(),
                    ColorModel::RGB => rgb_to_12_bit_gbr(px.rgb()).into(),
                });
                let alpha = buffer.pixels().map(|px| to_twelve(px.a));
                self.encode_raw_planes_12_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients).at()
            },
        }
    }

    fn convert_alpha_8bit(&self, in_buffer: Img<&[RGBA8]>) -> Option<ImgVec<RGBA8>> {
        match self.alpha_color_mode {
            AlphaColorMode::UnassociatedDirty => None,
            AlphaColorMode::UnassociatedClean => blurred_dirty_alpha(in_buffer),
            AlphaColorMode::Premultiplied => {
                let prem = in_buffer.pixels()
                    .map(|px| {
                        if px.a == 0 || px.a == 255 {
                            RGBA8::default()
                        } else {
                            RGBA8::new(
                                (u16::from(px.r) * 255 / u16::from(px.a)) as u8,
                                (u16::from(px.g) * 255 / u16::from(px.a)) as u8,
                                (u16::from(px.b) * 255 / u16::from(px.a)) as u8,
                                px.a,
                            )
                        }
                    })
                    .collect();
                Some(ImgVec::new(prem, in_buffer.width(), in_buffer.height()))
            },
        }
    }

    /// Make a new AVIF image from RGB pixels
    ///
    /// Make the `Img` for the `buffer` like this:
    ///
    /// ```rust,ignore
    /// Img::new(&pixels_rgb[..], width, height)
    /// ```
    ///
    /// If you have pixels as `u8` slice, then first do:
    ///
    /// ```rust,ignore
    /// use rgb::ComponentSlice;
    /// let pixels_rgb = pixels_u8.as_rgb();
    /// ```
    ///
    /// returns AVIF file, size of color metadata
    #[inline]
    pub fn encode_rgb(&self, buffer: Img<&[RGB8]>) -> Result<EncodedImage> {
        // Pre-flight: reject oversized dimensions before any pixel work.
        self.check_pixel_limit(buffer.width(), buffer.height()).at()?;
        self.encode_rgb_internal_from_8bit(buffer.width(), buffer.height(), buffer.pixels()).at()
    }

    fn encode_rgb_internal_from_8bit(&self, width: usize, height: usize, pixels: impl Iterator<Item = RGB8> + Send + Sync) -> Result<EncodedImage> {
        let matrix_coefficients = match self.color_model {
            ColorModel::YCbCr => MatrixCoefficients::BT601,
            ColorModel::RGB => MatrixCoefficients::Identity,
        };

        let pixel_range = self.pixel_range.unwrap_or(PixelRange::Full);
        match self.output_depth {
            BitDepth::Eight => {
                let planes = pixels.map(|px| {
                    let (y, u, v) = match self.color_model {
                        ColorModel::YCbCr => rgb_to_8_bit_ycbcr(px, BT601),
                        ColorModel::RGB => rgb_to_8_bit_gbr(px),
                    };
                    [y, u, v]
                });
                self.encode_raw_planes_8_bit(width, height, planes, None::<[_; 0]>, pixel_range, matrix_coefficients)
            },
            BitDepth::Ten | BitDepth::Auto => {
                let planes = pixels.map(|px| {
                    let (y, u, v) = match self.color_model {
                        ColorModel::YCbCr => rgb_to_10_bit_ycbcr(px, BT601),
                        ColorModel::RGB => rgb_to_10_bit_gbr(px),
                    };
                    [y, u, v]
                });
                self.encode_raw_planes_10_bit(width, height, planes, None::<[_; 0]>, pixel_range, matrix_coefficients)
            },
            BitDepth::Twelve => {
                let planes = pixels.map(|px| {
                    let (y, u, v) = match self.color_model {
                        ColorModel::YCbCr => rgb_to_12_bit_ycbcr(px, BT601),
                        ColorModel::RGB => rgb_to_12_bit_gbr(px),
                    };
                    [y, u, v]
                });
                self.encode_raw_planes_12_bit(width, height, planes, None::<[_; 0]>, pixel_range, matrix_coefficients)
            },
        }
    }

    /// Encodes an 8-bit grayscale image as a true monochrome AVIF
    /// (AV1 `Cs400`: the bitstream codes only a luma plane — no chroma).
    ///
    /// `buffer` holds one `u8` luma sample per pixel (sRGB transfer).
    /// The image's `av1C`/`pixi` properties are written in monochrome form
    /// (seq_profile 0/2, 1 channel).
    ///
    /// Compared to expanding gray to RGB and calling [`Self::encode_rgb`],
    /// output bytes are at parity on typical content (neutral chroma is
    /// skip-coded by the RDO anyway), but encoding is measurably faster —
    /// about 2–3× — because chroma RDO is skipped entirely
    /// (imazen/zenavif#6, benchmarks/mono_encode_ab_2026-06-11.txt).
    ///
    /// [`Self::with_bit_depth`] is honored: `Eight` stays 8-bit,
    /// `Ten`/`Auto` widen to 10-bit, `Twelve` to 12-bit.
    /// Alpha-related settings are ignored (there is no alpha item).
    pub fn encode_gray8(&self, buffer: Img<&[u8]>) -> Result<EncodedImage> {
        // Pre-flight: reject oversized dimensions before any pixel work.
        self.check_pixel_limit(buffer.width(), buffer.height()).at()?;
        let (width, height) = (buffer.width(), buffer.height());
        match self.output_depth {
            BitDepth::Eight => {
                self.encode_gray_planes_internal::<u8>(width, height, buffer.pixels(), 8).at()
            },
            BitDepth::Ten | BitDepth::Auto => {
                self.encode_gray_planes_internal::<u16>(width, height, buffer.pixels().map(to_ten), 10).at()
            },
            BitDepth::Twelve => {
                self.encode_gray_planes_internal::<u16>(width, height, buffer.pixels().map(to_twelve), 12).at()
            },
        }
    }

    fn encode_gray_planes_internal<P: zenrav1e::Pixel + Default>(
        &self,
        width: usize,
        height: usize,
        planes: impl IntoIterator<Item = P> + Send,
        input_pixels_bit_depth: u8,
    ) -> Result<EncodedImage> {
        let color_pixel_range = self.pixel_range.unwrap_or(PixelRange::Full);

        // Monochrome has no chroma to describe; MC is signaled Unspecified
        // (Identity would additionally be rejected for non-4:4:4 layouts).
        let color_description = Some(ColorDescription {
            transfer_characteristics: self.transfer_characteristics
                .unwrap_or(TransferCharacteristics::SRGB),
            color_primaries: self.color_primaries
                .unwrap_or(ColorPrimaries::BT709),
            matrix_coefficients: MatrixCoefficients::Unspecified,
        });

        let threads = self.threads.map(|threads| {
            if threads > 0 { threads } else { rayon::current_num_threads() }
        });
        let cancel_token = self.cancellation_token.as_ref();
        let deadline = self.timeout.map(|timeout| std::time::Instant::now() + timeout);

        #[cfg_attr(not(feature = "imazen"), allow(unused_mut))]
        let mut speed = SpeedTweaks::from_my_preset(self.speed, self.quantizer, width.max(height));
        #[cfg(feature = "imazen")]
        {
            if let Some(v) = self.override_cdef { speed.cdef = Some(v); }
            if let Some(v) = self.override_rdo_tx_decision { speed.rdo_tx_decision = Some(v); }
            if let Some(v) = self.override_sgr_complexity { speed.sgr_complexity_full = Some(v); }
            if let Some(v) = self.override_lru_on_skip { speed.lru_on_skip = Some(v); }
            if let Some(v) = self.override_segmentation_complex {
                speed.segmentation = Some(if v { SegmentationLevel::Complex } else { SegmentationLevel::Simple });
            }
            if let Some(v) = self.override_encode_bottomup { speed.encode_bottomup = Some(v); }
            if let Some(r) = self.override_partition_range { speed.partition_range = Some(r); }
            if let Some(v) = self.override_complex_prediction_modes {
                speed.complex_prediction_modes = Some(v);
            }
            if let Some(v) = self.override_lrf { speed.lrf = Some(v); }
            if let Some(v) = self.override_fast_deblock { speed.fast_deblock = Some(v); }
            apply_fast_tier_overrides(
                &mut speed,
                self.override_rdo_tx_size,
                self.override_rdo_tx_size_depth,
                self.override_rdo_tx_type,
                self.override_reduced_tx_set,
                self.override_num_modes_rdo,
                self.override_non_square_max_px,
                self.override_prune,
            );
        }
        if self.palette_mode.is_some() {
            speed.palette = self.palette_mode;
        }

        let color = encode_to_av1::<P>(
            &Av1EncodeConfig {
                width,
                height,
                bit_depth: input_pixels_bit_depth.into(),
                quantizer: self.quantizer.into(),
                speed,
                threads,
                pixel_range: color_pixel_range,
                chroma_sampling: ChromaSampling::Cs400,
                color_description,
                mastering_display: self.mastering_display,
                content_light: self.content_light,
                #[cfg(feature = "imazen")]
                enable_qm: self.enable_qm,
                #[cfg(feature = "imazen")]
                enable_vaq: self.enable_vaq,
                #[cfg(feature = "imazen")]
                vaq_strength: self.vaq_strength,
                #[cfg(feature = "imazen")]
                tune_still_image: self.tune_still_image,
                #[cfg(feature = "imazen")]
                lossless: self.lossless,
                #[cfg(feature = "imazen")]
                seg_boost: self.seg_boost,
                #[cfg(feature = "imazen")]
                override_cdef: self.override_cdef,
                #[cfg(feature = "imazen")]
                override_rdo_tx_decision: self.override_rdo_tx_decision,
                #[cfg(feature = "imazen")]
                enable_trellis: self.enable_trellis,
                // The gray plane IS the primary luma plane, so the
                // keyframe-scoped per-SB delta-q hints apply to it exactly
                // as they do to the color path's luma.
                #[cfg(feature = "imazen")]
                frame_hints_sb_q_scale: self.override_sb_q_scale.clone(),
                is_alpha: false,
                max_pixels: self.max_pixels,
                #[cfg(feature = "stop")]
                stop_token: self.stop_token.clone(),
            },
            cancel_token,
            deadline,
            |frame| init_frame_1(width, height, planes, frame, cancel_token, deadline),
        ).at()?;

        let mut serializer_config = zenavif_serialize::Aviffy::new();
        serializer_config
            .set_monochrome(true)
            .matrix_coefficients(zenavif_serialize::constants::MatrixCoefficients::Unspecified);

        let tc = self.transfer_characteristics.unwrap_or(TransferCharacteristics::SRGB);
        serializer_config.set_transfer_characteristics(map_transfer_characteristics(tc));
        let cp = self.color_primaries.unwrap_or(ColorPrimaries::BT709);
        serializer_config.set_color_primaries(map_color_primaries(cp));
        serializer_config.set_full_color_range(color_pixel_range == PixelRange::Full);

        if let Some(exif) = &self.exif {
            serializer_config.set_exif(exif.to_vec());
        }
        if let Some(md) = self.mastering_display {
            serializer_config.set_mastering_display(
                [(md.primaries[0].x, md.primaries[0].y),
                 (md.primaries[1].x, md.primaries[1].y),
                 (md.primaries[2].x, md.primaries[2].y)],
                (md.white_point.x, md.white_point.y),
                md.max_luminance, md.min_luminance,
            );
        }
        if let Some(cl) = self.content_light {
            serializer_config.set_content_light_level(
                cl.max_content_light_level, cl.max_frame_average_light_level,
            );
        }
        if let Some(angle) = self.rotation {
            serializer_config.set_rotation(angle);
        }
        if let Some(axis) = self.mirror {
            serializer_config.set_mirror(axis);
        }
        if let Some(ref icc) = self.icc_profile {
            serializer_config.set_icc_profile(icc.clone());
        }
        if let Some(ref xmp) = self.xmp {
            serializer_config.set_xmp(xmp.clone());
        }
        if let Some(ref gm) = self.gain_map {
            serializer_config.set_gain_map(
                gm.av1_data.clone(),
                gm.width,
                gm.height,
                gm.bit_depth,
                gm.metadata.clone(),
            );
        }

        let avif_file = serializer_config.to_vec(&color, None, width as u32, height as u32, input_pixels_bit_depth);
        let color_byte_size = color.len();

        Ok(EncodedImage {
            avif_file, color_byte_size, alpha_byte_size: 0,
        })
    }

    /// Encodes AVIF from 3 planar channels that are in the color space described by `matrix_coefficients`,
    /// with sRGB transfer characteristics and color primaries.
    ///
    /// Alpha always uses full range. Chroma subsampling is not supported, and it's a bad idea for AVIF anyway.
    /// If there's no alpha, use `None::<[_; 0]>`.
    ///
    /// `color_pixel_range` should be `PixelRange::Full` to avoid worsening already small 8-bit dynamic range.
    /// Support for limited range may be removed in the future.
    ///
    /// If `AlphaColorMode::Premultiplied` has been set, the alpha pixels must be premultiplied.
    /// `AlphaColorMode::UnassociatedClean` has no effect in this function, and is equivalent to `AlphaColorMode::UnassociatedDirty`.
    ///
    /// returns AVIF file, size of color metadata, size of alpha metadata overhead
    #[inline]
    pub fn encode_raw_planes_8_bit(
        &self, width: usize, height: usize,
        planes: impl IntoIterator<Item = [u8; 3]> + Send,
        alpha: Option<impl IntoIterator<Item = u8> + Send>,
        color_pixel_range: PixelRange, matrix_coefficients: MatrixCoefficients,
    ) -> Result<EncodedImage> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 8).at()
    }

    /// Encodes AVIF from 3 planar channels that are in the color space described by `matrix_coefficients`,
    /// with sRGB transfer characteristics and color primaries.
    ///
    /// The pixels are 10-bit (values `0..=1023`) in host's native endian.
    ///
    /// Alpha always uses full range. Chroma subsampling is not supported, and it's a bad idea for AVIF anyway.
    /// If there's no alpha, use `None::<[_; 0]>`.
    ///
    /// `color_pixel_range` should be `PixelRange::Full`. Support for limited range may be removed in the future.
    ///
    /// If `AlphaColorMode::Premultiplied` has been set, the alpha pixels must be premultiplied.
    /// `AlphaColorMode::UnassociatedClean` has no effect in this function, and is equivalent to `AlphaColorMode::UnassociatedDirty`.
    ///
    /// returns AVIF file, size of color metadata, size of alpha metadata overhead
    #[inline]
    pub fn encode_raw_planes_10_bit(
        &self, width: usize, height: usize,
        planes: impl IntoIterator<Item = [u16; 3]> + Send,
        alpha: Option<impl IntoIterator<Item = u16> + Send>,
        color_pixel_range: PixelRange, matrix_coefficients: MatrixCoefficients,
    ) -> Result<EncodedImage> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 10).at()
    }

    /// Encodes AVIF from 3 planar channels that are in the color space described by `matrix_coefficients`.
    ///
    /// The pixels are 12-bit (values `0..=4095`) in host's native endian.
    /// 12-bit depth is useful for HDR content with PQ or HLG transfer characteristics.
    ///
    /// Alpha always uses full range. If there's no alpha, use `None::<[_; 0]>`.
    ///
    /// returns AVIF file, size of color metadata, size of alpha metadata overhead
    #[inline]
    pub fn encode_raw_planes_12_bit(
        &self, width: usize, height: usize,
        planes: impl IntoIterator<Item = [u16; 3]> + Send,
        alpha: Option<impl IntoIterator<Item = u16> + Send>,
        color_pixel_range: PixelRange, matrix_coefficients: MatrixCoefficients,
    ) -> Result<EncodedImage> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 12).at()
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn encode_raw_planes_internal<P: zenrav1e::Pixel + Default>(
        &self, width: usize, height: usize,
        planes: impl IntoIterator<Item = [P; 3]> + Send,
        alpha: Option<impl IntoIterator<Item = P> + Send>,
        color_pixel_range: PixelRange, matrix_coefficients: MatrixCoefficients,
        input_pixels_bit_depth: u8,
    ) -> Result<EncodedImage> {
        // Pre-flight: the single shared chokepoint for every still encode
        // path (encode_rgba / encode_rgb / encode_raw_planes_{8,10,12}_bit).
        // Rejecting oversized dimensions here — before the rav1e context is
        // built — guarantees no public entry point can bypass the cap.
        self.check_pixel_limit(width, height).at()?;
        if self.chroma_subsampling == ChromaSubsampling::Yuv420 && matrix_coefficients == MatrixCoefficients::Identity {
            return Err(at!(Error::Unsupported("4:2:0 chroma subsampling with RGB color model")));
        }

        let color_description = Some(ColorDescription {
            transfer_characteristics: self.transfer_characteristics
                .unwrap_or(TransferCharacteristics::SRGB),
            color_primaries: self.color_primaries
                .unwrap_or(ColorPrimaries::BT709),
            matrix_coefficients,
        });

        let threads = self.threads.map(|threads| {
            if threads > 0 { threads } else { rayon::current_num_threads() }
        });

        let cancel_token = self.cancellation_token.as_ref();
        let cancel_token_alpha = self.cancellation_token.as_ref();

        // Calculate deadline from timeout if set
        let deadline = self.timeout.map(|timeout| std::time::Instant::now() + timeout);

        let chroma_sampling = match self.chroma_subsampling {
            ChromaSubsampling::Yuv444 => ChromaSampling::Cs444,
            ChromaSubsampling::Yuv420 => ChromaSampling::Cs420,
        };

        let use_420 = self.chroma_subsampling == ChromaSubsampling::Yuv420;
        let mastering_display = self.mastering_display;
        let content_light = self.content_light;
        #[cfg(feature = "imazen")]
        let override_cdef = self.override_cdef;
        #[cfg(feature = "imazen")]
        let override_rdo_tx_decision = self.override_rdo_tx_decision;
        #[cfg(feature = "imazen")]
        let override_sgr_complexity = self.override_sgr_complexity;
        #[cfg(feature = "imazen")]
        let override_lru_on_skip = self.override_lru_on_skip;
        #[cfg(feature = "imazen")]
        let override_segmentation_complex = self.override_segmentation_complex;
        #[cfg(feature = "imazen")]
        let override_encode_bottomup = self.override_encode_bottomup;
        #[cfg(feature = "imazen")]
        let override_partition_range = self.override_partition_range;
        #[cfg(feature = "imazen")]
        let override_complex_prediction_modes = self.override_complex_prediction_modes;
        #[cfg(feature = "imazen")]
        let override_lrf = self.override_lrf;
        #[cfg(feature = "imazen")]
        let override_fast_deblock = self.override_fast_deblock;
        #[cfg(feature = "imazen")]
        let override_sb_q_scale = self.override_sb_q_scale.clone();
        #[cfg(feature = "imazen")]
        let fast_tier_overrides = (
            self.override_rdo_tx_size,
            self.override_rdo_tx_size_depth,
            self.override_rdo_tx_type,
            self.override_reduced_tx_set,
            self.override_num_modes_rdo,
            self.override_non_square_max_px,
            self.override_prune,
        );
        let palette_mode = self.palette_mode;
        let encode_color = move || {
            #[cfg_attr(not(feature = "imazen"), allow(unused_mut))]
            let mut speed = SpeedTweaks::from_my_preset(self.speed, self.quantizer, width.max(height));
            #[cfg(feature = "imazen")]
            {
                if let Some(v) = override_cdef { speed.cdef = Some(v); }
                if let Some(v) = override_rdo_tx_decision { speed.rdo_tx_decision = Some(v); }
                if let Some(v) = override_sgr_complexity { speed.sgr_complexity_full = Some(v); }
                if let Some(v) = override_lru_on_skip { speed.lru_on_skip = Some(v); }
                if let Some(v) = override_segmentation_complex {
                    speed.segmentation = Some(if v { SegmentationLevel::Complex } else { SegmentationLevel::Simple });
                }
                if let Some(v) = override_encode_bottomup { speed.encode_bottomup = Some(v); }
                if let Some(r) = override_partition_range { speed.partition_range = Some(r); }
                if let Some(v) = override_complex_prediction_modes {
                    speed.complex_prediction_modes = Some(v);
                }
                if let Some(v) = override_lrf { speed.lrf = Some(v); }
                if let Some(v) = override_fast_deblock { speed.fast_deblock = Some(v); }
                let (ts, tsd, tt, rts, nm, nsq, prune) = fast_tier_overrides;
                apply_fast_tier_overrides(
                    &mut speed, ts, tsd, tt, rts, nm, nsq, prune,
                );
            }
            if palette_mode.is_some() {
                speed.palette = palette_mode;
            }
            encode_to_av1::<P>(
                &Av1EncodeConfig {
                    width,
                    height,
                    bit_depth: input_pixels_bit_depth.into(),
                    quantizer: self.quantizer.into(),
                    speed,
                    threads,
                    pixel_range: color_pixel_range,
                    chroma_sampling,
                    color_description,
                    mastering_display,
                    content_light,
                    #[cfg(feature = "imazen")]
                    enable_qm: self.enable_qm,
                    #[cfg(feature = "imazen")]
                    enable_vaq: self.enable_vaq,
                    #[cfg(feature = "imazen")]
                    vaq_strength: self.vaq_strength,
                    #[cfg(feature = "imazen")]
                    tune_still_image: self.tune_still_image,
                    #[cfg(feature = "imazen")]
                    lossless: self.lossless,
                    #[cfg(feature = "imazen")]
                    seg_boost: self.seg_boost,
                    #[cfg(feature = "imazen")]
                    override_cdef,
                    #[cfg(feature = "imazen")]
                    override_rdo_tx_decision,
                    #[cfg(feature = "imazen")]
                    enable_trellis: self.enable_trellis,
                    #[cfg(feature = "imazen")]
                    frame_hints_sb_q_scale: override_sb_q_scale,
                    is_alpha: false,
                    max_pixels: self.max_pixels,
                    #[cfg(feature = "stop")]
                    stop_token: self.stop_token.clone(),
                },
                cancel_token,
                deadline,
                move |frame| {
                    if use_420 {
                        init_frame_3_420(width, height, planes, frame, cancel_token, deadline)
                    } else {
                        init_frame_3(width, height, planes, frame, cancel_token, deadline)
                    }
                },
            )
        };
        let encode_alpha = move || {
            alpha.map(|alpha| {
                encode_to_av1::<P>(
                    &Av1EncodeConfig {
                        width,
                        height,
                        bit_depth: input_pixels_bit_depth.into(),
                        quantizer: self.alpha_quantizer.into(),
                        speed: SpeedTweaks::from_my_preset(self.speed, self.alpha_quantizer, width.max(height)),
                        threads,
                        pixel_range: PixelRange::Full,
                        chroma_sampling: ChromaSampling::Cs400,
                        color_description: None,
                        mastering_display: None,
                        content_light: None,
                        #[cfg(feature = "imazen")]
                        enable_qm: false,
                        #[cfg(feature = "imazen")]
                        enable_vaq: false,
                        #[cfg(feature = "imazen")]
                        vaq_strength: 1.0,
                        #[cfg(feature = "imazen")]
                        tune_still_image: false,
                        #[cfg(feature = "imazen")]
                        lossless: self.lossless,
                        #[cfg(feature = "imazen")]
                        seg_boost: 1.0,
                        #[cfg(feature = "imazen")]
                        override_cdef: None,
                        #[cfg(feature = "imazen")]
                        override_rdo_tx_decision: None,
                        #[cfg(feature = "imazen")]
                        enable_trellis: false,
                        // Per-SB hints are a color-plane signal; the alpha
                        // encode never receives the map.
                        #[cfg(feature = "imazen")]
                        frame_hints_sb_q_scale: None,
                        is_alpha: true,
                        max_pixels: self.max_pixels,
                        #[cfg(feature = "stop")]
                        stop_token: self.stop_token.clone(),
                    },
                    cancel_token_alpha,
                    deadline,
                    |frame| init_frame_1(width, height, alpha, frame, cancel_token_alpha, deadline),
                )
            })
        };
        #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
        let (color, alpha) = (encode_color(), encode_alpha());
        #[cfg(not(all(target_arch = "wasm32", not(target_feature = "atomics"))))]
        let (color, alpha) = rayon::join(encode_color, encode_alpha);
        let (color, alpha) = (color.at()?, alpha.transpose().at()?);

        let mut serializer_config = zenavif_serialize::Aviffy::new();
        serializer_config
            .matrix_coefficients(match matrix_coefficients {
                MatrixCoefficients::Identity => zenavif_serialize::constants::MatrixCoefficients::Rgb,
                MatrixCoefficients::BT709 => zenavif_serialize::constants::MatrixCoefficients::Bt709,
                MatrixCoefficients::Unspecified => zenavif_serialize::constants::MatrixCoefficients::Unspecified,
                MatrixCoefficients::BT601 => zenavif_serialize::constants::MatrixCoefficients::Bt601,
                MatrixCoefficients::YCgCo => zenavif_serialize::constants::MatrixCoefficients::Ycgco,
                MatrixCoefficients::BT2020NCL => zenavif_serialize::constants::MatrixCoefficients::Bt2020Ncl,
                MatrixCoefficients::BT2020CL => zenavif_serialize::constants::MatrixCoefficients::Bt2020Cl,
                _ => return Err(at!(Error::Unsupported("matrix coefficients"))),
            })
            .premultiplied_alpha(self.premultiplied_alpha);

        let tc = self.transfer_characteristics.unwrap_or(TransferCharacteristics::SRGB);
        serializer_config.set_transfer_characteristics(map_transfer_characteristics(tc));

        let cp = self.color_primaries.unwrap_or(ColorPrimaries::BT709);
        serializer_config.set_color_primaries(map_color_primaries(cp));

        let pixel_range = self.pixel_range.unwrap_or(PixelRange::Full);
        serializer_config.set_full_color_range(pixel_range == PixelRange::Full);

        if self.chroma_subsampling == ChromaSubsampling::Yuv420 {
            serializer_config.set_chroma_subsampling((true, true));
            serializer_config.set_seq_profile(0); // Main profile for 4:2:0
        }
        if let Some(exif) = &self.exif {
            serializer_config.set_exif(exif.to_vec());
        }
        if let Some(md) = self.mastering_display {
            serializer_config.set_mastering_display(
                [(md.primaries[0].x, md.primaries[0].y),
                 (md.primaries[1].x, md.primaries[1].y),
                 (md.primaries[2].x, md.primaries[2].y)],
                (md.white_point.x, md.white_point.y),
                md.max_luminance, md.min_luminance,
            );
        }
        if let Some(cl) = self.content_light {
            serializer_config.set_content_light_level(
                cl.max_content_light_level, cl.max_frame_average_light_level,
            );
        }
        if let Some(angle) = self.rotation {
            serializer_config.set_rotation(angle);
        }
        if let Some(axis) = self.mirror {
            serializer_config.set_mirror(axis);
        }
        if let Some(ref icc) = self.icc_profile {
            serializer_config.set_icc_profile(icc.clone());
        }
        if let Some(ref xmp) = self.xmp {
            serializer_config.set_xmp(xmp.clone());
        }
        if let Some(ref gm) = self.gain_map {
            serializer_config.set_gain_map(
                gm.av1_data.clone(),
                gm.width,
                gm.height,
                gm.bit_depth,
                gm.metadata.clone(),
            );
            // The gain-map item's av1C must describe its actual payload.
            serializer_config.set_gain_map_chroma_subsampling(
                zenavif_serialize::ChromaSubsampling {
                    horizontal: gm.chroma_subsampling.0,
                    vertical: gm.chroma_subsampling.1,
                },
            );
            serializer_config.set_gain_map_monochrome(gm.monochrome);
            if let Some((p, t, m, full)) = gm.alt_colr_cicp {
                serializer_config.set_gain_map_alt_colr(gain_map_alt_colr_box(p, t, m, full)?);
            }
            if let Some(ref icc) = gm.alt_icc {
                serializer_config.set_gain_map_alt_icc(icc.clone());
            }
        }
        let avif_file = serializer_config.to_vec(&color, alpha.as_deref(), width as u32, height as u32, input_pixels_bit_depth);
        let color_byte_size = color.len();
        let alpha_byte_size = alpha.as_ref().map_or(0, |a| a.len());

        Ok(EncodedImage {
            avif_file, color_byte_size, alpha_byte_size,
        })
    }
}

/// Native endian
#[inline(always)]
fn to_ten(x: u8) -> u16 {
    (u16::from(x) << 2) | (u16::from(x) >> 6)
}

/// Native endian
#[inline(always)]
fn rgb_to_10_bit_gbr(px: rgb::RGB<u8>) -> (u16, u16, u16) {
    (to_ten(px.g), to_ten(px.b), to_ten(px.r))
}

/// Scale 8-bit to 12-bit: [0,255] → [0,4095]
#[inline(always)]
fn to_twelve(x: u8) -> u16 {
    (u16::from(x) << 4) | (u16::from(x) >> 4)
}

/// Native endian
#[inline(always)]
fn rgb_to_12_bit_gbr(px: rgb::RGB<u8>) -> (u16, u16, u16) {
    (to_twelve(px.g), to_twelve(px.b), to_twelve(px.r))
}

#[inline(always)]
fn rgb_to_8_bit_gbr(px: rgb::RGB<u8>) -> (u8, u8, u8) {
    (px.g, px.b, px.r)
}

// const REC709: [f32; 3] = [0.2126, 0.7152, 0.0722];
const BT601: [f32; 3] = [0.2990, 0.5870, 0.1140];

#[inline(always)]
fn rgb_to_ycbcr(px: rgb::RGB<u8>, depth: u8, matrix: [f32; 3]) -> (f32, f32, f32) {
    let max_value = ((1 << depth) - 1) as f32;
    let scale = max_value / 255.;
    let shift = (max_value * 0.5).round();
    let y = (scale * matrix[2]).mul_add(f32::from(px.b), (scale * matrix[0]).mul_add(f32::from(px.r), scale * matrix[1] * f32::from(px.g)));
    let cb = f32::from(px.b).mul_add(scale, -y).mul_add(0.5 / (1. - matrix[2]), shift);
    let cr = f32::from(px.r).mul_add(scale, -y).mul_add(0.5 / (1. - matrix[0]), shift);
    (y.round(), cb.round(), cr.round())
}

#[inline(always)]
fn rgb_to_10_bit_ycbcr(px: rgb::RGB<u8>, matrix: [f32; 3]) -> (u16, u16, u16) {
    let (y, u, v) = rgb_to_ycbcr(px, 10, matrix);
    (y as u16, u as u16, v as u16)
}

#[inline(always)]
fn rgb_to_12_bit_ycbcr(px: rgb::RGB<u8>, matrix: [f32; 3]) -> (u16, u16, u16) {
    let (y, u, v) = rgb_to_ycbcr(px, 12, matrix);
    (y as u16, u as u16, v as u16)
}

#[inline(always)]
fn rgb_to_8_bit_ycbcr(px: rgb::RGB<u8>, matrix: [f32; 3]) -> (u8, u8, u8) {
    let (y, u, v) = rgb_to_ycbcr(px, 8, matrix);
    (y as u8, u as u8, v as u8)
}

pub(crate) fn quality_to_quantizer(quality: f32) -> u8 {
    let q = quality.clamp(1., 100.) / 100.;
    let x = if q >= 0.70 {
        (1. - q) * 1.4          // Q70-100 → qindex 0-107
    } else if q > 0.10 {
        0.42 + (0.70 - q) * 0.85  // Q10-70 → qindex 107-237
    } else {
        0.93 + (0.10 - q) * 0.78  // Q1-10 → qindex 237-255
    };
    (x.min(1.0) * 255.).round() as u8
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct SpeedTweaks {
    pub speed_preset: u8,

    pub fast_deblock: Option<bool>,
    pub reduced_tx_set: Option<bool>,
    pub tx_domain_distortion: Option<bool>,
    pub tx_domain_rate: Option<bool>,
    pub encode_bottomup: Option<bool>,
    pub rdo_tx_decision: Option<bool>,
    pub cdef: Option<bool>,
    /// loop restoration filter
    pub lrf: Option<bool>,
    pub sgr_complexity_full: Option<bool>,
    pub use_satd_subpel: Option<bool>,
    pub inter_tx_split: Option<bool>,
    pub fine_directional_intra: Option<bool>,
    pub complex_prediction_modes: Option<bool>,
    pub partition_range: Option<(u8, u8)>,
    pub segmentation: Option<SegmentationLevel>,
    pub lru_on_skip: Option<bool>,
    pub non_square_partition_max_threshold: Option<BlockSize>,
    /// Screen-content palette mode (zenrav1e `PaletteMode`, default `Off`).
    /// Set by the [`Encoder::with_palette`] pass-through — zenavif's
    /// `palette_gate` drives it (pf-threshold → Always/Auto). Color/gray
    /// streams only; the alpha stream never receives it.
    pub palette: Option<PaletteMode>,
    /// Offer AV1's mixed 3-way partition types (HORZ_A/B, VERT_A/B) in the
    /// RDO search. ~1.5x encode time for a small compression win — the s1
    /// "deep" mode ingredient (zenrav1e#27).
    // dead_code: not applied until the zenrav1e dep bump (the knob lands
    // post-0.1.4); the apply line in `speed_settings()` is commented until
    // then. Remove the allow when uncommenting.
    #[allow(dead_code)]
    pub mixed_3way_partitions: Option<bool>,
    /// Top-7 keyframe intra RDO via `ComplexKeyframes` +
    /// `filter_intra=Some(false)` (the zenrav1e#5-safe form) — the P2HEADS
    /// measured global fast-tier arm (see `S6_INTRA7_LIVE`).
    // dead_code: not applied until the zenrav1e dep bump (the filter_intra
    // override lands post-0.1.4); the apply block in `speed_settings()` is
    // commented until then. Remove the allow when uncommenting.
    #[allow(dead_code)]
    pub intra_top7: Option<bool>,
    /// Recursion depth of the topdown SPLIT-trial cost refinement (1 = the
    /// shipped one-level estimate; measured depth-2 verdict below at the
    /// `split_trial_depth` arm — zenrav1e#27).
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub split_trial_depth: Option<u8>,
    /// Decoupled intra tx-SIZE RDO (zenrav1e `rdo_tx_size_override`): keep
    /// the tx-size search alive at tiers whose `rdo_tx_decision` is off,
    /// without re-enabling the tx-TYPE search (DCT-only). The s6-s8 arm of
    /// the FASTWINS P0 decomposition — see `S6_TX_SIZE_RDO_LIVE`.
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub rdo_tx_size_override: Option<bool>,
    /// Depth cap for the intra tx-size RDO walk (zenrav1e
    /// `rdo_tx_size_depth`): 1 = largest + one split level, the measured
    /// sweet spot of the P0 decomposition (depth 2 adds ~-0.7% median BD
    /// for +10% time).
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub rdo_tx_size_depth: Option<u8>,
    /// Decoupled intra tx-TYPE RDO (zenrav1e `rdo_tx_type_override`): the
    /// type-search half of the P0 decomposition, standalone
    /// butteraugli-max-vetoed but paired with `reduced_tx_set` it is the
    /// TxBudget::Min head's arm. `None` everywhere in the preset — only the
    /// expert passthrough sets it.
    pub rdo_tx_type_override: Option<bool>,
    /// P1 partition-pruning arm (zenrav1e `topdown_prune`, P1PART
    /// 2026-07-04): NONE-first top-down candidate walk + the opt-in gates
    /// below, used to keep HORZ/VERT partitions affordable at fast tiers
    /// whose tables previously amputated them. See `S6_PART_PRUNE_LIVE`.
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub prune_none_breakout: Option<f32>,
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub prune_rect_margin: Option<f32>,
    /// `Some(0.0)` = HORZ_4/VERT_4 (and mixed 3-way) candidates evaluated
    /// only on SPLIT-dominant blocks (the one-sided NONE-dominance gate at
    /// margin 0: any NONE advantage over the SPLIT-trial estimate skips
    /// them) while HORZ/VERT stay unconditionally live — the shipped s4-s8
    /// configuration.
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub prune_four_way_margin: Option<f32>,
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub prune_homogeneity_gate: Option<f32>,
    /// SATD-decides intra budget (zenrav1e `num_modes_rdo_override`,
    /// 071e9844): cap the number of SATD-ranked intra candidates that get
    /// full RD. `Some(1)` = RD codes only the SATD winner — the aom
    /// winner_mode_reduction analog, the S10-program s9'/s10' ingredient
    /// (see `S10_RETIER_LIVE`).
    // dead_code: same release-gating as mixed_3way_partitions above.
    #[allow(dead_code)]
    pub num_modes_rdo_override: Option<u8>,
    pub min_tile_size: u16,
}

impl SpeedTweaks {
    /// Master switch for the speed-1 "deep" arms (mixed 3-way partitions,
    /// unconditional tx RDO, tuned partition range, deeper SPLIT trial).
    /// FALSE until the zenrav1e dep bumps past 0.1.4: the knobs the deep
    /// arms rely on land on zenrav1e master after that release, and the
    /// partition-range change is only validated on the fixed SPLIT-trial
    /// estimate. While false, `from_my_preset` output is byte-identical to
    /// the pre-s1 table at every speed. Flip at the dep bump and uncomment
    /// the two apply lines in `speed_settings()` (see zenrav1e#27).
    const S1_DEEP_ARMS_LIVE: bool = true;

    /// Master switch for the s6-s8 depth-1 tx-size RDO arms (FASTWINS P0,
    /// FAST_TIER_PARITY_PLAN; zenavif benchmarks/rd_gap_fastwins_2026-07-04
    /// .tsv). The s4->s6 cliff decomposition measured that the coupled
    /// `rdo_tx_decision` boolean's two halves price very differently at s6
    /// (train26, tune-ss2, vs s6-base): depth-1 tx-SIZE RDO with DCT-only
    /// types recovers 51% of the whole s6->s4 RD step (median ssim2 BD
    /// -2.8%, butteraugli-3n -4.4%, butteraugli-max -6.7%, 19-20/24 images
    /// better) at ~1.5x encode time, while the tx-TYPE half costs 2.4x for
    /// -4.5% ssim2 with a butteraugli-max VETO (+0.29 median, better on only
    /// 10/23) — so only the size half ships. Wedge-family recovery at s6:
    /// 6000 scans -14.2%, 9000 clipart -21.8%, 5000 nps -11.3%, 9226
    /// AI-products -7.4% (the traffic-weighted wedge). s8 mirrors it
    /// (-2.9/-4.4/-6.5 at 1.4x). Known cost: fam-7000 plots pay +2..18%
    /// bytes on ~3 KB near-lossless-floor files (worst 7050 +19 BD) — the
    /// class the intraBC/near-lossless program owns, accepted per the
    /// wins+median rule. FALSE until the zenrav1e dep bumps past 0.1.4
    /// (the `rdo_tx_size_override`/`rdo_tx_size_depth` knobs land after
    /// that release); while false, `from_my_preset` output is
    /// byte-identical at every speed. Flip at the dep bump and uncomment
    /// the two apply lines in `speed_settings()`.
    const S6_TX_SIZE_RDO_LIVE: bool = true;

    /// Master switch for the s4-s8 rect-partition liveness arms (P1PART
    /// 2026-07-04, FAST_TIER_PARITY_PLAN P1 lever 1; zenavif
    /// benchmarks/rd_gap_p1part_2026-07-04.tsv). The speed table amputated
    /// HORZ/VERT at s4+ (`non_square_partition_max_threshold` 8×8) and the
    /// SPEED_LADDER wedge map attributed the un-recovered share of the
    /// interiors/food/nature families to exactly that. Ships the measured
    /// gate triple over 16×16-threshold liveness: NONE-first walk +
    /// skip-gated `none_breakout` 1.0 + 16-parent 4-ways restricted to
    /// SPLIT-dominant blocks (`four_way_margin=0.0`, one-sided) +
    /// homogeneity vargate 2.0 — cheaper than the same liveness ungated at
    /// every tier (solo 2.16/2.08/1.75× vs 2.33/2.23/1.91× at s6/s8/s4).
    /// Full-grid 12-q confirms (train26 tune-ss2, vs s6+size1 / s8+size1 /
    /// stock-s4 bases; ssim2/ba3n/bamax medians): s6 −2.89/−2.51/−2.45
    /// (24/24 both primaries), s8 −3.00/−2.49/−2.86 (24/24), s4
    /// −1.94/−2.32/−2.74 (22/23) — no butteraugli-max veto anywhere.
    /// Ladder movement (photos, vs cached aom-allintra arms): s6 vs
    /// cpu4def-ai +1.4→−4.6/−6.3 (crossed both metrics), vs cpu4iq-ai
    /// +7.1→+2.9/+0.9; s8 vs cpu6iq-ai +0.3→−3.6/−5.1 (crossed); s4 vs
    /// cpu2def-ai +2.8→−0.9/−5.6 (crossed). s6 per-family recovery of the
    /// remaining (s6+size1)→s4 step: interiors 60%, food 68%, nps 63%,
    /// scans 183%, screens 175%, ALL 77%. Honest budget note: the ~1.7×
    /// per-lever aspiration is NOT met — the cheapest liveness point IS
    /// 1.75-2.2×; rects-only (`four_way_margin=-1.0`, no other gates)
    /// measured −2.40 median at ~1.8× solo as the fallback, and the
    /// beyond-budget vargate/max32 arms (88-104% step recovery at
    /// 2.4-2.9×) are recorded as P2 per-image-hint targets.
    /// The beyond-budget points (homogeneity-vargate arms at 2.4-2.9×
    /// keeping 88-104% of the remaining s6→s4 step) are recorded in the TSV
    /// as P2 per-image-hint targets, not shipped. Gate decomposition:
    /// NONE-dominance margins on rects are a measured dead end (26-48%
    /// retention in both semantics), the skip-gated none_breakout is a null
    /// at every τ, and the 4×4-log-var homogeneity gate is the one gate
    /// that pays — but it exceeds the per-lever time budget, so the shipped
    /// config is pure liveness + 4-way-off. FALSE until the zenrav1e dep
    /// bumps past 0.1.4 (the `topdown_prune` knob lands after that
    /// release); while false, `from_my_preset` output is byte-identical at
    /// every speed (the 16×16 threshold value is ALSO gated: it is live in
    /// bottom-up edge-superblock coding even on registry builds). Flip at
    /// the dep bump and uncomment the apply block in `speed_settings()`.
    const S6_PART_PRUNE_LIVE: bool = true;

    /// Master switch for the s6-s8 top-7 keyframe intra RDO arm (P2HEADS
    /// 2026-07-04, FAST_TIER_PARITY_PLAN P2 head-3 axis; zenavif
    /// benchmarks/rd_gap_p2heads_2026-07-04.tsv). The table forces
    /// `Simple` (top-3 intra RDO) at every speed as the zenrav1e#5
    /// filter_intra guard; the safe top-7 form is `ComplexKeyframes` +
    /// `filter_intra=Some(false)` (the override knob landed
    /// zenrav1e@49982460, post-0.1.4). Measured on train26 (coarse 6-q,
    /// tune-ss2, veto-adjusted per-image BD): s6 −0.56 med / −0.72 mean
    /// (17/24 better), s8 −1.17 med / −1.03 mean (16/24), composition-
    /// stable on the P1 partition ship point (−0.51 med on-ship), one
    /// +1.4 regressor (8268 screenshot) with no per-image feature
    /// structure at n=24 → a GLOBAL arm, not a zenavif head. On the P2
    /// composed fast mode it added −0.39 med (train26) / −1.34 med (VAL:
    /// composed+i7 −5.32 vs base, 13/13 better, 0 butteraugli vetoes).
    /// Solo cost: see the P2HEADS TSV timing section (p2t_intra7*).
    /// FALSE until the zenrav1e dep bumps past 0.1.4 (the `filter_intra`
    /// override lands after that release; ComplexKeyframes WITHOUT it
    /// re-opens zenrav1e#5); while false, byte-identical at every speed.
    /// Flip at the dep bump and uncomment the apply block in
    /// `speed_settings()`.
    const S6_INTRA7_LIVE: bool = true;

    /// Small-rendition effort mode (zenavif size-decay non-tune A/B,
    /// 2026-07-03): keep tx-size/type RDO ON at high quality when the frame's
    /// long edge is below 1024. Measured (tune-off s2, 12 photo-like train
    /// origins x 16-q, vs the byte-identical baseline): median ssim2 BD
    /// +0.80 @256 / +0.88 @512, arm better 12/12 at BOTH sizes, butteraugli
    /// 3n +2.5/+1.7 agreeing; moves the vs-aom-cpu2 median from +4.17 to
    /// +3.31 @256 and +0.81 to -0.46 @512. Cost is confined to the hi-q band
    /// where the gate flips (~6.5x those cells: ~0.3->2.0 s @256), which is
    /// the point: small frames are where deep search is affordable — the
    /// same philosophy as libaom's resolution-keyed speed features. FALSE
    /// FLIPPED ON 2026-07-03 per user policy sign-off ("you can flip the
    /// smallpx rdo live on"): small renditions (long_edge < 1024) keep
    /// tx-size/type RDO at every quality. Measured (size-decay A/B,
    /// pre-registered rule; the A/B convicted NO default -- this is a
    /// uniform win with size-conditional COST taken deliberately):
    /// VAL +1.44% ssim2 BD @256 / +1.30% @512 (12/12 both), butteraugli
    /// +3.7/+2.9, vs-cpu2 val 512 flips negative; ~6.5x encode time on the
    /// changed high-q small-frame cells only; >=1024 byte-identical by
    /// construction. Record: zenavif docs/RD_GAP_VS_LIBAOM.md "Non-tune size
    /// decay isolation A/B" + benchmarks/hyperparam_sizedecay_nontune_2026-07-03.tsv.
    const SMALL_PX_RDO_TX_LIVE: bool = true;

    /// Master switch for the S10-program re-tiered s9/s10 rows (2026-07-05,
    /// zenavif docs/S10_PROGRAM.md + benchmarks/rd_gap_s10_2026-07-05.tsv).
    /// The JPEG-anchored scoreboard measured the shipped s10 row LOSING to
    /// mozjpeg-class JPEG outright on bytes at matched ssim2 (registry
    /// config 1.05-1.06x jpeg-moz at ssim2<=60, >=1.0 in 7/12 train26
    /// families; with the ss2 tune 0.79-0.84x at 4.6x jpeg-moz encode
    /// time), and decomposed the cliff: `tx_domain_rate` -7.45% median
    /// ssim2 BD for 1.14x (22/22 better, all three metrics), CDEF-on
    /// -1.70% at 1.04x (22-23/23), the (16,16) partition floor -13.5/-17.1/
    /// -20.7 at s9 (23/23), depth-1 tx-size RDO -7.8/-13.3/-13.0 at 1.49x;
    /// fine-directional-intra and reduced_tx_set measured null, (8,32)
    /// ruled out (+10/+18 — 32px blocks misprice under TX LARGEST).
    /// Re-tiered rows (measured composed, train26 6q, tune-ss2+palette,
    /// 0 CELLFAIL / 0 PALCONF failures anywhere):
    ///   s10' = txdr off + CDEF on + SATD-decides intra (num_modes_rdo 1):
    ///          the c11 arm — see the TSV; c1 (txdr+cdef alone) is -8.8%
    ///          median BD at 1.24x, SATD-decides claws back ~19% time.
    ///   s9'  = s10' + partition floor (8,16) + depth-1 tx-size RDO: the
    ///          c7/c13 shape — -28.9/-33.8/-31.2 median BD vs the old s10
    ///          rung at 2.09x its RD-pass time (~678 solo ms/MP = 9.2x
    ///          mozjpeg-class JPEG, bytes 0.54-0.61x JPEG at matched ssim2
    ///          50-80); the old s9 (411 ms/MP) sat at 0.67-0.78x.
    /// The old rungs were off the pareto the moment the scoreboard anchor
    /// became JPEG; the re-tiered ladder is monotone: s10' ~340 ms/MP →
    /// s9' ~680 → s8-composed ~2394 (solo internal, 1 MP renditions).
    /// FALSE until the zenrav1e dep bumps past 0.1.4: the measured configs
    /// include Tune::Ssimulacra2 + PaletteMode::Auto (release-gated) and
    /// the tx-size/num_modes_rdo override knobs land after that release.
    /// While false, `from_my_preset` output is byte-identical at every
    /// speed. Flip at the dep bump together with the tune-default decision
    /// and uncomment the num_modes_rdo apply line in `speed_settings()`.
    /// Alpha-channel caveat: these rows also govern the alpha (Cs400)
    /// encode; the corpus carries no alpha — cost impact there unmeasured.
    const S10_RETIER_LIVE: bool = true;

    pub fn from_my_preset(speed: u8, quantizer: u8, long_edge: usize) -> Self {
        // Use fixed quantizer thresholds instead of quality_to_quantizer()
        // so these don't shift when the quality curve changes
        let low_quality = quantizer > 150;  // ~Q50 and below
        let high_quality = quantizer < 80;   // ~Q80 and above
        let max_block_size = if high_quality { 16 } else { 64 };
        let small_px_rdo_tx =
            Self::SMALL_PX_RDO_TX_LIVE && long_edge < 1024 && long_edge > 0;

        Self {
            speed_preset: speed,

            // Speed 1 is the "deep" maximum-RD mode (no longer byte-identical
            // to speed 2): mixed 3-way partition types, unconditional
            // tx-size/type RDO, and partition_range (4,32) at every quality —
            // the winner of the 16/32/64 s1-bundle ablation (zenavif
            // benchmarks/rd_gap_s1_2026-07-02.tsv; 64 helps only a few smooth
            // images and bleeds elsewhere, 16 starves large blocks).
            partition_range: Some(match speed {
                0 => (4, 64.min(max_block_size)),
                1 if Self::S1_DEEP_ARMS_LIVE => (4, 32.min(max_block_size)),
                1 if low_quality => (4, 64.min(max_block_size)),
                2 if low_quality => (4, 32.min(max_block_size)),
                1..=4 => (4, 16),
                5..=8 => (8, 16),
                // S10 program: the (16,16) floor was the s9 cliff's dominant
                // owner (-13.5/-17.1/-20.7 median BD, 23/23; S10_RETIER_LIVE).
                9 if Self::S10_RETIER_LIVE => (8, 16),
                _ => (16, 16),
            }),

            // ComplexAll only affects inter frames — for AVIF still images
            // (all keyframes), ComplexKeyframes already searches all intra modes.
            // ComplexAll triggers filter_intra RDO paths with broken cost estimation
            // (see zenrav1e#5), causing 12 dB PSNR regression at speed 1.
            complex_prediction_modes: Some(false),
            sgr_complexity_full: Some(speed <= 2),
            // Bottom-up partition search interacts badly with QM: the RDO
            // cost model doesn't account for QM's frequency-dependent weights,
            // causing bottomup to select partitions that are suboptimal under QM.
            // Disabling produces identical quality with QM off and 2-3x faster.
            // TODO: fix the bottomup RDO cost model to account for QM weights
            encode_bottomup: Some(false),

            // big blocks disabled at 3

            // these two are together?
            // Speed 1 keeps tx RDO on at every quality: the !high_quality
            // gate is a matched-speed tradeoff (~7.5x time at -Q80-95 for
            // -5.7% median bytes AND better ssim2 — measured 2026-07-01,
            // zenavif docs/RD_GAP_VS_LIBAOM.md §6b); s1 deliberately spends
            // that time.
            rdo_tx_decision: Some(if speed <= 1 && Self::S1_DEEP_ARMS_LIVE {
                true
            } else if small_px_rdo_tx && speed <= 4 {
                // Small-rendition effort mode: see SMALL_PX_RDO_TX_LIVE.
                true
            } else {
                speed <= 4 && !high_quality // it tends to blur subtle textures
            }),
            reduced_tx_set: Some(speed == 4 || speed >= 9), // It interacts with tx_domain_distortion too?

            // 4px blocks disabled at 5

            fine_directional_intra: Some(speed <= 6),
            fast_deblock: Some(speed >= 7 && !high_quality), // mixed bag?

            // 8px blocks disabled at 8
            lrf: Some(low_quality && speed <= 8), // hardly any help for hi-q images. recovers some q at low quality
            // S10 program: CDEF forced on at s9/s10 measured -1.70/-2.45/
            // -1.89 median BD at 1.04x (22-23/23 better) at s10; +0.30
            // marginal at s9 (S10_RETIER_LIVE).
            cdef: Some(if Self::S10_RETIER_LIVE && speed >= 9 {
                true
            } else {
                low_quality && speed <= 9
            }), // hardly any help for hi-q images. recovers some q at low quality

            inter_tx_split: Some(speed >= 9), // mixed bag even when it works, and it backfires if not used together with reduced_tx_set
            // The "10% larger files" was under-sold: on the JPEG-anchored
            // scoreboard txdr costs -7.45% median ssim2 BD (22/22 better off,
            // all metrics) for only 1.14x time — the s10 cliff's #1 owner
            // (S10_RETIER_LIVE turns it off).
            tx_domain_rate: Some(speed >= 10 && !Self::S10_RETIER_LIVE), // 20% faster, but also 10% larger files!

            tx_domain_distortion: None, // very mixed bag, sometimes helps speed sometimes it doesn't
            use_satd_subpel: Some(false), // doesn't make sense
            segmentation: Some(if speed <= 2 {
                SegmentationLevel::Complex
            } else {
                SegmentationLevel::Simple
            }),
            lru_on_skip: Some(speed <= 1),
            non_square_partition_max_threshold: Some(match speed {
                0..=2 => BlockSize::BLOCK_64X64,
                3 => BlockSize::BLOCK_32X32,
                // s4-s8 rect liveness (P1PART; see S6_PART_PRUNE_LIVE). The
                // value itself must stay gated: it is live in bottom-up
                // edge-superblock coding even on registry zenrav1e builds.
                // s9-s10 unmeasured (s10's 16×16 block floor would make
                // 16×16-threshold rects its ONLY split shapes): keep 8×8.
                4..=8 if Self::S6_PART_PRUNE_LIVE => BlockSize::BLOCK_16X16,
                _ => BlockSize::BLOCK_8X8,
            }),
            // Palette rides the Encoder-level pass-through (with_palette),
            // never the preset: content-gating lives in zenavif's palette_gate.
            palette: None,
            mixed_3way_partitions: Some(speed <= 1 && Self::S1_DEEP_ARMS_LIVE),
            // s6-s8 top-7 keyframe intra RDO (P2HEADS head-3 axis; see
            // S6_INTRA7_LIVE). None elsewhere = the forced-Simple top-3
            // (s2-s5 unmeasured for this arm; s9-s10 drop angle deltas so
            // the top-7 premium is a different question there).
            intra_top7: if Self::S6_INTRA7_LIVE && (6..=8).contains(&speed) {
                Some(true)
            } else {
                None
            },
            // Depth 2 was measured (2026-07-02 s1 ablation): it rescues the
            // worst outliers and turns the mean negative, but loses the
            // pre-registered wins+median rule vs depth 1 at (4,32) — ships 1.
            split_trial_depth: Some(1),
            // s6-s8 keep depth-1 intra tx-SIZE RDO (DCT-only): the measured
            // half of the s4->s6 rdo_tx cliff that pays for itself (see
            // S6_TX_SIZE_RDO_LIVE). None elsewhere = follow rdo_tx_decision
            // exactly (s2-s4 coupled behavior unchanged; s9-s10 unmeasured).
            rdo_tx_size_override: if (Self::S6_TX_SIZE_RDO_LIVE
                && (6..=8).contains(&speed))
                || (Self::S10_RETIER_LIVE && speed == 9)
            {
                Some(true)
            } else {
                None
            },
            rdo_tx_size_depth: if (Self::S6_TX_SIZE_RDO_LIVE
                && (6..=8).contains(&speed))
                || (Self::S10_RETIER_LIVE && speed == 9)
            {
                Some(1)
            } else {
                None
            },
            // Expert-passthrough-only (TxBudget::Min pairs it with
            // reduced_tx_set); never set by the preset.
            rdo_tx_type_override: None,
            // s4-s8 keep HORZ/VERT live (rect threshold 16×16, gated above)
            // under the NONE-first topdown_prune walk with the measured
            // gate triple: skip-gated none_breakout τ=1.0, 16-parent
            // 4-ways restricted to SPLIT-dominant blocks (one-sided margin
            // 0.0), homogeneity vargate 2.0 — cheaper than the same
            // liveness without the gates at every tier (solo 2.16/2.08/
            // 1.75× vs 2.33/2.23/1.91× at s6/s8/s4) at equal RD (see
            // S6_PART_PRUNE_LIVE). All-None elsewhere = historical
            // candidate walk exactly.
            prune_none_breakout: if Self::S6_PART_PRUNE_LIVE
                && (4..=8).contains(&speed)
            {
                Some(1.0)
            } else {
                None
            },
            prune_rect_margin: None,
            prune_four_way_margin: if Self::S6_PART_PRUNE_LIVE
                && (4..=8).contains(&speed)
            {
                Some(0.0)
            } else {
                None
            },
            prune_homogeneity_gate: if Self::S6_PART_PRUNE_LIVE
                && (4..=8).contains(&speed)
            {
                Some(2.0)
            } else {
                None
            },
            // S10 program: SATD-decides at the re-tiered ultra-fast rungs
            // (s10 solo 337 -> ~277 ms/MP for BD ~-0.9; on the composed s9'
            // it keeps 89% of the c4 win at 66% of its time). None elsewhere.
            num_modes_rdo_override: if Self::S10_RETIER_LIVE && speed >= 9 {
                Some(1)
            } else {
                None
            },
            min_tile_size: match speed {
                0 => 4096,
                1 => 2048,
                2 => 1024,
                3 => 512,
                4 => 256,
                _ => 128,
            } * if high_quality { 2 } else { 1 },
        }
    }

    pub(crate) fn speed_settings(&self) -> SpeedSettings {
        let mut speed_settings = SpeedSettings::from_preset(self.speed_preset);

        speed_settings.multiref = false;
        speed_settings.rdo_lookahead_frames = 1;
        speed_settings.scene_detection_mode = SceneDetectionSpeed::None;
        speed_settings.motion.include_near_mvs = false;

        if let Some(v) = self.fast_deblock { speed_settings.fast_deblock = v; }
        if let Some(v) = self.reduced_tx_set { speed_settings.transform.reduced_tx_set = v; }
        if let Some(v) = self.tx_domain_distortion { speed_settings.transform.tx_domain_distortion = v; }
        if let Some(v) = self.tx_domain_rate { speed_settings.transform.tx_domain_rate = v; }
        if let Some(v) = self.encode_bottomup { speed_settings.partition.encode_bottomup = v; }
        if let Some(v) = self.rdo_tx_decision { speed_settings.transform.rdo_tx_decision = v; }
        if let Some(v) = self.cdef { speed_settings.cdef = v; }
        if let Some(v) = self.lrf { speed_settings.lrf = v; }
        if let Some(v) = self.inter_tx_split { speed_settings.transform.enable_inter_tx_split = v; }
        if let Some(v) = self.sgr_complexity_full { speed_settings.sgr_complexity = if v { SGRComplexityLevel::Full } else { SGRComplexityLevel::Reduced } }
        if let Some(v) = self.use_satd_subpel { speed_settings.motion.use_satd_subpel = v; }
        if let Some(v) = self.fine_directional_intra { speed_settings.prediction.fine_directional_intra = v; }
        if let Some(v) = self.complex_prediction_modes { speed_settings.prediction.prediction_modes = if v { PredictionModesSetting::ComplexAll } else { PredictionModesSetting::Simple} }
        // Uncommented on the cooptloop branch (zenrav1e path dep supplies the knob).
        // Must stay AFTER the complex_prediction_modes apply (it refines the forced-Simple guard):
        if self.intra_top7 == Some(true) {
            speed_settings.prediction.prediction_modes = PredictionModesSetting::ComplexKeyframes;
            speed_settings.prediction.filter_intra = Some(false);
        }
        if let Some((min, max)) = self.partition_range {
            debug_assert!(min <= max);
            fn sz(s: u8) -> BlockSize {
                match s {
                    4 => BlockSize::BLOCK_4X4,
                    8 => BlockSize::BLOCK_8X8,
                    16 => BlockSize::BLOCK_16X16,
                    32 => BlockSize::BLOCK_32X32,
                    64 => BlockSize::BLOCK_64X64,
                    128 => BlockSize::BLOCK_128X128,
                    _ => panic!("bad size {s}"),
                }
            }
            speed_settings.partition.partition_range = PartitionRange::new(sz(min), sz(max));
        }
        if let Some(v) = self.segmentation { speed_settings.segmentation = v; }
        if let Some(v) = self.lru_on_skip { speed_settings.lru_on_skip = v; }
        if let Some(v) = self.non_square_partition_max_threshold { speed_settings.partition.non_square_partition_max_threshold = v; }
        if let Some(v) = self.palette { speed_settings.prediction.palette = v; }
        // DEV-ONLY (cooptloop Q2 hole-sweep): env-gated grafts of s6-bundle
        // members onto faster presets, for run_gap-driven candidate arms.
        // Unset env = byte-identical. Never lands beyond the branch.
        match std::env::var("ZENRAVIF_Q2_GRAFT").as_deref() {
            Ok("i7") => {
                speed_settings.prediction.prediction_modes =
                    PredictionModesSetting::ComplexKeyframes;
                speed_settings.prediction.filter_intra = Some(false);
            }
            Ok("prune") => {
                speed_settings.partition.non_square_partition_max_threshold =
                    BlockSize::BLOCK_16X16;
                speed_settings.partition.topdown_prune =
                    Some(TopdownPartitionPrune {
                        none_breakout: Some(1.0),
                        rect_margin: None,
                        four_way_margin: Some(0.0),
                        homogeneity_gate: Some(2.0),
                    });
            }
            Ok("txd2") => {
                speed_settings.transform.rdo_tx_size_override = Some(true);
                speed_settings.transform.rdo_tx_size_depth = Some(2);
            }
            Ok("txmin") => {
                speed_settings.transform.rdo_tx_size_override = Some(true);
                speed_settings.transform.rdo_tx_size_depth = Some(1);
                speed_settings.transform.rdo_tx_type_override = Some(true);
                speed_settings.transform.reduced_tx_set = true;
            }
            Ok("i7prune") => {
                speed_settings.prediction.prediction_modes =
                    PredictionModesSetting::ComplexKeyframes;
                speed_settings.prediction.filter_intra = Some(false);
                speed_settings.partition.non_square_partition_max_threshold =
                    BlockSize::BLOCK_16X16;
                speed_settings.partition.topdown_prune =
                    Some(TopdownPartitionPrune {
                        none_breakout: Some(1.0),
                        rect_margin: None,
                        four_way_margin: Some(0.0),
                        homogeneity_gate: Some(2.0),
                    });
            }
            _ => {}
        }
        // Uncommented on the cooptloop branch (zenrav1e path dep supplies the knobs):
        if let Some(v) = self.mixed_3way_partitions { speed_settings.partition.mixed_3way_partitions = v; }
        if let Some(v) = self.split_trial_depth { speed_settings.partition.split_trial_depth = v; }
        if let Some(v) = self.rdo_tx_size_override { speed_settings.transform.rdo_tx_size_override = Some(v); }
        if let Some(v) = self.rdo_tx_size_depth { speed_settings.transform.rdo_tx_size_depth = Some(v); }
        if let Some(v) = self.rdo_tx_type_override { speed_settings.transform.rdo_tx_type_override = Some(v); }
        if let Some(v) = self.num_modes_rdo_override { speed_settings.prediction.num_modes_rdo_override = Some(v); }
        if self.prune_none_breakout.is_some() || self.prune_rect_margin.is_some()
            || self.prune_four_way_margin.is_some() || self.prune_homogeneity_gate.is_some()
        {
            speed_settings.partition.topdown_prune = Some(TopdownPartitionPrune {
                none_breakout: self.prune_none_breakout,
                rect_margin: self.prune_rect_margin,
                four_way_margin: self.prune_four_way_margin,
                homogeneity_gate: self.prune_homogeneity_gate,
            });
        }

        speed_settings
    }
}

/// The topdown-prune gate quartet (breakout, rect margin, 4-way margin,
/// homogeneity vargate) overriding as a unit — see
/// `expert::InternalParams::prune_none_breakout`.
#[cfg(feature = "imazen")]
type PruneQuartet = (Option<f32>, Option<f32>, Option<f32>, Option<f32>);

/// Apply the fast-tier expert overrides onto a `SpeedTweaks` value (shared by
/// the gray and color encode paths; the alpha stream never receives them).
/// The prune quartet replaces the preset's gates AS A UNIT (see
/// `expert::InternalParams::prune_none_breakout`).
#[cfg(feature = "imazen")]
#[allow(clippy::too_many_arguments)]
fn apply_fast_tier_overrides(
    speed: &mut SpeedTweaks,
    rdo_tx_size: Option<bool>,
    rdo_tx_size_depth: Option<u8>,
    rdo_tx_type: Option<bool>,
    reduced_tx_set: Option<bool>,
    num_modes_rdo: Option<u8>,
    non_square_max_px: Option<u8>,
    prune: Option<PruneQuartet>,
) {
    if rdo_tx_size.is_some() {
        speed.rdo_tx_size_override = rdo_tx_size;
    }
    if rdo_tx_size_depth.is_some() {
        speed.rdo_tx_size_depth = rdo_tx_size_depth;
    }
    if rdo_tx_type.is_some() {
        speed.rdo_tx_type_override = rdo_tx_type;
    }
    if let Some(v) = reduced_tx_set {
        speed.reduced_tx_set = Some(v);
    }
    if num_modes_rdo.is_some() {
        speed.num_modes_rdo_override = num_modes_rdo;
    }
    if let Some(px) = non_square_max_px {
        speed.non_square_partition_max_threshold = Some(match px {
            4 => BlockSize::BLOCK_4X4,
            8 => BlockSize::BLOCK_8X8,
            16 => BlockSize::BLOCK_16X16,
            32 => BlockSize::BLOCK_32X32,
            64 => BlockSize::BLOCK_64X64,
            128 => BlockSize::BLOCK_128X128,
            _ => panic!("bad non_square_partition_max_threshold {px}"),
        });
    }
    if let Some((bk, rect, four_way, vg)) = prune {
        speed.prune_none_breakout = bk;
        speed.prune_rect_margin = rect;
        speed.prune_four_way_margin = four_way;
        speed.prune_homogeneity_gate = vg;
    }
}

struct Av1EncodeConfig {
    pub width: usize,
    pub height: usize,
    pub bit_depth: usize,
    pub quantizer: usize,
    pub speed: SpeedTweaks,
    /// 0 means num_cpus
    pub threads: Option<usize>,
    pub pixel_range: PixelRange,
    pub chroma_sampling: ChromaSampling,
    pub color_description: Option<ColorDescription>,
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
    #[cfg(feature = "imazen")]
    pub enable_qm: bool,
    #[cfg(feature = "imazen")]
    pub enable_vaq: bool,
    #[cfg(feature = "imazen")]
    pub vaq_strength: f64,
    #[cfg(feature = "imazen")]
    pub tune_still_image: bool,
    #[cfg(feature = "imazen")]
    pub lossless: bool,
    #[cfg(feature = "imazen")]
    pub seg_boost: f64,
    // Stored on the `Av1EncodeConfig` for forward-compat plumbing,
    // but actually consumed via `SpeedTweaks` before the config is
    // built — so the fields here are write-only.
    #[cfg(feature = "imazen")]
    #[allow(dead_code)]
    pub override_cdef: Option<bool>,
    #[cfg(feature = "imazen")]
    #[allow(dead_code)]
    pub override_rdo_tx_decision: Option<bool>,
    #[cfg(feature = "imazen")]
    pub enable_trellis: bool,
    /// Per-superblock AC quantizer scale map forwarded to zenrav1e as
    /// `FrameHints::sb_q_scale` (release-gated: see [`FRAME_HINTS_LIVE`]).
    #[cfg(feature = "imazen")]
    pub frame_hints_sb_q_scale: Option<Box<[f32]>>,
    /// True only for the ALPHA plane's AV1 stream. Perceptual tunes measurably
    /// ring on alpha (libavif's finding; its alpha is pinned to tune=psnr) —
    /// this flag pins the alpha stream to `Tune::Psnr` while color/gray keep
    /// the perceptual still-image tune.
    pub is_alpha: bool,
    /// Forwarded to zenrav1e's `max_pixel_count` guard. `0` = unlimited.
    pub max_pixels: u64,
    #[cfg(feature = "stop")]
    pub stop_token: Option<almost_enough::StopToken>,
}

fn rav1e_config(p: &Av1EncodeConfig) -> Config {
    // Tiles are zenrav1e's only intra-frame parallelism unit — but every
    // additional tile costs bytes: each tile restarts entropy-context (CDF)
    // adaptation and truncates cross-tile intra prediction, and the loss is
    // largest exactly where bytes matter most (low quality / low-entropy
    // content, where per-tile adaptation waste is a big share of a small
    // file). Measured on 1024px-class stills (s6, tune-ss2, Q30/60/85,
    // photo/screenshot/product): median +0.9% bytes at 2 tiles, +2.0% at 4,
    // +7.9% at 64 — up to +28% at Q30 on smooth content. See zenavif
    // docs/SPEED_LADDER.md ("Wrapper-level threading/tiling hazard") and
    // zenavif benchmarks/rd_gap_fastwins_2026-07-04.tsv.
    //
    // Default policy: host core count must never silently degrade
    // compression. The tile count is capped so every tile keeps at least
    // TILE_RD_MIN_AREA pixels: images at or below 1 MP never tile (their
    // bytes are identical on a 1-core laptop and a 48-core server), and
    // larger images tile only as far as ≥1 MP tiles allow (4 MP → ≤4,
    // 12 MP → ≤12), which keeps multi-core wins where encodes are actually
    // slow. The speed-preset min_tile_size floor still applies where it is
    // stricter (slow presets). A thread pool wider than the tile count
    // idles during the tile encode loop, so small-image encodes give wall
    // time back on many-core hosts — that is the RD-honest default; prefer
    // a faster `speed` preset over tiling for cheaper small-image encodes.
    //
    // The tile count is a pure function of image size and speed preset —
    // NOT of the thread count. The earlier form of this policy still took
    // `min(threads, …)`, which made >1 MP bitstreams depend on the host's
    // core count / `with_num_threads` value (a 2.3 MP encode differed
    // between threads=1 and threads=8 — caught by zenavif's
    // `gate_kit determinism`, engineering-baseline invariant A3). That
    // violated this comment's own principle: byte output must be identical
    // on a 1-core laptop and a 48-core server. The thread pool now sizes
    // only the tile-encode worker pool (wall time), never the bitstream;
    // a low-thread host encoding a large image pays the same small
    // area-capped tile overhead as everyone else and stays byte-identical.
    let tiles = {
        // Minimum pixel area per tile before the default policy adds a tile.
        const TILE_RD_MIN_AREA: usize = 1 << 20; // 1 MP
        let px = p.width * p.height;
        (px / TILE_RD_MIN_AREA)
            .min(px / (p.speed.min_tile_size as usize).pow(2))
    };
    #[cfg_attr(not(feature = "imazen"), allow(unused_mut))]
    let mut speed_settings = p.speed.speed_settings();
    // The s6-s9 decoupled tx-size RDO arm (S6_TX_SIZE_RDO_LIVE / S10_RETIER's
    // s9 row) was measured on 8-bit SDR corpora only. On 10-bit content it
    // regresses the PQ10 pixel-fidelity envelope (zenavif
    // tests/hdr_roundtrip.rs: q95/s8 max |Δ| 647 → 1025 16-bit units,
    // bisected 2026-07-10 — tune exonerated, intra7/part-prune clean).
    // Restrict the arm to its measured domain until a 10/12-bit re-measure
    // earns it there.
    if p.bit_depth > 8 {
        speed_settings.transform.rdo_tx_size_override = None;
        speed_settings.transform.rdo_tx_size_depth = None;
    }
    let cfg = Config::new()
        .with_encoder_config(EncoderConfig {
        width: p.width,
        height: p.height,
        time_base: Rational::new(1, 1),
        sample_aspect_ratio: Rational::new(1, 1),
        bit_depth: p.bit_depth,
        chroma_sampling: p.chroma_sampling,
        chroma_sample_position: ChromaSamplePosition::Unknown,
        pixel_range: p.pixel_range,
        color_description: p.color_description,
        mastering_display: p.mastering_display,
        content_light: p.content_light,
        enable_timing_info: false,
        still_picture: true,
        error_resilient: false,
        switch_frame_interval: 0,
        min_key_frame_interval: 0,
        max_key_frame_interval: 0,
        reservoir_frame_delay: None,
        low_latency: false,
        quantizer: {
            #[cfg(feature = "imazen")]
            { if p.lossless { 0 } else { p.quantizer } }
            #[cfg(not(feature = "imazen"))]
            { p.quantizer }
        },
        min_quantizer: {
            #[cfg(feature = "imazen")]
            { if p.lossless { 0 } else { p.quantizer as _ } }
            #[cfg(not(feature = "imazen"))]
            { p.quantizer as _ }
        },
        bitrate: 0,
        tune: {
            // cooptloop branch (the flip's tune-default decision): still-image
            // color/gray default to Tune::Ssimulacra2 — the composed tune every
            // ladder measurement was made with (RD_GAP "CURRENT POSITION").
            // Alpha is pinned to Tune::Psnr (perceptual tunes ring on alpha —
            // libavif's measured finding; it pins alpha to tune=psnr).
            // `tune_still_image` keeps its explicit StillImage override.
            if p.is_alpha {
                Tune::Psnr
            } else {
                #[cfg(feature = "imazen")]
                { if p.tune_still_image { Tune::StillImage } else { Tune::Ssimulacra2 } }
                #[cfg(not(feature = "imazen"))]
                { Tune::Ssimulacra2 }
            }
        },
        tile_cols: 0,
        tile_rows: 0,
        tiles,
        film_grain_params: None,
        level_idx: None,
        enable_qm: {
            #[cfg(feature = "imazen")]
            { p.enable_qm }
            #[cfg(not(feature = "imazen"))]
            { false }
        },
        enable_vaq: {
            #[cfg(feature = "imazen")]
            { p.enable_vaq }
            #[cfg(not(feature = "imazen"))]
            { false }
        },
        vaq_strength: {
            #[cfg(feature = "imazen")]
            { p.vaq_strength }
            #[cfg(not(feature = "imazen"))]
            { 1.0 }
        },
        seg_boost: {
            #[cfg(feature = "imazen")]
            { p.seg_boost }
            #[cfg(not(feature = "imazen"))]
            { 1.0 }
        },
        enable_trellis: {
            #[cfg(feature = "imazen")]
            { p.enable_trellis }
            #[cfg(not(feature = "imazen"))]
            { false }
        },
        // Forward zenravif's pixel cap to zenrav1e's own guard instead of
        // nulling it with u64::MAX. `0` keeps the guard disabled (unlimited),
        // matching zenrav1e's `max_pixel_count > 0` convention.
        max_pixel_count: p.max_pixels,
        speed_settings,
        // cooptloop branch: zenrav1e master knobs beyond the 0.1.4 literal
        // (coeff_rd_stack, ssim_rdmult_strength, quant_rounding_bias, variance
        // boost overrides, ...) — all default-inert (the measured-off values).
        ..Default::default()
    });

    if let Some(threads) = p.threads {
        cfg.with_threads(threads)
    } else {
        cfg
    }
}

/// Map rav1e TransferCharacteristics to avif-serialize TransferCharacteristics.
/// Both use CICP values, so this is a 1:1 mapping on the common variants.
fn map_transfer_characteristics(tc: TransferCharacteristics) -> zenavif_serialize::constants::TransferCharacteristics {
    use zenavif_serialize::constants::TransferCharacteristics as TC;
    match tc {
        TransferCharacteristics::BT709 => TC::Bt709,
        TransferCharacteristics::Unspecified => TC::Unspecified,
        TransferCharacteristics::BT470M => TC::Bt470M,
        TransferCharacteristics::BT470BG => TC::Bt470BG,
        TransferCharacteristics::BT601 => TC::Bt601,
        TransferCharacteristics::SMPTE240 => TC::Smpte240,
        TransferCharacteristics::Linear => TC::Linear,
        TransferCharacteristics::Log100 => TC::Log,
        TransferCharacteristics::Log100Sqrt10 => TC::LogSqrt,
        TransferCharacteristics::IEC61966 => TC::Iec61966,
        TransferCharacteristics::BT1361 => TC::Bt1361,
        TransferCharacteristics::SRGB => TC::Srgb,
        TransferCharacteristics::BT2020_10Bit => TC::Bt2020_10,
        TransferCharacteristics::BT2020_12Bit => TC::Bt2020_12,
        TransferCharacteristics::SMPTE2084 => TC::Smpte2084,
        TransferCharacteristics::SMPTE428 => TC::Smpte428,
        TransferCharacteristics::HLG => TC::Hlg,
    }
}

/// Map rav1e ColorPrimaries to avif-serialize ColorPrimaries.
/// Both use CICP values. avif-serialize has fewer variants, so some map to Unspecified.
fn map_color_primaries(cp: ColorPrimaries) -> zenavif_serialize::constants::ColorPrimaries {
    use zenavif_serialize::constants::ColorPrimaries as CP;
    match cp {
        ColorPrimaries::BT709 => CP::Bt709,
        ColorPrimaries::Unspecified => CP::Unspecified,
        ColorPrimaries::BT601 => CP::Bt601,
        ColorPrimaries::BT2020 => CP::Bt2020,
        ColorPrimaries::SMPTE431 => CP::DciP3,
        ColorPrimaries::SMPTE432 => CP::DisplayP3,
        ColorPrimaries::BT470M
        | ColorPrimaries::BT470BG
        | ColorPrimaries::SMPTE240
        | ColorPrimaries::GenericFilm
        | ColorPrimaries::XYZ
        | ColorPrimaries::EBU3213 => CP::Unspecified,
    }
}

fn init_frame_3<P: zenrav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = [P; 3]> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> core::result::Result<(), Error> {
    let mut f = frame.planes.iter_mut();
    let mut planes = planes.into_iter();

    // it doesn't seem to be necessary to fill padding area
    let mut y = f.next().unwrap().mut_slice(Default::default());
    let mut u = f.next().unwrap().mut_slice(Default::default());
    let mut v = f.next().unwrap().mut_slice(Default::default());

    let mut pixel_count = 0usize;
    const CHECK_INTERVAL: usize = 1_000_000; // Check every ~1MP

    for ((y, u), v) in y.rows_iter_mut().zip(u.rows_iter_mut()).zip(v.rows_iter_mut()).take(height) {
        let y = &mut y[..width];
        let u = &mut u[..width];
        let v = &mut v[..width];
        for ((y, u), v) in y.iter_mut().zip(u).zip(v) {
            let px = planes.next().ok_or(Error::TooFewPixels)?;
            *y = px[0];
            *u = px[1];
            *v = px[2];

            pixel_count += 1;
            if pixel_count.is_multiple_of(CHECK_INTERVAL) {
                check_cancellation(cancel_token, deadline)?;
            }
        }
    }
    Ok(())
}

/// Initialize a frame with 4:2:0 chroma subsampling.
/// Luma is written at full resolution, chroma is box-filtered to half resolution.
fn init_frame_3_420<P: zenrav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = [P; 3]> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> core::result::Result<(), Error> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let mut f = frame.planes.iter_mut();
    let mut planes = planes.into_iter();

    let mut y_plane = f.next().unwrap().mut_slice(Default::default());
    let mut u_plane = f.next().unwrap().mut_slice(Default::default());
    let mut v_plane = f.next().unwrap().mut_slice(Default::default());

    // Process two luma rows at a time, producing one chroma row each pair
    let mut y_rows = y_plane.rows_iter_mut();
    let mut u_rows = u_plane.rows_iter_mut();
    let mut v_rows = v_plane.rows_iter_mut();

    let mut pixel_count = 0usize;
    const CHECK_INTERVAL: usize = 1_000_000;

    // We need to buffer one row of chroma accumulators
    // Use u32 to avoid overflow when summing up to 4 P values
    let mut u_acc: Vec<u32> = vec![0; chroma_width];
    let mut v_acc: Vec<u32> = vec![0; chroma_width];
    let mut row_count: Vec<u8> = vec![0; chroma_width];

    for row_idx in 0..height {
        let y_row = y_rows.next().unwrap();
        let y_row = &mut y_row[..width];

        for (col_idx, y_out) in y_row.iter_mut().enumerate() {
            let px = planes.next().ok_or(Error::TooFewPixels)?;
            *y_out = px[0];

            let cx = col_idx / 2;
            u_acc[cx] += Into::<u32>::into(px[1]);
            v_acc[cx] += Into::<u32>::into(px[2]);
            // Track how many pixels contribute to this chroma sample
            // (1, 2, or 4 depending on edge conditions)
            if row_idx % 2 == 0 && col_idx % 2 == 0 {
                row_count[cx] = 1;
            } else {
                row_count[cx] += 1;
            }

            pixel_count += 1;
            if pixel_count.is_multiple_of(CHECK_INTERVAL) {
                check_cancellation(cancel_token, deadline)?;
            }
        }

        // After every second row (or the last row if height is odd), write chroma
        if row_idx % 2 == 1 || row_idx == height - 1 {
            let chroma_row_idx = row_idx / 2;
            if chroma_row_idx < chroma_height {
                let u_row = u_rows.next().unwrap();
                let v_row = v_rows.next().unwrap();
                let u_row = &mut u_row[..chroma_width];
                let v_row = &mut v_row[..chroma_width];

                for cx in 0..chroma_width {
                    let count = u32::from(row_count[cx]);
                    // Box filter: average with rounding
                    let u_val = (u_acc[cx] + count / 2) / count;
                    let v_val = (v_acc[cx] + count / 2) / count;
                    u_row[cx] = P::cast_from(u_val);
                    v_row[cx] = P::cast_from(v_val);
                }

                // Reset accumulators for next pair
                u_acc.iter_mut().for_each(|v| *v = 0);
                v_acc.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(())
}

fn init_frame_1<P: zenrav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = P> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> core::result::Result<(), Error> {
    let mut y = frame.planes[0].mut_slice(Default::default());
    let mut planes = planes.into_iter();

    let mut pixel_count = 0usize;
    const CHECK_INTERVAL: usize = 1_000_000; // Check every ~1MP

    for y in y.rows_iter_mut().take(height) {
        let y = &mut y[..width];
        for y in y.iter_mut() {
            *y = planes.next().ok_or(Error::TooFewPixels)?;

            pixel_count += 1;
            if pixel_count.is_multiple_of(CHECK_INTERVAL) {
                check_cancellation(cancel_token, deadline)?;
            }
        }
    }
    Ok(())
}

/// Whether the per-superblock quantizer-scale hint passthrough
/// (`expert::InternalParams::sb_q_scale` → zenrav1e `FrameHints`) is
/// active in this build. **FALSE until the zenrav1e dep bumps past
/// 0.1.4**: the `FrameHints` input lands on zenrav1e master at
/// `c4047cec`, after the 0.1.4 release. While false, supplied maps are
/// accepted but not applied (encodes stay byte-identical), so
/// closed-loop callers MUST check this and fail honestly rather than
/// silently paying for a second pass that cannot steer anything. At the
/// dep bump: flip to `true` and uncomment the hinted-send block in
/// `encode_to_av1`.
#[cfg(feature = "imazen")]
pub const FRAME_HINTS_LIVE: bool = true;

#[inline(never)]
fn encode_to_av1<P: zenrav1e::Pixel>(
    p: &Av1EncodeConfig,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
    // `init` runs the per-pixel fill loop, so it stays bare `Error` (no `At`)
    // and we attach the trace once, at the single call site below.
    init: impl FnOnce(&mut Frame<P>) -> core::result::Result<(), Error>,
) -> Result<Vec<u8>> {
    // Check cancellation/timeout before starting
    if cancel_token.is_some_and(|t| t.is_cancelled()) {
        return Err(at!(Error::Cancelled));
    }
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return Err(at!(Error::Cancelled));
    }

    // Consume zenrav1e's bare `InvalidConfig` and trace it here. `Error::from`
    // preserves the rav1e reason string (see error.rs).
    // TODO(whereat): when this crate bumps to zenrav1e ^0.2.0 (which returns
    // `At<InvalidConfig>`), switch this to `.map_err_at(Error::from)?` to carry
    // zenrav1e's own trace instead of starting a fresh one here.
    let mut ctx: Context<P> = rav1e_config(p).new_context().map_err(|e| at!(Error::from(e)))?;

    // Wire per-superblock cooperative cancellation via zenrav1e's stop feature.
    // This enables cancellation DURING encoding, not just between packets.
    // Prefer the direct stop token; fall back to wrapping CancellationToken.
    #[cfg(feature = "stop")]
    {
        if let Some(ref stop) = p.stop_token {
            ctx.set_stop(std::sync::Arc::new(stop.clone()));
        } else if let Some(token) = cancel_token {
            ctx.set_stop(std::sync::Arc::new(token.clone()));
        }
    }

    let mut frame = ctx.new_frame();

    // `init` ran the per-pixel fill loop with bare errors; trace it here, at the
    // boundary, rather than inside the loop.
    init(&mut frame).map_err(|e| at!(e))?;
    // `send_frame` returns a bare `EncoderStatus`; convert it (preserving the
    // rav1e reason) and trace it at this boundary.
    //
    // Per-SB quantizer-scale hints (closed-loop second pass). Armed on the
    // cooptloop branch (zenrav1e path dep supplies `FrameParameters.frame_hints`
    // + `FrameHints`, master `c4047cec`): the hinted send replaces the plain
    // send whenever a hint map is present.
    #[cfg(feature = "imazen")]
    let hint_map: Option<Box<[f32]>> =
        if FRAME_HINTS_LIVE { p.frame_hints_sb_q_scale.clone() } else { None };
    #[cfg(not(feature = "imazen"))]
    let hint_map: Option<Box<[f32]>> = None;
    if let Some(map) = hint_map {
        let params = FrameParameters {
            frame_hints: Some(std::sync::Arc::new(
                FrameHints::new().with_sb_q_scale(map),
            )),
            ..Default::default()
        };
        ctx.send_frame((std::sync::Arc::new(frame), params))
            .map_err(|e| at!(Error::from(e)))?;
    } else {
        ctx.send_frame(frame).map_err(|e| at!(Error::from(e)))?;
    }
    ctx.flush();

    let mut out = Vec::new();

    loop {
        // Check cancellation on every iteration (fast: ~5-15ns for token, ~20-50ns for timeout)
        // This ensures responsive cancellation even if receive_packet() is slow.
        // These are genuine cancellation EXITS (not bare receive_packet status
        // matching), so they trace via `at!`.
        if cancel_token.is_some_and(|t| t.is_cancelled()) {
            return Err(at!(Error::Cancelled));
        }
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Err(at!(Error::Cancelled));
        }

        // Hot loop: bare `EncoderStatus` matching for the non-error control-flow
        // statuses (Encoded/LimitReached → break) stays untraced by design.
        // Only the genuine-error exits below trace via `at!`.
        match ctx.receive_packet() {
            Ok(mut packet) => match packet.frame_type {
                FrameType::KEY => {
                    out.append(&mut packet.data);
                },
                _ => continue,
            },
            Err(EncoderStatus::Encoded | EncoderStatus::LimitReached) => break,
            #[cfg(feature = "stop")]
            Err(EncoderStatus::Cancelled) => return Err(at!(Error::Cancelled)),
            Err(err) => return Err(at!(Error::from(err))),
        }
    }
    Ok(out)
}
