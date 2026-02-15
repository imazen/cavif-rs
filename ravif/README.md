# zenravif

[![crates.io](https://img.shields.io/crates/v/zenravif.svg)](https://crates.io/crates/zenravif)
[![docs.rs](https://docs.rs/zenravif/badge.svg)](https://docs.rs/zenravif)
[![license](https://img.shields.io/crates/l/zenravif.svg)](LICENSE)

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
zenravif = "0.1"
```

### Still image

```rust
use zenravif::*;

let pixels: &[RGBA8] = &[/* your pixel data */];
let img = Img::new(pixels, width, height);

let result = Encoder::new()
    .with_quality(70.0)
    .with_speed(4)
    .encode_rgba(img)?;

std::fs::write("output.avif", &result.avif_file)?;
```

### With timeout

```rust
use zenravif::*;
use std::time::Duration;

let result = Encoder::new()
    .with_quality(70.0)
    .with_timeout(Duration::from_millis(500))
    .encode_rgba(img);

match result {
    Ok(encoded) => { /* use encoded.avif_file */ }
    Err(Error::Cancelled) => { /* timed out */ }
    Err(e) => { /* other error */ }
}
```

## License

BSD-3-Clause. Original code copyright Cloudflare, Inc. Fork additions copyright Imazen LLC.
