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
