//! Device creation and the window surface.

use thiserror::Error;

use crate::texture::TextureError;

/// Errors from setting up or using the renderer.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(#[from] wgpu::RequestAdapterError),
    #[error("could not create the GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("could not create the window surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("the adapter cannot present to this surface")]
    SurfaceUnsupported,
    #[error(transparent)]
    Texture(#[from] TextureError),
    #[error("reading back the frame failed: {0}")]
    Readback(String),
}

/// The wgpu instance, adapter, device and queue.
#[derive(Debug)]
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// A device without a surface, for offscreen rendering and tests. Falls back to a software
    /// adapter (WARP, lavapipe) when no hardware adapter is available.
    pub fn headless() -> Result<Gpu, RenderError> {
        let instance = new_instance();
        let adapter = pollster::block_on(request_adapter(&instance, None))?;
        let (device, queue) = pollster::block_on(request_device(&adapter))?;
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// A device able to present to `window`, plus the configured surface.
    pub fn for_window(
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<(Gpu, WindowSurface), RenderError> {
        let instance = new_instance();
        let surface = instance.create_surface(window)?;
        let adapter = pollster::block_on(request_adapter(&instance, Some(&surface)))?;
        let (device, queue) = pollster::block_on(request_device(&adapter))?;
        let gpu = Gpu {
            instance,
            adapter,
            device,
            queue,
        };
        let surface = WindowSurface::new(&gpu, surface, width, height, vsync)?;
        Ok((gpu, surface))
    }

    /// Short description of the adapter, e.g. for the debug overlay.
    pub fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }
}

fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}

async fn request_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: surface,
        apply_limit_buckets: false,
    };
    match instance.request_adapter(&options).await {
        Ok(adapter) => Ok(adapter),
        Err(_) => {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..options
                })
                .await
        }
    }
}

async fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    // PAA textures are DXT; use them natively when the adapter can.
    let features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("a3-render device"),
            required_features: features,
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
}

/// The window's swap chain.
#[derive(Debug)]
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl WindowSurface {
    fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<WindowSurface, RenderError> {
        let mut config = surface
            .get_default_config(&gpu.adapter, width.max(1), height.max(1))
            .ok_or(RenderError::SurfaceUnsupported)?;
        let caps = surface.get_capabilities(&gpu.adapter);
        if let Some(srgb) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *srgb;
        }
        config.present_mode = present_mode(vsync);
        surface.configure(&gpu.device, &config);
        Ok(WindowSurface { surface, config })
    }

    /// Format of the swap-chain textures.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Reconfigure for a new window size (ignored when zero, e.g. minimised).
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    /// Switch vertical sync on or off.
    pub fn set_vsync(&mut self, gpu: &Gpu, vsync: bool) {
        self.config.present_mode = present_mode(vsync);
        self.surface.configure(&gpu.device, &self.config);
    }

    /// The next texture to draw into, or `None` if this frame should be skipped (window
    /// occluded, surface being reconfigured).
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.surface.configure(&gpu.device, &self.config);
                Some(t)
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                None
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                None
            }
        }
    }
}

fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}
