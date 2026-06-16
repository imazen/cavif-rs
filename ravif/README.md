# zenravif ![CI](https://img.shields.io/github/actions/workflow/status/imazen/cavif-rs/ci.yml?style=flat-square&label=CI) ![crates.io](https://img.shields.io/crates/v/zenravif?style=flat-square) [![lib.rs](https://img.shields.io/crates/v/zenravif?style=flat-square&label=lib.rs&color=blue)](https://lib.rs/crates/zenravif) ![docs.rs](https://img.shields.io/docsrs/zenravif?style=flat-square) ![license](https://img.shields.io/crates/l/zenravif?style=flat-square)

Pure Rust AVIF encoder. Wraps [zenrav1e](https://lib.rs/crates/zenrav1e) (AV1 encoder) and [zenavif-serialize](https://lib.rs/crates/zenavif-serialize) (AVIF container muxer) to produce AVIF files from RGB/RGBA pixels.

Supports still images, animated sequences, HDR metadata, chroma subsampling, and cancellation/timeout.

## Fork of ravif

Forked from [ravif](https://lib.rs/crates/ravif) v0.13.0 by Kornel Lesinski. Uses [zenrav1e](https://lib.rs/crates/zenrav1e) (Imazen's rav1e fork) instead of upstream rav1e.

Changes from upstream:
- **Animation** — `encode_animation_rgb`, `encode_animation_rgba`, 16-bit variants
- **HDR metadata** — mastering display (SMPTE ST 2086), content light level (CEA-861.3)
- **Container metadata** — rotation, mirror, ICC profile, XMP, EXIF embedding
- **Cancellation** — `CancellationToken` + `with_timeout()` for responsive cancellation
- **12-bit encoding** — `BitDepth::Twelve` for HDR content
- **4:2:0 subsampling** — `ChromaSubsampling::Yuv420` with box-filter downsampling
- **libavif-compatible quality** — `with_libavif_quality()` for avifenc-matching Q scale
- **Imazen fork features** — QM, VAQ, still-image tuning, lossless, trellis quantization (behind `imazen` feature)

## Usage

```toml
[dependencies]
zenravif = "0.2.0"
```

### Error handling

The encode API returns `At<Error>` — the `Error` enum wrapped by
[`whereat`](https://lib.rs/crates/whereat) with a `file:line:col` trace (and a
clickable GitHub link when built from a checkout of this repo). This makes
server-side failures debuggable: you learn *where* an encode failed, not just
*that* it did, with zero cost on the success path. The crate's `Result<T>`
alias is `core::result::Result<T, At<Error>>`.

- Match the underlying error by borrowing it with `.error()`:
  `Err(e) if matches!(e.error(), Error::Cancelled)`.
- Print the location trail with `.full_trace()`. The plain `Display` of
  `At<Error>` shows only the error message, so code that just prints the error
  keeps working, and `?`-conversion into `Box<dyn Error>` / `anyhow::Error`
  works unchanged (`At<Error>` implements `std::error::Error`).

Errors reported by the underlying rav1e encoder preserve rav1e's own reason
string (e.g. `Encoding error reported by rav1e: invalid width 70000 (expected >= 16, <= 65535)`),
rather than a fixed placeholder.

### Pixel types — where they come from

`use zenravif::*;` re-exports everything you need, so you do **not** need to add
the `rgb` or `imgref` crates as direct dependencies:

- `RGB8` and `RGBA8` are re-exported from the [`rgb`](https://lib.rs/crates/rgb)
  crate. Construct them field-by-field: `RGB8::new(r, g, b)` and
  `RGBA8::new(r, g, b, a)` — the argument order is red, green, blue (then alpha).
- `Img` is re-exported from the [`imgref`](https://lib.rs/crates/imgref) crate.
  `Img::new(pixels, width, height)` wraps a tightly-packed slice with its
  dimensions.

If you already have raw `&[u8]` RGB bytes, the `rgb` crate's `ComponentSlice`
trait gives you `as_rgb()` to view them as `&[RGB8]` — but that trait is not
re-exported here, so add `rgb` directly if you go that route. Building `RGB8`
values explicitly (as below) needs no extra dependency.

### Still image (RGB8)

For opaque images, encode 3-byte RGB directly with `encode_rgb` — there is no
need to pad to RGBA:

```rust
use zenravif::*;

# fn demo(width: usize, height: usize) -> Result<()> {
// RGB8::new(r, g, b) — one entry per pixel, row-major, tightly packed.
let pixels: Vec<RGB8> = vec![RGB8::new(255, 0, 0); width * height];
let img = Img::new(pixels.as_slice(), width, height);

let result = Encoder::new()
    .with_quality(80.0)                                  // see "Quality scale" below
    .with_speed(5)                                       // 1 = slowest/best … 10 = fastest
    .with_chroma_subsampling(ChromaSubsampling::Yuv420)  // 4:2:0; omit for 4:4:4 (default)
    .encode_rgb(img)?;

std::fs::write("output.avif", &result.avif_file)?;
# Ok(()) }
```

### Still image (RGBA8)

For images with transparency, use `encode_rgba` with `RGBA8` pixels:

```rust
use zenravif::*;

# fn demo(width: usize, height: usize) -> Result<()> {
let pixels: Vec<RGBA8> = vec![RGBA8::new(255, 0, 0, 255); width * height];
let img = Img::new(pixels.as_slice(), width, height);

let result = Encoder::new()
    .with_quality(80.0)
    .with_speed(5)
    .encode_rgba(img)?;

std::fs::write("output.avif", &result.avif_file)?;
# Ok(()) }
```

### Quality scale (`with_quality` vs `with_libavif_quality`)

There are two ways to set quality, and they use **different Q scales** — passing
the same number to each produces a different image, so pick one deliberately:

- **`with_quality(q: f32)`** — `q` is in `1..=100`, and **higher means better
  quality** (larger files). The default, if you set nothing, is `80`. This uses
  zenravif's own non-linear quality→quantizer curve, tuned so the perceptual step
  between adjacent values is roughly even across the range.
- **`with_libavif_quality(q: f32)`** — also `1..=100`, higher = better, but it
  applies the **exact libavif/avifenc linear mapping** (`qindex = (100 − q) ×
  255 / 100`). Use this only when you want your Q numbers to line up with
  `avifenc`'s — e.g. for apples-to-apples comparisons. At a *matched perceived
  quality* (not a matched Q number) zenravif typically produces smaller files
  than avifenc thanks to rav1e's efficiency.

Out-of-range or zero values are silently clamped during encoding; call
`Encoder::validate()` first if you want fail-fast behaviour instead.
`with_alpha_quality(q)` sets the alpha plane's quality on the same `1..=100`
scale.

### Speed and chroma subsampling

- **`with_speed(s: u8)`** — `s` is `1..=10` where **`1` is the slowest preset
  with the best compression** and **`10` is the fastest with larger files**. The
  default is `5`. (Values outside the range are accepted: `> 10` behaves like the
  fastest preset, `0` like the slowest.)
- **`with_chroma_subsampling(mode)`** — `ChromaSubsampling::Yuv444` (the default)
  keeps full-resolution color; `ChromaSubsampling::Yuv420` halves chroma in both
  dimensions, cutting file size ~25–35% with minimal loss on photographic
  content (not recommended for text or sharp edges, and it cannot be combined
  with the `RGB` internal color model).

### With timeout

```rust
use zenravif::*;
use std::time::Duration;

# fn demo(pixels: &[RGBA8], width: usize, height: usize) {
let result = Encoder::new()
    .with_quality(80.0)
    .with_timeout(Duration::from_millis(500))
    .encode_rgba(Img::new(pixels, width, height));

match result {
    Ok(encoded) => { /* use encoded.avif_file */ }
    // `At<Error>`: borrow the inner error to match it.
    Err(e) if matches!(e.error(), Error::Cancelled) => { /* timed out */ }
    Err(e) => { /* other error — e.full_trace() shows where it failed */ }
}
# }
```

### Manual cancellation

Construct a token with `CancellationToken::new()`, clone it, and call `.cancel()`
from another thread to interrupt encoding (returns `Error::Cancelled`, wrapped in
`At<Error>`):

```rust
use zenravif::*;

# fn demo(pixels: &[RGBA8], width: usize, height: usize) {
let token = CancellationToken::new();
let token_for_thread = token.clone();
std::thread::spawn(move || {
    // …decide to abort…
    token_for_thread.cancel();
});

let _ = Encoder::new()
    .with_cancellation_token(token)
    .encode_rgba(Img::new(pixels, width, height));
# }
```

### Higher bit depth, HDR, and animation

- **Bit depth** — `with_bit_depth(BitDepth::Twelve)` (or `Ten`/`Eight`/`Auto`)
  sets the internal AV1 precision; `Auto` (the default) uses 10-bit, which works
  best even for 8-bit input.
- **HDR metadata** — `with_mastering_display(...)` (SMPTE ST 2086),
  `with_content_light(...)` (CEA-861.3), plus `with_color_primaries`,
  `with_transfer_characteristics`, and `with_pixel_range`.
- **Animation** — `encode_animation_rgb` / `encode_animation_rgba` for 8-bit
  sequences, and `encode_animation_rgb16` / `encode_animation_rgba16` for 16-bit.

## License

Dual-licensed: [AGPL-3.0](LICENSE-AGPL3) or [commercial](LICENSE-COMMERCIAL).

I've maintained and developed open-source image server software — and the 40+
library ecosystem it depends on — full-time since 2011. Fifteen years of
continual maintenance, backwards compatibility, support, and the (very rare)
security patch. That kind of stability requires sustainable funding, and
dual-licensing is how we make it work without venture capital or rug-pulls.
Support sustainable and secure software; swap patch tuesday for patch leap-year.

[Our open-source products](https://www.imazen.io/open-source)

**Your options:**

- **Startup license** — $1 if your company has under $1M revenue and fewer
  than 5 employees. [Get a key →](https://www.imazen.io/pricing)
- **Commercial subscription** — Governed by the Imazen Site-wide Subscription
  License v1.1 or later. Apache 2.0-like terms, no source-sharing requirement.
  Sliding scale by company size.
  [Pricing & 60-day free trial →](https://www.imazen.io/pricing)
- **AGPL v3** — Free and open. Share your source if you distribute.

See [LICENSE-COMMERCIAL](LICENSE-COMMERCIAL) for details.

Upstream code from [kornelski/cavif-rs](https://github.com/kornelski/cavif-rs) is licensed under Apache-2.0.
Our additions and improvements are dual-licensed (AGPL-3.0 or commercial) as above.

### Upstream Contribution

We are willing to release our improvements under the original Apache-2.0
license if upstream takes over maintenance of those improvements. We'd rather
contribute back than maintain a parallel codebase. Open an issue or reach out.
