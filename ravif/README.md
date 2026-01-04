# `ravif` — Pure Rust library for AVIF image encoding

Encoder for AVIF images. Based on [`rav1e`](https://lib.rs/crates/rav1e) and [`avif-serialize`](https://lib.rs/crates/avif-serialize).

The API is just a single `encode_rgba()` function call that spits an AVIF image.

This library powers the [`cavif`](https://lib.rs/crates/cavif) encoder. It has an encoding configuration specifically tuned for still images, and gives better quality/performance than stock `rav1e`.

## Features

- **Built-in Timeout**: Simple `with_timeout()` method with 10ms granularity - perfect for image proxies
- **Cancellation Support**: Thread-safe `CancellationToken` for manual control from other threads
- **Responsive**: Timeout/cancellation checked every 10ms with minimal overhead (~20-50ns per check)
- **Quality Control**: Configurable quality (1-100) for both color and alpha channels
- **Speed Presets**: 1 (slowest/best) to 10 (fastest)
- **Flexible Color Models**: YCbCr (default, best compression) or RGB
- **Alpha Channel**: Multiple alpha handling modes including premultiplied alpha

## Cancellation and Timeout

### Built-in Timeout

The simplest way to limit encoding time:

```rust
use ravif::*;
use std::time::Duration;

let img = /* your RGBA8 image data */;

let encoder = Encoder::new()
    .with_quality(70.0)
    .with_speed(5)
    .with_timeout(Duration::from_millis(100)); // Cancel after 100ms

match encoder.encode_rgba(img) {
    Ok(result) => println!("Encoded: {} bytes", result.avif_file.len()),
    Err(Error::Cancelled) => println!("Encoding timed out"),
    Err(e) => eprintln!("Error: {:?}", e),
}
```

### Manual Cancellation Token

For cancelling from another thread:

```rust
use ravif::*;
use std::thread;
use std::time::Duration;

let img = /* your RGBA8 image data */;
let token = CancellationToken::new();
let token_clone = token.clone();

// Cancel encoding from another thread
thread::spawn(move || {
    thread::sleep(Duration::from_millis(100));
    token_clone.cancel();
});

let encoder = Encoder::new()
    .with_quality(70.0)
    .with_speed(5)
    .with_cancellation_token(token);

match encoder.encode_rgba(img) {
    Ok(result) => println!("Encoded: {} bytes", result.avif_file.len()),
    Err(Error::Cancelled) => println!("Encoding was cancelled"),
    Err(e) => eprintln!("Error: {:?}", e),
}
```

### Combined

You can use both timeout and cancellation token together:

```rust
let encoder = Encoder::new()
    .with_timeout(Duration::from_secs(1))      // Timeout after 1 second
    .with_cancellation_token(token);            // OR cancel via token
```

See `examples/cancellation.rs` for more usage patterns.
