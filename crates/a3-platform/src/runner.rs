//! The windowed main loop: winit event loop, frame clock, input and gamepad polling.

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use a3_input::{InputCode, InputState};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use crate::clock::{ClockConfig, FrameClock, FrameTime};
use crate::keymap;

/// Pixels of motion that one wheel "line" is worth for pixel-precise scroll devices.
const PIXELS_PER_WHEEL_LINE: f64 = 40.0;
/// Gamepad stick dead zone.
const STICK_DEAD_ZONE: f32 = 0.15;

/// Errors from the platform layer.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("could not create the window: {0}")]
    Window(#[from] winit::error::OsError),
    #[error("application failed: {0}")]
    App(Box<dyn Error + Send + Sync>),
}

/// How to create the main window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowConfig {
    pub title: String,
    /// Inner size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Windowed (`true`) or borderless fullscreen on the current monitor (`false`).
    pub windowed: bool,
    pub clock: ClockConfig,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            title: "a3-rust".to_owned(),
            width: 1280,
            height: 720,
            windowed: true,
            clock: ClockConfig::default(),
        }
    }
}

/// The application driven by [`run`].
pub trait App {
    /// Called once the window exists, before the first frame.
    fn init(&mut self, cx: &mut Context) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Simulate and render one frame. `cx.input()` holds this frame's input.
    fn frame(&mut self, cx: &mut Context, time: &FrameTime);

    /// The window's inner size changed (physical pixels, never zero).
    fn resized(&mut self, _cx: &mut Context, _width: u32, _height: u32) {}

    /// The loop is about to exit.
    fn exiting(&mut self) {}
}

/// What the platform hands to the [`App`] each call.
pub struct Context {
    window: Arc<Window>,
    input: InputState,
    clock: FrameClock,
    exit: bool,
    cursor_captured: bool,
}

impl Context {
    /// The main window (shareable, e.g. to create a render surface).
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Inner size of the window in physical pixels.
    pub fn window_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// Input state of the current frame.
    pub fn input(&self) -> &InputState {
        &self.input
    }

    /// The frame clock (time scale, pause).
    pub fn clock_mut(&mut self) -> &mut FrameClock {
        &mut self.clock
    }

    /// Leave the main loop after this frame.
    pub fn request_exit(&mut self) {
        self.exit = true;
    }

    /// Hide and lock the cursor for mouse look, or release it.
    pub fn set_cursor_captured(&mut self, captured: bool) {
        if captured == self.cursor_captured {
            return;
        }
        let result = if captured {
            self.window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            self.window.set_cursor_grab(CursorGrabMode::None)
        };
        if let Err(e) = result {
            log::warn!("cursor grab change failed: {e}");
        }
        self.window.set_cursor_visible(!captured);
        self.cursor_captured = captured;
    }

    pub fn is_cursor_captured(&self) -> bool {
        self.cursor_captured
    }
}

struct Runner<A: App> {
    config: WindowConfig,
    app: A,
    cx: Option<Context>,
    gilrs: Option<gilrs::Gilrs>,
    epoch: Instant,
    error: Option<PlatformError>,
}

/// Open the main window and run `app` until it requests exit or the window closes.
pub fn run<A: App>(config: WindowConfig, app: A) -> Result<(), PlatformError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let gilrs = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("gamepad support unavailable: {e}");
            None
        }
    };
    let mut runner = Runner {
        config,
        app,
        cx: None,
        gilrs,
        epoch: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut runner)?;
    match runner.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl<A: App> Runner<A> {
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<(), PlatformError> {
        let mut attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height));
        if !self.config.windowed {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs)?);
        let mut cx = Context {
            window,
            input: InputState::new(),
            clock: FrameClock::new(self.config.clock),
            exit: false,
            cursor_captured: false,
        };
        self.app.init(&mut cx).map_err(PlatformError::App)?;
        self.cx = Some(cx);
        Ok(())
    }

    fn poll_gamepads(&mut self) {
        let (Some(gilrs), Some(cx)) = (self.gilrs.as_mut(), self.cx.as_mut()) else {
            return;
        };
        while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
            match event {
                gilrs::EventType::ButtonChanged(button, value, _) => {
                    if let Some(g) = keymap::gamepad_button(button) {
                        cx.input.set_analog(InputCode::Gamepad(g), value);
                    }
                }
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    if let Some((neg, pos)) = keymap::gamepad_axis(axis) {
                        let (n, p) = keymap::split_axis(value, STICK_DEAD_ZONE);
                        cx.input.set_analog(InputCode::Gamepad(neg), n);
                        cx.input.set_analog(InputCode::Gamepad(pos), p);
                    }
                }
                gilrs::EventType::Disconnected => cx.input.release_all(),
                _ => {}
            }
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_gamepads();
        let Some(cx) = self.cx.as_mut() else { return };
        let now = Instant::now();
        let time = cx.clock.tick(now);
        cx.input.set_time(self.epoch.elapsed().as_secs_f64());
        self.app.frame(cx, &time);
        cx.input.end_frame();
        if cx.exit {
            event_loop.exit();
        }
    }
}

impl<A: App> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.cx.is_some() {
            return;
        }
        if let Err(e) = self.create(event_loop) {
            self.error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let now = self.now();
        let Some(cx) = self.cx.as_mut() else { return };
        cx.input.set_time(now);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.app.resized(cx, size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                cx.input.release_all();
                cx.set_cursor_captured(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let dik = match event.physical_key {
                    PhysicalKey::Code(code) => keymap::dik_from_keycode(code),
                    PhysicalKey::Unidentified(_) => None,
                };
                if let Some(dik) = dik {
                    let input = InputCode::Key(dik);
                    match event.state {
                        ElementState::Pressed => cx.input.press(input),
                        ElementState::Released => cx.input.release(input),
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(input) = keymap::mouse_button_code(button) {
                    match state {
                        ElementState::Pressed => cx.input.press(input),
                        ElementState::Released => cx.input.release(input),
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / PIXELS_PER_WHEEL_LINE) as f32,
                };
                cx.input.wheel(lines);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta }, Some(cx)) = (event, self.cx.as_mut()) {
            cx.input.mouse_motion(delta.0 as f32, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(cx) = &self.cx {
            cx.window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.app.exiting();
    }
}
