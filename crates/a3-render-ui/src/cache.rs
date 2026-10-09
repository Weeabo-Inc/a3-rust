//! The UI texture cache: paths from the draw list to uploaded, bound textures.
//!
//! Slot 0 is a white 1x1 texture, the source of untextured (solid colour) quads. Every other
//! slot is one decoded texture, keyed by its normalized path; a path that failed to load is
//! remembered so a missing file is not re-read every frame.

use std::collections::{HashMap, HashSet};

use a3_render::{ColorSpace, GpuTexture, TextureData};
use log::warn;

use crate::assets::UiAssets;
use crate::decode::decode_ui_texture;
use crate::geometry::WHITE_SLOT;

/// What one texture key of the draw list resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureInfo {
    /// Bind group slot in [`TextureCache`] (`0` is the white texture).
    pub slot: u32,
    /// Size in pixels of the uploaded top mip (mip 0 for every PAA).
    pub size: (u32, u32),
}

struct Slot {
    group: wgpu::BindGroup,
    size: (u32, u32),
    _texture: GpuTexture,
}

/// Textures uploaded for the UI.
pub(crate) struct TextureCache {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: Vec<Slot>,
    by_path: HashMap<String, u32>,
    failed: HashSet<String>,
    bc_supported: bool,
}

impl TextureCache {
    /// Creates the (empty) cache with its white slot. `bc_supported` decides whether DXT
    /// textures stay block-compressed.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, bc_supported: bool) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // UI art is drawn at its own size or scaled up; linear filtering and clamping keep
        // glyph pages and icons crisp at the edges instead of wrapping in the neighbouring
        // glyph.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let mut cache = TextureCache {
            layout,
            sampler,
            slots: Vec::new(),
            by_path: HashMap::new(),
            failed: HashSet::new(),
            bc_supported,
        };
        cache.upload(
            device,
            queue,
            "ui white",
            &TextureData::solid_rgba8([255; 4]),
        );
        debug_assert_eq!(cache.slots[WHITE_SLOT as usize].size, (1, 1));
        cache
    }

    /// The bind group layout of group 1.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// The bind group of `slot`, for `set_bind_group(1, ..)`.
    pub fn group(&self, slot: u32) -> Option<&wgpu::BindGroup> {
        self.slots.get(slot as usize).map(|s| &s.group)
    }

    /// The info of `path`, decoding and uploading it on first use. `None` (and a remembered
    /// failure) when there is no such texture.
    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Option<&dyn UiAssets>,
        path: &str,
    ) -> Option<TextureInfo> {
        if let Some(&slot) = self.by_path.get(path) {
            return Some(TextureInfo {
                slot,
                size: self.slots[slot as usize].size,
            });
        }
        if self.failed.contains(path) {
            return None;
        }
        // Procedural textures generate themselves; files come from the asset source.
        let bytes = if path.starts_with('#') {
            None
        } else {
            match assets.and_then(|a| a.read(path)) {
                Some(bytes) => Some(bytes),
                None => {
                    warn!("ui texture not found: {path}");
                    self.failed.insert(path.to_owned());
                    return None;
                }
            }
        };
        match decode_ui_texture(path, bytes.as_deref(), self.bc_supported) {
            Ok(data) => {
                let size = (data.width, data.height);
                let slot = self.upload(device, queue, path, &data);
                self.by_path.insert(path.to_owned(), slot);
                Some(TextureInfo { slot, size })
            }
            Err(e) => {
                warn!("ui texture {path}: {e}");
                self.failed.insert(path.to_owned());
                None
            }
        }
    }

    /// Number of textures held (the white texture included).
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        data: &TextureData,
    ) -> u32 {
        let texture = GpuTexture::upload(device, queue, data, ColorSpace::Srgb, Some(label))
            .expect("ui texture data is validated by the decoder");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let slot = self.slots.len() as u32;
        self.slots.push(Slot {
            group,
            size: (data.width, data.height),
            _texture: texture,
        });
        slot
    }
}
