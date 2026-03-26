# zenravif

[![crates.io](https://img.shields.io/crates/v/zenravif.svg)](https://crates.io/crates/zenravif)
[![docs.rs](https://docs.rs/zenravif/badge.svg)](https://docs.rs/zenravif)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg?style=for-the-badge)](LICENSE-AGPL3)

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
