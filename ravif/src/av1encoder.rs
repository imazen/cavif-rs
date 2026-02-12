#![allow(deprecated)]
use std::borrow::Cow;
use crate::cancel::CancellationToken;
use crate::dirtyalpha::blurred_dirty_alpha;
use crate::error::Error;
#[cfg(not(feature = "threading"))]
use crate::rayoff as rayon;
use imgref::{Img, ImgVec};
use rav1e::prelude::*;
use rgb::{RGB8, RGBA8};

/// Helper to check cancellation with minimal overhead
/// Returns Error::Cancelled if cancellation is requested
#[inline(always)]
fn check_cancellation(
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> Result<(), Error> {
    if let Some(token) = cancel_token {
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
    }
    if let Some(deadline) = deadline {
        if std::time::Instant::now() >= deadline {
            return Err(Error::Cancelled);
        }
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
    quantizer: u8,
    /// 0-255 scale
    alpha_quantizer: u8,
    /// rav1e preset 1 (slow) 10 (fast but crappy)
    speed: u8,
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
    /// Optional timeout duration for encoding
    timeout: Option<std::time::Duration>,
    /// Override color primaries (default: BT709 for sRGB)
    color_primaries: Option<ColorPrimaries>,
    /// Override transfer characteristics (default: SRGB)
    transfer_characteristics: Option<TransferCharacteristics>,
    /// Override pixel range (default: Full)
    pixel_range: Option<PixelRange>,
    /// HDR mastering display metadata (SMPTE ST 2086)
    mastering_display: Option<MasteringDisplay>,
    /// HDR content light level metadata (CEA-861.3)
    content_light: Option<ContentLight>,
    /// Enable AV1 quantization matrices (imazen/rav1e fork)
    #[cfg(feature = "imazen")]
    enable_qm: bool,
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
    /// Override CDEF on/off (None = use speed preset default)
    #[cfg(feature = "imazen")]
    override_cdef: Option<bool>,
    /// Override rdo_tx_decision on/off (None = use speed preset default)
    #[cfg(feature = "imazen")]
    override_rdo_tx_decision: Option<bool>,
}

impl<'exif_slice> Default for Encoder<'exif_slice> {
    fn default() -> Self {
        Self {
            quantizer: quality_to_quantizer(80.),
            alpha_quantizer: quality_to_quantizer(80.),
            speed: 5,
            output_depth: BitDepth::default(),
            chroma_subsampling: ChromaSubsampling::default(),
            premultiplied_alpha: false,
            color_model: ColorModel::YCbCr,
            threads: None,
            exif: None,
            alpha_color_mode: AlphaColorMode::UnassociatedClean,
            cancellation_token: None,
            timeout: None,
            color_primaries: None,
            transfer_characteristics: None,
            pixel_range: None,
            mastering_display: None,
            content_light: None,
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
            override_cdef: None,
            #[cfg(feature = "imazen")]
            override_rdo_tx_decision: None,
        }
    }
}

/// Builder methods
impl<'exif_slice> Encoder<'exif_slice> {
    /// Start here
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Quality `1..=100`. Panics if out of range.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_quality(mut self, quality: f32) -> Self {
        assert!((1. ..=100.).contains(&quality));
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

    /// Quality for the alpha channel only. `1..=100`. Panics if out of range.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_alpha_quality(mut self, quality: f32) -> Self {
        assert!((1. ..=100.).contains(&quality));
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
    /// Note: At the same perceived quality, ravif typically produces smaller files
    /// due to rav1e's superior encoding efficiency. This advantage appears when
    /// comparing images with matched visual quality scores, not matched Q numbers.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_libavif_quality(mut self, quality: f32) -> Self {
        assert!((1. ..=100.).contains(&quality));
        let q = quality.clamp(0., 100.);
        // Use exact libavif mapping: qindex = (100 - q) * 255 / 100
        self.quantizer = ((100. - q) * 255. / 100.).round() as u8;
        self
    }

    /// * 1 = very very slow, but max compression.
    /// * 10 = quick, but larger file sizes and lower quality.
    ///
    /// Panics if outside `1..=10`.
    #[inline(always)]
    #[track_caller]
    #[must_use]
    pub fn with_speed(mut self, speed: u8) -> Self {
        assert!((1..=10).contains(&speed));
        self.speed = speed;
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
        assert!(num_threads.is_none_or(|n| n > 0));
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
    /// use ravif::*;
    /// use std::time::Duration;
    /// # fn example(pixels: &[RGBA8], width: usize, height: usize) {
    ///
    /// let encoder = Encoder::new()
    ///     .with_quality(70.0)
    ///     .with_timeout(Duration::from_millis(100));
    ///
    /// match encoder.encode_rgba(Img::new(pixels, width, height)) {
    ///     Ok(result) => println!("Encoded successfully"),
    ///     Err(Error::Cancelled) => println!("Encoding timed out"),
    ///     Err(e) => eprintln!("Error: {:?}", e),
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
    /// and fewer bits to textured regions. Default: enabled, strength 0.5.
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
    /// Default: enabled.
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
}

/// Once done with config, call one of the `encode_*` functions
impl Encoder<'_> {
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
    pub fn encode_rgba(&self, in_buffer: Img<&[rgb::RGBA<u8>]>) -> Result<EncodedImage, Error> {
        let new_alpha = self.convert_alpha_8bit(in_buffer);
        let buffer = new_alpha.as_ref().map(|b| b.as_ref()).unwrap_or(in_buffer);
        let use_alpha = buffer.pixels().any(|px| px.a != 255);
        if !use_alpha {
            return self.encode_rgb_internal_from_8bit(buffer.width(), buffer.height(), buffer.pixels().map(|px| px.rgb()));
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
                self.encode_raw_planes_8_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients)
            },
            BitDepth::Ten | BitDepth::Auto => {
                let planes = buffer.pixels().map(|px| match self.color_model {
                    ColorModel::YCbCr => rgb_to_10_bit_ycbcr(px.rgb(), BT601).into(),
                    ColorModel::RGB => rgb_to_10_bit_gbr(px.rgb()).into(),
                });
                let alpha = buffer.pixels().map(|px| to_ten(px.a));
                self.encode_raw_planes_10_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients)
            },
            BitDepth::Twelve => {
                let planes = buffer.pixels().map(|px| match self.color_model {
                    ColorModel::YCbCr => rgb_to_12_bit_ycbcr(px.rgb(), BT601).into(),
                    ColorModel::RGB => rgb_to_12_bit_gbr(px.rgb()).into(),
                });
                let alpha = buffer.pixels().map(|px| to_twelve(px.a));
                self.encode_raw_planes_12_bit(width, height, planes, Some(alpha), pixel_range, matrix_coefficients)
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
    pub fn encode_rgb(&self, buffer: Img<&[RGB8]>) -> Result<EncodedImage, Error> {
        self.encode_rgb_internal_from_8bit(buffer.width(), buffer.height(), buffer.pixels())
    }

    fn encode_rgb_internal_from_8bit(&self, width: usize, height: usize, pixels: impl Iterator<Item = RGB8> + Send + Sync) -> Result<EncodedImage, Error> {
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
    ) -> Result<EncodedImage, Error> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 8)
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
    ) -> Result<EncodedImage, Error> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 10)
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
    ) -> Result<EncodedImage, Error> {
        self.encode_raw_planes_internal(width, height, planes, alpha, color_pixel_range, matrix_coefficients, 12)
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn encode_raw_planes_internal<P: rav1e::Pixel + Default>(
        &self, width: usize, height: usize,
        planes: impl IntoIterator<Item = [P; 3]> + Send,
        alpha: Option<impl IntoIterator<Item = P> + Send>,
        color_pixel_range: PixelRange, matrix_coefficients: MatrixCoefficients,
        input_pixels_bit_depth: u8,
    ) -> Result<EncodedImage, Error> {
        if self.chroma_subsampling == ChromaSubsampling::Yuv420 && matrix_coefficients == MatrixCoefficients::Identity {
            return Err(Error::Unsupported("4:2:0 chroma subsampling with RGB color model"));
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
        let encode_color = move || {
            let mut speed = SpeedTweaks::from_my_preset(self.speed, self.quantizer);
            #[cfg(feature = "imazen")]
            {
                if let Some(v) = override_cdef { speed.cdef = Some(v); }
                if let Some(v) = override_rdo_tx_decision { speed.rdo_tx_decision = Some(v); }
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
                    override_cdef,
                    #[cfg(feature = "imazen")]
                    override_rdo_tx_decision,
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
                        speed: SpeedTweaks::from_my_preset(self.speed, self.alpha_quantizer),
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
                        override_cdef: None,
                        #[cfg(feature = "imazen")]
                        override_rdo_tx_decision: None,
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
        let (color, alpha) = (color?, alpha.transpose()?);

        let mut serializer_config = avif_serialize::Aviffy::new();
        serializer_config
            .matrix_coefficients(match matrix_coefficients {
                MatrixCoefficients::Identity => avif_serialize::constants::MatrixCoefficients::Rgb,
                MatrixCoefficients::BT709 => avif_serialize::constants::MatrixCoefficients::Bt709,
                MatrixCoefficients::Unspecified => avif_serialize::constants::MatrixCoefficients::Unspecified,
                MatrixCoefficients::BT601 => avif_serialize::constants::MatrixCoefficients::Bt601,
                MatrixCoefficients::YCgCo => avif_serialize::constants::MatrixCoefficients::Ycgco,
                MatrixCoefficients::BT2020NCL => avif_serialize::constants::MatrixCoefficients::Bt2020Ncl,
                MatrixCoefficients::BT2020CL => avif_serialize::constants::MatrixCoefficients::Bt2020Cl,
                _ => return Err(Error::Unsupported("matrix coefficients")),
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

fn quality_to_quantizer(quality: f32) -> u8 {
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
struct SpeedTweaks {
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
    pub min_tile_size: u16,
}

impl SpeedTweaks {
    pub fn from_my_preset(speed: u8, quantizer: u8) -> Self {
        // Use fixed quantizer thresholds instead of quality_to_quantizer()
        // so these don't shift when the quality curve changes
        let low_quality = quantizer > 150;  // ~Q50 and below
        let high_quality = quantizer < 80;   // ~Q80 and above
        let max_block_size = if high_quality { 16 } else { 64 };

        Self {
            speed_preset: speed,

            partition_range: Some(match speed {
                0 => (4, 64.min(max_block_size)),
                1 if low_quality => (4, 64.min(max_block_size)),
                2 if low_quality => (4, 32.min(max_block_size)),
                1..=4 => (4, 16),
                5..=8 => (8, 16),
                _ => (16, 16),
            }),

            complex_prediction_modes: Some(speed <= 1), // 2x-3x slower, 2% better
            sgr_complexity_full: Some(speed <= 2), // 15% slower, barely improves anything -/+1%

            encode_bottomup: Some(speed <= 2), // may be costly (+60%), may even backfire

            // big blocks disabled at 3

            // these two are together?
            rdo_tx_decision: Some(speed <= 4 && !high_quality), // it tends to blur subtle textures
            reduced_tx_set: Some(speed == 4 || speed >= 9), // It interacts with tx_domain_distortion too?

            // 4px blocks disabled at 5

            fine_directional_intra: Some(speed <= 6),
            fast_deblock: Some(speed >= 7 && !high_quality), // mixed bag?

            // 8px blocks disabled at 8
            lrf: Some(low_quality && speed <= 8), // hardly any help for hi-q images. recovers some q at low quality
            cdef: Some(low_quality && speed <= 9), // hardly any help for hi-q images. recovers some q at low quality

            inter_tx_split: Some(speed >= 9), // mixed bag even when it works, and it backfires if not used together with reduced_tx_set
            tx_domain_rate: Some(speed >= 10), // 20% faster, but also 10% larger files!

            tx_domain_distortion: None, // very mixed bag, sometimes helps speed sometimes it doesn't
            use_satd_subpel: Some(false), // doesn't make sense
            segmentation: Some(if speed <= 2 {
                SegmentationLevel::Complex
            } else {
                SegmentationLevel::Simple
            }),
            lru_on_skip: Some(speed <= 1),
            non_square_partition_max_threshold: Some(match speed {
                0..=1 => BlockSize::BLOCK_64X64,
                2..=3 => BlockSize::BLOCK_32X32,
                _ => BlockSize::BLOCK_8X8,
            }),
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

        speed_settings
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
    pub override_cdef: Option<bool>,
    #[cfg(feature = "imazen")]
    pub override_rdo_tx_decision: Option<bool>,
}

fn rav1e_config(p: &Av1EncodeConfig) -> Config {
    // AV1 needs all the CPU power you can give it,
    // except when it'd create inefficiently tiny tiles
    let tiles = {
        let threads = p.threads.unwrap_or_else(rayon::current_num_threads);
        threads.min((p.width * p.height) / (p.speed.min_tile_size as usize).pow(2))
    };
    let speed_settings = p.speed.speed_settings();
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
            #[cfg(feature = "imazen")]
            { if p.tune_still_image { Tune::StillImage } else { Tune::Psychovisual } }
            #[cfg(not(feature = "imazen"))]
            { Tune::Psychovisual }
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
        speed_settings,
    });

    if let Some(threads) = p.threads {
        cfg.with_threads(threads)
    } else {
        cfg
    }
}

/// Map rav1e TransferCharacteristics to avif-serialize TransferCharacteristics.
/// Both use CICP values, so this is a 1:1 mapping on the common variants.
fn map_transfer_characteristics(tc: TransferCharacteristics) -> avif_serialize::constants::TransferCharacteristics {
    use avif_serialize::constants::TransferCharacteristics as TC;
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
fn map_color_primaries(cp: ColorPrimaries) -> avif_serialize::constants::ColorPrimaries {
    use avif_serialize::constants::ColorPrimaries as CP;
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

fn init_frame_3<P: rav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = [P; 3]> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> Result<(), Error> {
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
            if pixel_count % CHECK_INTERVAL == 0 {
                check_cancellation(cancel_token, deadline)?;
            }
        }
    }
    Ok(())
}

/// Initialize a frame with 4:2:0 chroma subsampling.
/// Luma is written at full resolution, chroma is box-filtered to half resolution.
fn init_frame_3_420<P: rav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = [P; 3]> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> Result<(), Error> {
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
            if pixel_count % CHECK_INTERVAL == 0 {
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

fn init_frame_1<P: rav1e::Pixel + Default>(
    width: usize,
    height: usize,
    planes: impl IntoIterator<Item = P> + Send,
    frame: &mut Frame<P>,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
) -> Result<(), Error> {
    let mut y = frame.planes[0].mut_slice(Default::default());
    let mut planes = planes.into_iter();

    let mut pixel_count = 0usize;
    const CHECK_INTERVAL: usize = 1_000_000; // Check every ~1MP

    for y in y.rows_iter_mut().take(height) {
        let y = &mut y[..width];
        for y in y.iter_mut() {
            *y = planes.next().ok_or(Error::TooFewPixels)?;

            pixel_count += 1;
            if pixel_count % CHECK_INTERVAL == 0 {
                check_cancellation(cancel_token, deadline)?;
            }
        }
    }
    Ok(())
}

#[inline(never)]
fn encode_to_av1<P: rav1e::Pixel>(
    p: &Av1EncodeConfig,
    cancel_token: Option<&CancellationToken>,
    deadline: Option<std::time::Instant>,
    init: impl FnOnce(&mut Frame<P>) -> Result<(), Error>,
) -> Result<Vec<u8>, Error> {
    // Check cancellation/timeout before starting
    if let Some(token) = cancel_token {
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
    }
    if let Some(deadline) = deadline {
        if std::time::Instant::now() >= deadline {
            return Err(Error::Cancelled);
        }
    }

    let mut ctx: Context<P> = rav1e_config(p).new_context()?;
    let mut frame = ctx.new_frame();

    init(&mut frame)?;
    ctx.send_frame(frame)?;
    ctx.flush();

    let mut out = Vec::new();

    loop {
        // Check cancellation on every iteration (fast: ~5-15ns for token, ~20-50ns for timeout)
        // This ensures responsive cancellation even if receive_packet() is slow
        if let Some(token) = cancel_token {
            if token.is_cancelled() {
                return Err(Error::Cancelled);
            }
        }
        if let Some(deadline) = deadline {
            if std::time::Instant::now() >= deadline {
                return Err(Error::Cancelled);
            }
        }

        match ctx.receive_packet() {
            Ok(mut packet) => match packet.frame_type {
                FrameType::KEY => {
                    out.append(&mut packet.data);
                },
                _ => continue,
            },
            Err(EncoderStatus::Encoded | EncoderStatus::LimitReached) => break,
            Err(err) => Err(err)?,
        }
    }
    Ok(out)
}
