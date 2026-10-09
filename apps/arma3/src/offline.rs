//! Window-less runs: the headless smoke test and offscreen screenshots.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::{Duration, Instant};

use a3_input::{Dik, InputCode, InputState};
use a3_platform::{ClockConfig, FrameClock};
use a3_render::{DrawList, Gpu, Renderer};
use anyhow::Context as _;

use crate::engine::EngineContext;
use crate::scene::{DebugScene, FpsCounter};

/// Longest a screenshot waits for terrain tiles to finish streaming.
const STREAMING_TIMEOUT: Duration = Duration::from_secs(60);

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
/// Over a World, keeps rendering until the terrain tiles near the camera have streamed in.
pub fn screenshot(
    engine: &EngineContext,
    path: &Path,
    (width, height): (u32, u32),
    frames: u64,
    bench_frames: u64,
) -> anyhow::Result<()> {
    let gpu = Gpu::headless().context("no GPU adapter for offscreen rendering")?;
    let adapter = gpu.adapter_name();
    log::info!("offscreen renderer: {adapter}");
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut scene = DebugScene::new();
    scene.load(&gpu, &mut renderer);
    if let Some(world) = engine.load_world()? {
        let play = engine.play.then_some(engine.camera_mode);
        scene.load_world(
            &gpu,
            &mut renderer,
            world,
            engine.camera,
            &engine.environment,
            play,
        );
    }
    if let Some((vfs, spec, config)) = engine.load_model_vfs()? {
        scene.load_model(&gpu, &mut renderer, vfs, spec, config.as_deref());
    }
    let mut input = InputState::new();
    if engine.play {
        // An offscreen run has no keyboard, so a `--play` capture holds the forward key: the
        // Man is caught mid-stride rather than standing in his idle Move.
        input.press(InputCode::Key(Dik::W));
    }
    let mut fps = FpsCounter::default();
    let mut draws = DrawList::default();
    let start = Instant::now();
    let mut frame = 0;
    let image = loop {
        let started = Instant::now();
        scene.update(&input, false, FRAME.as_secs_f64());
        draws.clear();
        scene.draw(&mut draws);
        if !engine.hide_overlay {
            scene.overlay(&mut draws, &fps, &adapter, false, "BUILT-IN");
        }
        scene.prepare_render(&mut renderer, FRAME.as_secs_f32());
        scene.update_hud((width, height), FRAME.as_secs_f64());
        let image = renderer.render_to_image(
            &gpu,
            width,
            height,
            &scene.camera,
            &draws,
            FRAME.as_secs_f32(),
        )?;
        fps.frame(started.elapsed().as_secs_f64());
        frame += 1;
        let streaming =
            scene.terrain_stats().is_some_and(|s| s.pending_tiles > 0) || scene.models_loading();
        if frame >= frames.max(1) && (!streaming || start.elapsed() > STREAMING_TIMEOUT) {
            break image;
        }
    };
    log::info!(
        "rendered {frame} frames in {:.2?} ({:.1} ms last), terrain {:?}",
        start.elapsed(),
        fps.frame_ms(),
        scene.terrain_stats()
    );
    if bench_frames > 0 {
        bench(
            &gpu,
            &mut renderer,
            &mut scene,
            (width, height),
            bench_frames,
        );
    }
    write_png(path, width, height, &image)?;
    log::info!("wrote {}", path.display());
    Ok(())
}

/// Time `frames` frames rendered into an offscreen target (no readback) and print the average.
fn bench(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &mut DebugScene,
    size: (u32, u32),
    frames: u64,
) {
    use a3_render::wgpu;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench frame"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.output_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let input = InputState::new();
    let mut draws = DrawList::default();
    let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..frames {
        let started = Instant::now();
        scene.update(&input, false, FRAME.as_secs_f64());
        draws.clear();
        scene.draw(&mut draws);
        renderer.render(gpu, &view, size, &scene.camera, &draws, FRAME.as_secs_f32());
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let t = started.elapsed();
        total += t;
        worst = worst.max(t);
    }
    let avg = total / frames as u32;
    println!(
        "bench: {frames} frames at {}x{}, avg {:.2} ms ({:.0} fps), worst {:.2} ms",
        size.0,
        size.1,
        avg.as_secs_f64() * 1000.0,
        1.0 / avg.as_secs_f64(),
        worst.as_secs_f64() * 1000.0
    );
    if let Some(stats) = scene.model_stats() {
        println!("bench: {stats:?}");
    }
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
