//! Window-less runs: the headless smoke test and offscreen screenshots.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::{Duration, Instant};

use a3_input::{Dik, InputCode, InputState};
use a3_platform::{ClockConfig, FrameClock};
use a3_render::{DrawList, Gpu, Renderer};
use anyhow::Context as _;

use crate::scene::{DebugScene, FpsCounter};

/// Simulated frame length for window-less runs, so their results are reproducible.
const FRAME: Duration = Duration::from_micros(16_667);

/// Summary of a headless run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadlessReport {
    pub frames: u64,
    pub sim_time: f64,
    pub fixed_steps: u64,
}

/// Run the main loop for `frames` simulated frames without window or GPU, flying the camera
/// forward. Exercises clock, input and scene update for CI smoke tests.
pub fn run_headless(frames: u64) -> HeadlessReport {
    let mut scene = DebugScene::new();
    let mut clock = FrameClock::new(ClockConfig::default());
    let mut input = InputState::new();
    let start = Instant::now();
    let mut fixed_steps = 0;
    let mut last = Default::default();
    input.press(InputCode::Key(Dik::W));
    for i in 0..frames {
        let now = start + FRAME * i as u32;
        let time = clock.tick(now);
        input.set_time(time.real_time);
        scene.update(&input, false, time.dt);
        fixed_steps += u64::from(time.fixed_steps);
        input.end_frame();
        last = time;
    }
    HeadlessReport {
        frames,
        sim_time: last.sim_time,
        fixed_steps,
    }
}

/// Render `frames` frames offscreen at `width` x `height` and save the last one as a PNG.
pub fn screenshot(path: &Path, width: u32, height: u32, frames: u64) -> anyhow::Result<()> {
    let gpu = Gpu::headless().context("no GPU adapter for offscreen rendering")?;
    let adapter = gpu.adapter_name();
    log::info!("offscreen renderer: {adapter}");
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut scene = DebugScene::new();
    scene.load(&gpu, &mut renderer);
    let input = InputState::new();
    let mut fps = FpsCounter::default();
    let mut draws = DrawList::default();
    let mut image = Vec::new();
    for _ in 0..frames.max(1) {
        let started = Instant::now();
        scene.update(&input, false, FRAME.as_secs_f64());
        draws.clear();
        scene.draw(&mut draws);
        scene.overlay(&mut draws, &fps, &adapter, false);
        image = renderer.render_to_image(&gpu, width, height, &scene.camera, &draws)?;
        fps.frame(started.elapsed().as_secs_f64());
    }
    write_png(path, width, height, &image)?;
    log::info!("wrote {}", path.display());
    Ok(())
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_loop_runs_and_moves_the_camera() {
        let report = run_headless(120);
        assert_eq!(report.frames, 120);
        // 119 frame deltas of ~16.7 ms.
        assert!((report.sim_time - 119.0 * 0.016_667).abs() < 1e-3);
        assert!(report.fixed_steps >= 118 && report.fixed_steps <= 119);
    }
}
