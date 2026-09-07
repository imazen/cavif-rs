#![cfg(all(feature = "threading", feature = "stop"))]

use almost_enough::{Stop, StopReason};
use std::sync::{Arc, Mutex};
use zenravif::{AnimFrame, AnimFrame16, AnimFrameRgba, AnimFrameRgba16, Encoder, Img, RGB8, RGBA8, StopToken};

struct ObservePool(Arc<Mutex<Vec<usize>>>);
impl Stop for ObservePool {
    fn check(&self) -> Result<(), StopReason> {
        // Conversion runs on the caller. Observe only worker-pool checks,
        // including the checks inside each color and alpha coding context.
        if rayon::current_thread_index().is_some() {
            let threads = rayon::current_num_threads();
            let mut seen = self.0.lock().unwrap();
            if !seen.contains(&threads) { seen.push(threads); }
        }
        Ok(())
    }
}

#[test]
fn animation_honors_threads_without_changing_bytes() {
    const WIDTH: usize = 65;
    const HEIGHT: usize = 67;
    let rgb8 = (0..WIDTH * HEIGHT).map(|i| RGB8::new((i * 17) as u8, (i * 31) as u8, (i * 7) as u8)).collect::<Vec<_>>();
    let rgba8 = rgb8.iter().map(|p| RGBA8::new(p.r, p.g, p.b, 128)).collect::<Vec<_>>();
    let rgb16 = rgb8.iter().map(|p| rgb::RGB::new(u16::from(p.r) * 4 + 1, u16::from(p.g) * 4 + 2, u16::from(p.b) * 4 + 3)).collect::<Vec<_>>();
    let rgba16 = rgb16.iter().map(|p| rgb::RGBA::new(p.r, p.g, p.b, 511)).collect::<Vec<_>>();
    let frames_rgb8 = [20, 30].map(|duration_ms| AnimFrame { rgb: Img::new(&rgb8, WIDTH, HEIGHT), duration_ms });
    let frames_rgba8 = [20, 30].map(|duration_ms| AnimFrameRgba { rgba: Img::new(&rgba8, WIDTH, HEIGHT), duration_ms });
    let frames_rgb16 = [20, 30].map(|duration_ms| AnimFrame16 { rgb: Img::new(&rgb16, WIDTH, HEIGHT), duration_ms });
    let frames_rgba16 = [20, 30].map(|duration_ms| AnimFrameRgba16 { rgba: Img::new(&rgba16, WIDTH, HEIGHT), duration_ms });
    let mut baseline = None;
    for threads in [Some(1), Some(2), Some(0), None] {
        let mut files = Vec::new();
        for format in 0..4 {
            let observed = Arc::new(Mutex::new(Vec::new()));
            let enc = Encoder::new().with_speed(10).with_num_threads(threads)
                .with_stop(StopToken::new(ObservePool(observed.clone())));
            let result = match format {
                0 => enc.encode_animation_rgb(&frames_rgb8),
                1 => enc.encode_animation_rgba(&frames_rgba8),
                2 => enc.encode_animation_rgb16(&frames_rgb16),
                _ => enc.encode_animation_rgba16(&frames_rgba16),
            }.unwrap();
            if let Some(threads) = threads {
                let threads = if threads == 0 { rayon::current_num_threads() } else { threads };
                assert_eq!(*observed.lock().unwrap(), vec![threads],
                    "format {format}: coding must use only the requested {threads}-thread pool");
            }
            files.push(result.avif_file);
        }
        if let Some(ref baseline) = baseline {
            assert_eq!(&files, baseline, "thread count must not change complete AVIF bytes");
        } else {
            baseline = Some(files);
        }
    }
}
