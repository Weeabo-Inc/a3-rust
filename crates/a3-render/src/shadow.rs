//! Cascaded shadow maps for the sun: cascade layout math and GPU resources.
//!
//! Defaults follow `CfgVideoOptions >> ShadowQuality` ("High": 2048 texels, 4 cascades) and the
//! profile's `shadowZDistance`.

use glam::{DVec3, Mat4, Vec3};

use crate::camera::Camera;

/// Most cascades the shaders support (RV's "Extreme" preset uses 8 layers).
pub const MAX_CASCADES: usize = 4;

/// Sun shadow settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Cascade count, 1..=[`MAX_CASCADES`] (`cascadeLayers`).
    pub cascades: u32,
    /// Edge of each cascade's square shadow map in texels (`textureSize`).
    pub map_size: u32,
    /// Distance from the camera up to which shadows are drawn, metres (`shadowZDistance`).
    pub distance: f32,
    /// Blend between uniform (0) and logarithmic (1) cascade splits.
    pub split_lambda: f32,
    /// How far behind a cascade's bounds (towards the sun) casters are still captured, metres.
    pub caster_extension: f32,
    /// Receiver offset along the surface normal, in shadow-map texels, against acne.
    pub normal_bias: f32,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        ShadowSettings {
            enabled: true,
            cascades: 4,
            map_size: 2048,
            distance: 150.0,
            split_lambda: 0.75,
            caster_extension: 300.0,
            normal_bias: 1.5,
        }
    }
}

/// Far distances (along the view direction) of each cascade, mixing uniform and logarithmic
/// splits between `near` and `far`.
pub fn cascade_splits(near: f32, far: f32, count: u32, lambda: f32) -> Vec<f32> {
    let count = count.max(1);
    // The logarithmic split is undefined for a zero near plane.
    let log_near = near.max(1e-3);
    (1..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let log = log_near * (far / log_near).powf(t);
            let uniform = near + (far - near) * t;
            lambda * log + (1.0 - lambda) * uniform
        })
        .collect()
}

/// Smallest sphere around the view-frustum slice between view depths `near` and `far`, as
/// (distance of its centre along the view direction, radius). Rotation invariant, so the
/// cascade does not change size as the camera turns.
pub fn slice_bounding_sphere(near: f32, far: f32, tan_half_h: f32, tan_half_v: f32) -> (f32, f32) {
    let k2 = tan_half_h * tan_half_h + tan_half_v * tan_half_v;
    // Equidistant from the near and far corners, unless that lies past the far plane.
    let centre = ((far + near) * (1.0 + k2) * 0.5).min(far);
    let radius = ((far - centre).powi(2) + far * far * k2).sqrt();
    (centre, radius)
}

/// One cascade: light view-projection for camera-relative positions and its far split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cascade {
    pub view_projection: Mat4,
    pub far: f32,
    /// World-space size of one shadow-map texel.
    pub texel_size: f32,
}

/// Light basis for a sun direction: (right, up, forward) with forward pointing away from the
/// sun (the light's view direction).
fn light_basis(towards_sun: DVec3) -> (DVec3, DVec3, DVec3) {
    let forward = -towards_sun.normalize();
    let helper = if forward.y.abs() > 0.99 {
        DVec3::Z
    } else {
        DVec3::Y
    };
    let right = helper.cross(forward).normalize();
    let up = forward.cross(right);
    (right, up, forward)
}

/// Compute the cascades for `camera` with viewport `aspect`. `towards_sun` points at the sun.
pub fn compute_cascades(
    camera: &Camera,
    aspect: f32,
    towards_sun: Vec3,
    settings: &ShadowSettings,
) -> Vec<Cascade> {
    let count = settings.cascades.clamp(1, MAX_CASCADES as u32);
    let splits = cascade_splits(camera.near, settings.distance, count, settings.split_lambda);
    let (right, up, forward) = light_basis(towards_sun.as_dvec3());
    let view_dir = camera.forward().as_dvec3();
    let tan_v = camera.fov.top;
    let tan_h = camera.fov.left(aspect);
    let map_size = settings.map_size.max(1) as f64;

    let mut near = camera.near;
    splits
        .into_iter()
        .map(|far| {
            let (centre_depth, radius) = slice_bounding_sphere(near, far, tan_h, tan_v);
            near = far;
            let radius = f64::from(radius);
            let texel = 2.0 * radius / map_size;
            // Snap the centre to whole texels in the light's plane, in world space, so the
            // cascade only moves in texel steps and edges do not shimmer.
            let world_centre = camera.position + view_dir * f64::from(centre_depth);
            let x = world_centre.dot(right);
            let y = world_centre.dot(up);
            let snapped = world_centre
                + right * ((x / texel).round() * texel - x)
                + up * ((y / texel).round() * texel - y);
            let centre = (snapped - camera.position).as_vec3();

            let depth = radius + f64::from(settings.caster_extension);
            let eye = centre - (forward * depth).as_vec3();
            let view = glam::camera::lh::view::look_to_mat4(eye, forward.as_vec3(), up.as_vec3());
            let r = radius as f32;
            let projection = glam::camera::lh::proj::directx::orthographic(
                -r,
                r,
                -r,
                r,
                0.0,
                (depth + radius) as f32,
            );
            Cascade {
                view_projection: projection * view,
                far,
                texel_size: texel as f32,
            }
        })
        .collect()
}

/// Depth format of the shadow maps (standard depth: cleared to 1, nearer is smaller).
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Shadow data for shaders (group 0, binding 3), see `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowUniforms {
    cascades: [[[f32; 4]; 4]; MAX_CASCADES],
    /// Far split of each cascade along the view direction.
    splits: [f32; 4],
    /// xyz: camera forward; w: active cascade count (0 = shadows off).
    view_forward: [f32; 4],
    /// World size of one texel per cascade.
    texel_sizes: [f32; 4],
    /// x: normal bias in texels, y: map size, z: 1 / map size, w: fade start (fraction of the
    /// last split).
    params: [f32; 4],
}

impl ShadowUniforms {
    pub fn new(cascades: &[Cascade], camera: &Camera, settings: &ShadowSettings) -> Self {
        let mut u = ShadowUniforms {
            cascades: [[[0.0; 4]; 4]; MAX_CASCADES],
            splits: [0.0; 4],
            view_forward: camera.forward().extend(cascades.len() as f32).to_array(),
            texel_sizes: [0.0; 4],
            params: [
                settings.normal_bias,
                settings.map_size as f32,
                1.0 / settings.map_size.max(1) as f32,
                0.9,
            ],
        };
        for (i, c) in cascades.iter().take(MAX_CASCADES).enumerate() {
            u.cascades[i] = c.view_projection.to_cols_array_2d();
            u.splits[i] = c.far;
            u.texel_sizes[i] = c.texel_size;
        }
        u
    }
}

/// Shadow-map textures and per-cascade render state.
pub(crate) struct ShadowMaps {
    /// (cascades, map size) the textures were created for.
    pub shape: (u32, u32),
    pub array_view: wgpu::TextureView,
    pub layer_views: Vec<wgpu::TextureView>,
    /// 1x1 stand-in bound while the real maps are render targets.
    pub dummy_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub uniforms: wgpu::Buffer,
}

impl ShadowMaps {
    pub fn new(device: &wgpu::Device, cascades: u32, map_size: u32) -> ShadowMaps {
        let cascades = cascades.clamp(1, MAX_CASCADES as u32);
        let map_size = map_size.clamp(16, device.limits().max_texture_dimension_2d);
        let texture = |label, size, layers| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SHADOW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let array_view_of = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let maps = texture("shadow maps", map_size, cascades);
        let layer_views = (0..cascades)
            .map(|layer| {
                maps.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let dummy = texture("shadow dummy", 1, 1);
        ShadowMaps {
            shape: (cascades, map_size),
            array_view: array_view_of(&maps),
            layer_views,
            dummy_view: array_view_of(&dummy),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("shadow compare"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            }),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shadow uniforms"),
                size: std::mem::size_of::<ShadowUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }
}

/// Layout entries of the shadow bindings in the frame bind group (bindings 1..=3).
pub(crate) fn frame_layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
    [
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: vf,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: vf,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: vf,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_end_at_the_shadow_distance_and_grow() {
        let s = cascade_splits(0.1, 150.0, 4, 0.75);
        assert_eq!(s.len(), 4);
        assert!((s[3] - 150.0).abs() < 1e-3);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
        // Logarithmic weighting keeps the first cascade small.
        assert!(s[0] < 15.0, "{s:?}");
    }

    #[test]
    fn uniform_splits_with_lambda_zero() {
        let s = cascade_splits(0.0, 100.0, 4, 0.0);
        assert_eq!(s, vec![25.0, 50.0, 75.0, 100.0]);
    }

    #[test]
    fn slice_sphere_contains_all_corners() {
        let (tan_h, tan_v) = (1.333, 0.75);
        for (near, far) in [(0.1, 5.0), (5.0, 20.0), (20.0, 150.0)] {
            let (c, r) = slice_bounding_sphere(near, far, tan_h, tan_v);
            for d in [near, far] {
                for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    let corner = Vec3::new(sx * d * tan_h, sy * d * tan_v, d);
                    let dist = (corner - Vec3::new(0.0, 0.0, c)).length();
                    assert!(dist <= r * 1.0001, "corner {corner} outside ({c}, {r})");
                }
            }
        }
    }

    fn camera_at(position: DVec3) -> Camera {
        Camera {
            position,
            yaw: 0.3,
            pitch: -0.2,
            ..Camera::default()
        }
    }

    #[test]
    fn cascade_covers_the_points_it_should_shadow() {
        let camera = camera_at(DVec3::new(20_000.0, 30.0, 20_000.0));
        let settings = ShadowSettings::default();
        let sun = Vec3::new(0.3, 0.8, -0.5).normalize();
        let cascades = compute_cascades(&camera, 16.0 / 9.0, sun, &settings);
        // A point in the view 10 m ahead must land inside the first cascade that covers 10 m.
        let point = camera.forward() * 10.0;
        let cascade = cascades.iter().find(|c| c.far >= 10.0).unwrap();
        let clip = cascade.view_projection * point.extend(1.0);
        assert!(clip.x.abs() < 1.0 && clip.y.abs() < 1.0, "{clip}");
        assert!(clip.z > 0.0 && clip.z < 1.0, "{clip}");
        // A caster 100 m towards the sun from that point is still in the depth range.
        let caster = point + sun * 100.0;
        let clip = cascade.view_projection * caster.extend(1.0);
        assert!(
            clip.z > 0.0 && clip.z < clip_depth(cascade, point),
            "{clip}"
        );
    }

    fn clip_depth(cascade: &Cascade, p: Vec3) -> f32 {
        (cascade.view_projection * p.extend(1.0)).z
    }

    #[test]
    fn cascades_move_in_whole_texels() {
        let settings = ShadowSettings::default();
        let sun = Vec3::new(0.3, 0.8, -0.5).normalize();
        let a = camera_at(DVec3::new(20_000.0, 30.0, 20_000.0));
        let b = camera_at(a.position + DVec3::new(0.013, 0.0, 0.007));
        let ca = compute_cascades(&a, 1.5, sun, &settings);
        let cb = compute_cascades(&b, 1.5, sun, &settings);
        // A fixed world point maps to texel positions that differ by whole texels.
        let world = a.position + DVec3::new(3.0, -30.0, 8.0);
        let texel_coord = |c: &Cascade, cam: &Camera| {
            let clip = c.view_projection * cam.relative(world).extend(1.0);
            (clip.truncate().truncate() * 0.5 * settings.map_size as f32).to_array()
        };
        let (pa, pb) = (texel_coord(&ca[0], &a), texel_coord(&cb[0], &b));
        for i in 0..2 {
            let delta = pa[i] - pb[i];
            assert!((delta - delta.round()).abs() < 0.02, "texel delta {delta}");
        }
    }

    #[test]
    fn overhead_sun_has_a_valid_basis() {
        let (r, u, f) = light_basis(DVec3::Y);
        assert!((r.length() - 1.0).abs() < 1e-9 && r.dot(f).abs() < 1e-9 && u.dot(f).abs() < 1e-9);
    }
}
