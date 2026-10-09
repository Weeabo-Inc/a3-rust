//! Shared GPU for the integration tests of one test binary.
//!
//! Tests run on parallel threads; creating one device per test is slow and some drivers crash
//! when several devices work concurrently. All tests share one lazily created device and take
//! turns through a mutex.
//!
//! Without an adapter the tests skip, unless `A3_REQUIRE_GPU` is set (CI sets it and provides
//! a software adapter), in which case they fail.

use std::sync::{Mutex, MutexGuard, OnceLock};

use a3_render::Gpu;

static GPU: OnceLock<Option<Mutex<Gpu>>> = OnceLock::new();

/// The shared GPU, or `None` (with a printed note) when no adapter exists.
pub fn gpu() -> Option<MutexGuard<'static, Gpu>> {
    let gpu = GPU.get_or_init(|| match Gpu::headless() {
        Ok(gpu) => {
            eprintln!("adapter: {}", gpu.adapter_name());
            Some(Mutex::new(gpu))
        }
        Err(e) if require_gpu() => panic!("A3_REQUIRE_GPU is set but no GPU adapter exists: {e}"),
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            None
        }
    });
    // A test that panicked while holding the lock leaves the device usable.
    gpu.as_ref()
        .map(|m| m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

fn require_gpu() -> bool {
    std::env::var_os("A3_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The RGBA pixel at `(x, y)`, top row first.
pub fn pixel(image: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

/// Writes a rendered frame as a PNG when `A3_UI_SCREENSHOT` names a file, for eyeballing a
/// change; the tests themselves never depend on the file.
pub fn dump_png(image: &[u8], width: u32, height: u32) {
    let Some(path) = std::env::var_os("A3_UI_SCREENSHOT") else {
        return;
    };
    let file = std::fs::File::create(&path).expect("screenshot file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(image).expect("png data");
    writer.finish().expect("png trailer");
    eprintln!("wrote {}", std::path::Path::new(&path).display());
}
