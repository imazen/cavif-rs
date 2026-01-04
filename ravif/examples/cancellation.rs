use ravif::*;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    println!("AVIF Encoder Cancellation and Timeout Example");
    println!("==============================================\n");

    // Create a reasonably large test image
    let width = 1024;
    let height = 1024;
    println!("Creating {}x{} test image...", width, height);

    let img: Vec<RGBA8> = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                RGBA8::new(
                    ((x ^ y) & 0xFF) as u8,
                    ((x + y) & 0xFF) as u8,
                    ((x * y / 256) & 0xFF) as u8,
                    255,
                )
            })
        })
        .collect();

    let img = imgref::Img::new(img.as_slice(), width, height);

    // Example 1: Encoding without cancellation (baseline)
    println!("\n1. Encoding without cancellation...");
    let start = Instant::now();
    let encoder = Encoder::new()
        .with_quality(70.0)
        .with_speed(1); // Slow speed to make cancellation more visible

    match encoder.encode_rgba(img) {
        Ok(result) => {
            println!("   ✓ Encoding completed in {:?}", start.elapsed());
            println!("   File size: {} bytes", result.avif_file.len());
        }
        Err(e) => println!("   ✗ Error: {:?}", e),
    }

    // Example 2: Pre-cancelled token
    println!("\n2. Encoding with pre-cancelled token...");
    let token = CancellationToken::new();
    token.cancel(); // Cancel immediately

    let encoder = Encoder::new()
        .with_quality(70.0)
        .with_speed(1)
        .with_cancellation_token(token);

    let start = Instant::now();
    match encoder.encode_rgba(img) {
        Ok(_) => println!("   ✗ Unexpectedly succeeded"),
        Err(Error::Cancelled) => {
            println!("   ✓ Encoding cancelled as expected in {:?}", start.elapsed());
        }
        Err(e) => println!("   ✗ Unexpected error: {:?}", e),
    }

    // Example 3: Cancellation during encoding (timeout pattern)
    println!("\n3. Encoding with timeout cancellation...");
    let token = CancellationToken::new();
    let token_clone = token.clone();

    // Spawn thread to cancel after 50ms
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        println!("   → Cancelling after 50ms...");
        token_clone.cancel();
    });

    let encoder = Encoder::new()
        .with_quality(70.0)
        .with_speed(1)
        .with_cancellation_token(token);

    let start = Instant::now();
    match encoder.encode_rgba(img) {
        Ok(_) => {
            println!(
                "   ⚠ Encoding completed before cancellation in {:?}",
                start.elapsed()
            );
        }
        Err(Error::Cancelled) => {
            println!("   ✓ Encoding cancelled after {:?}", start.elapsed());
        }
        Err(e) => println!("   ✗ Unexpected error: {:?}", e),
    }

    // Example 4: Reusing a cancellation token
    println!("\n4. Reusing cancellation token after reset...");
    let token = CancellationToken::new();
    token.cancel();
    println!("   Token cancelled: {}", token.is_cancelled());

    token.reset();
    println!("   Token reset: {}", token.is_cancelled());

    let encoder = Encoder::new()
        .with_quality(70.0)
        .with_speed(10) // Fast speed
        .with_cancellation_token(token);

    let start = Instant::now();
    match encoder.encode_rgba(img) {
        Ok(result) => {
            println!("   ✓ Encoding completed in {:?}", start.elapsed());
            println!("   File size: {} bytes", result.avif_file.len());
        }
        Err(e) => println!("   ✗ Error: {:?}", e),
    }

    // Example 5: Image proxy pattern - multiple requests with shared timeout
    println!("\n5. Image proxy pattern - processing multiple images with timeout...");

    let images = vec![
        (256, 256, "small"),
        (512, 512, "medium"),
        (1024, 1024, "large"),
    ];

    for (w, h, name) in images {
        println!("\n   Processing {} image ({}x{})...", name, w, h);

        let img_data: Vec<RGBA8> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| RGBA8::new((x & 0xFF) as u8, (y & 0xFF) as u8, 128, 255))
            })
            .collect();
        let img = imgref::Img::new(img_data.as_slice(), w, h);

        let token = CancellationToken::new();
        let token_clone = token.clone();

        // 100ms timeout per request (typical for image proxy)
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            token_clone.cancel();
        });

        let encoder = Encoder::new()
            .with_quality(70.0)
            .with_speed(5)
            .with_cancellation_token(token);

        let start = Instant::now();
        match encoder.encode_rgba(img) {
            Ok(result) => {
                println!(
                    "   ✓ {} completed in {:?} ({} bytes)",
                    name,
                    start.elapsed(),
                    result.avif_file.len()
                );
            }
            Err(Error::Cancelled) => {
                println!("   ⚠ {} cancelled after {:?}", name, start.elapsed());
            }
            Err(e) => println!("   ✗ {} error: {:?}", name, e),
        }

        // Small delay between requests
        thread::sleep(Duration::from_millis(50));
    }

    // Example 6: Built-in timeout (recommended for image proxies)
    println!("\n6. Built-in timeout - simplest approach for image proxies...");

    let images = vec![
        (128, 128, "tiny", Duration::from_secs(1)),
        (512, 512, "medium", Duration::from_millis(200)),
        (1024, 1024, "large", Duration::from_millis(100)),
    ];

    for (w, h, name, timeout) in images {
        println!("\n   Processing {} image ({}x{}) with {:?} timeout...",
            name, w, h, timeout);

        let img_data: Vec<RGBA8> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    RGBA8::new(
                        ((x ^ y) & 0xFF) as u8,
                        ((x + y) & 0xFF) as u8,
                        ((x * y / 256) & 0xFF) as u8,
                        255,
                    )
                })
            })
            .collect();
        let img = imgref::Img::new(img_data.as_slice(), w, h);

        let encoder = Encoder::new()
            .with_quality(70.0)
            .with_speed(5)
            .with_timeout(timeout);

        let start = Instant::now();
        match encoder.encode_rgba(img) {
            Ok(result) => {
                println!(
                    "   ✓ {} completed in {:?} ({} bytes)",
                    name,
                    start.elapsed(),
                    result.avif_file.len()
                );
            }
            Err(Error::Cancelled) => {
                println!("   ⚠ {} timed out after {:?}", name, start.elapsed());
            }
            Err(e) => println!("   ✗ {} error: {:?}", name, e),
        }
    }

    println!("\n✓ All examples completed!");
    println!("\nRecommendation for image proxies:");
    println!("  Use .with_timeout(Duration::from_millis(100-500))");
    println!("  - No thread spawning required");
    println!("  - Checked every 10ms for responsive cancellation");
    println!("  - Minimal overhead (~20-50ns per check)");
    println!("  - Works well with async runtimes");
}
