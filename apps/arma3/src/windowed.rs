//! The interactive client: window, surface, free camera, overlay.

use std::error::Error;

use a3_input::{Dik, InputCode, actions};
use a3_platform::{App, Context, FrameTime};
use a3_render::{DrawList, Gpu, Renderer, WindowSurface};

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::engine::EngineContext;
use crate::keys::{self, Keybindings};
use crate::scene::{DebugScene, FpsCounter};

struct Graphics {
    gpu: Gpu,
    surface: WindowSurface,
    renderer: Renderer,
    adapter: String,
}

/// The windowed game client.
pub struct GameApp {
    engine: EngineContext,
    vsync: bool,
    graphics: Option<Graphics>,
    scene: DebugScene,
    draws: DrawList,
    fps: FpsCounter,
    presented: u64,
    keys_loading: Option<Receiver<anyhow::Result<Keybindings>>>,
    keys_description: String,
}

impl GameApp {
    pub fn new(engine: EngineContext, vsync: bool) -> GameApp {
        GameApp {
            engine,
            vsync,
            graphics: None,
            scene: DebugScene::new(),
            draws: DrawList::default(),
            fps: FpsCounter::default(),
            presented: 0,
            keys_loading: None,
            keys_description: "BUILT-IN".to_owned(),
        }
    }
}

impl GameApp {
    /// Swap in the keybindings once the background loader is done.
    fn poll_keybindings(&mut self) {
        let Some(rx) = &self.keys_loading else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("keybinding loader stopped")),
        };
        self.keys_loading = None;
        match result {
            Ok(k) => {
                log::info!("keybindings: {}", k.description);
                self.scene.actions = k.map;
                self.keys_description = k.description;
            }
            Err(e) => {
                log::error!("keybindings: {e:#}; keeping the built-in defaults");
                self.keys_description = "BUILT-IN (LOAD FAILED)".to_owned();
            }
        }
    }
}

impl App for GameApp {
    fn init(&mut self, cx: &mut Context) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (width, height) = cx.window_size();
        let (gpu, surface) = Gpu::for_window(cx.window().clone(), width, height, self.vsync)?;
        let adapter = gpu.adapter_name();
        log::info!(
            "renderer: {adapter}, surface {:?}, game dir {:?}",
            surface.format(),
            self.engine.game_dir
        );
        let mut renderer = Renderer::new(&gpu, surface.format());
        self.scene.load(&gpu, &mut renderer);
        if !self.engine.keys.is_empty() {
            self.keys_loading = Some(keys::load_in_background(self.engine.keys.clone()));
            self.keys_description = "LOADING...".to_owned();
        }
        self.graphics = Some(Graphics {
            gpu,
            surface,
            renderer,
            adapter,
        });
        Ok(())
    }

    fn frame(&mut self, cx: &mut Context, time: &FrameTime) {
        self.fps.frame(time.real_dt);
        self.poll_keybindings();
        let input = cx.input();
        if self
            .scene
            .actions
            .just_triggered(input, actions::INGAME_PAUSE)
        {
            cx.request_exit();
            return;
        }
        if !cx.is_cursor_captured() && input.was_pressed(InputCode::MouseButton(0)) {
            cx.set_cursor_captured(true);
        } else if cx.is_cursor_captured() && input.was_pressed(InputCode::Key(Dik::TAB)) {
            cx.set_cursor_captured(false);
        }
        let captured = cx.is_cursor_captured();
        self.scene.update(cx.input(), captured, time.dt);

        let Some(g) = self.graphics.as_mut() else {
            return;
        };
        let Some(frame) = g.surface.acquire(&g.gpu) else {
            return;
        };
        self.draws.clear();
        self.scene.draw(&mut self.draws);
        self.scene.overlay(
            &mut self.draws,
            &self.fps,
            &g.adapter,
            captured,
            &self.keys_description,
        );
        let view = frame.texture.create_view(&Default::default());
        g.renderer.render(
            &g.gpu,
            &view,
            g.surface.size(),
            &self.scene.camera,
            &self.draws,
            // Eye adaptation follows wall-clock time, also while the simulation is paused.
            time.real_dt as f32,
        );
        cx.window().pre_present_notify();
        g.gpu.queue.present(frame);
        self.presented += 1;
        if self.presented == 1 || self.presented % 600 == 0 {
            log::info!(
                "presented frame {} ({:.0} fps, {:.2} ms)",
                self.presented,
                self.fps.fps(),
                self.fps.frame_ms()
            );
        }
    }

    fn resized(&mut self, _cx: &mut Context, width: u32, height: u32) {
        if let Some(g) = self.graphics.as_mut() {
            g.surface.resize(&g.gpu, width, height);
        }
    }

    fn exiting(&mut self) {
        log::info!("exiting after {} presented frames", self.presented);
    }
}
