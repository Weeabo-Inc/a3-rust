//! GPU tests of texture residency: streaming, refinement, budget eviction, failures and use
//! through `MeshDraw`. Skip without an adapter.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use a3_render::residency::{LoadedMips, MipRequest, TextureInfo};
use a3_render::{
    AntiAliasing, BloomSettings, Camera, ColorSpace, DrawList, Gpu, HdrSettings, MeshData,
    MeshDraw, Renderer, ResidencyConfig, TextureFormat, TextureHandle, TextureResidency,
    TextureSource, wgpu,
};
use common::gpu;
use glam::{DVec3, Vec3};

/// Square RGBA8 textures named `"<size>/<r>,<g>,<b>"`, one colour on every mip; `"missing"`
/// fails. Records every request.
#[derive(Default)]
struct Synthetic {
    requests: Mutex<Vec<(String, MipRequest)>>,
}

impl TextureSource for Synthetic {
    fn load(&self, key: &str, request: MipRequest) -> Result<LoadedMips, String> {
        self.requests
            .lock()
            .unwrap()
            .push((key.to_owned(), request));
        let (size, rgb) = key.split_once('/').ok_or("missing")?;
        let size: u32 = size.parse().map_err(|_| "bad size")?;
        let rgb: Vec<u8> = rgb.split(',').map(|c| c.parse().unwrap()).collect();
        let info = TextureInfo {
            format: TextureFormat::Rgba8,
            width: size,
            height: size,
            mip_count: 32 - size.leading_zeros(),
        };
        let (first, end) = match request {
            MipRequest::Tail { max_size } => (info.tail_start(max_size), info.mip_count),
            MipRequest::Range { first, end } => (first, end),
        };
        let mips = (first..end)
            .map(|level| {
                let edge = (size >> level).max(1) as usize;
                [rgb[0], rgb[1], rgb[2], 255].repeat(edge * edge)
            })
            .collect();
        Ok(LoadedMips {
            info,
            first_mip: first,
            mips,
        })
    }
}

fn manager(config: ResidencyConfig) -> (TextureResidency, Arc<Synthetic>) {
    let source = Arc::new(Synthetic::default());
    (TextureResidency::new(source.clone(), config), source)
}

/// Run updates until `done` holds (loads arrive asynchronously).
fn settle(gpu: &Gpu, r: &mut TextureResidency, mut done: impl FnMut(&TextureResidency) -> bool) {
    // Up to ~20 s: software adapters in CI are slow.
    for _ in 0..4000 {
        r.update(&gpu.device, &gpu.queue, None);
        if done(r) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("residency did not settle: {:?}", r.stats());
}

fn bytes(size: u32, top: u32) -> u64 {
    TextureInfo {
        format: TextureFormat::Rgba8,
        width: size,
        height: size,
        mip_count: 32 - size.leading_zeros(),
    }
    .bytes_from(top)
}

#[test]
fn tail_loads_first_then_wanted_mips_refine_it() {
    let Some(gpu) = gpu() else { return };
    let (mut r, source) = manager(ResidencyConfig::default());
    let h = r.acquire("1024/0,255,0", ColorSpace::Srgb);
    settle(&gpu, &mut r, |r| r.resident_mip(&h).is_some());
    assert_eq!(
        r.resident_mip(&h),
        Some(4),
        "64 px tail of a 1024 px texture"
    );
    assert!(r.view(&h).is_some());

    settle(&gpu, &mut r, |r| {
        h.want_mip(1);
        r.resident_mip(&h) == Some(1)
    });
    let requests = source.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        vec![
            ("1024/0,255,0".into(), MipRequest::Tail { max_size: 64 }),
            (
                "1024/0,255,0".into(),
                MipRequest::Range { first: 1, end: 4 }
            ),
        ],
        "refinement loads only the missing mips"
    );
    assert_eq!(r.stats().resident_bytes, bytes(1024, 1));
}

#[test]
fn unreferenced_textures_are_evicted_least_recently_used_first_under_budget() {
    let Some(gpu) = gpu() else { return };
    // Room for one full 256 px texture plus a few tails.
    let (mut r, _) = manager(ResidencyConfig {
        budget_bytes: bytes(256, 0) + 4 * bytes(256, 2),
        ..ResidencyConfig::default()
    });
    let a = r.acquire("256/255,0,0", ColorSpace::Srgb);
    // Wants last one frame: renew them every frame, as a renderer does.
    settle(&gpu, &mut r, |r| {
        a.want_mip(0);
        r.resident_mip(&a) == Some(0)
    });
    drop(a);
    let b = r.acquire("256/0,0,255", ColorSpace::Srgb);
    for _ in 0..4000 {
        b.want_mip(0);
        r.update(&gpu.device, &gpu.queue, None);
        if r.resident_mip(&b) == Some(0) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.resident_mip(&b), Some(0));
    let stats = r.stats();
    assert_eq!(stats.evictions, 1, "{stats:?}");
    assert!(stats.resident_bytes <= r.config().budget_bytes, "{stats:?}");
    // The evicted texture loads again on demand.
    let again = r.acquire("256/255,0,0", ColorSpace::Srgb);
    settle(&gpu, &mut r, |r| r.resident_mip(&again).is_some());
}

#[test]
fn failed_textures_are_not_retried() {
    let Some(gpu) = gpu() else { return };
    let (mut r, source) = manager(ResidencyConfig::default());
    let h = r.acquire("missing", ColorSpace::Srgb);
    settle(&gpu, &mut r, |r| r.failed(&h));
    for _ in 0..5 {
        r.update(&gpu.device, &gpu.queue, None);
    }
    assert_eq!(source.requests.lock().unwrap().len(), 1);
    assert!(r.view(&h).is_none());
}

#[test]
fn uploads_per_frame_are_limited() {
    let Some(gpu) = gpu() else { return };
    let (mut r, _) = manager(ResidencyConfig {
        upload_bytes_per_frame: 1,
        ..ResidencyConfig::default()
    });
    let handles: Vec<TextureHandle> = (0..4)
        .map(|i| r.acquire(&format!("64/{i},0,0"), ColorSpace::Srgb))
        .collect();
    r.update(&gpu.device, &gpu.queue, None);
    // Let all four loads finish, then count how many become resident per update.
    std::thread::sleep(Duration::from_millis(200));
    let resident = |r: &TextureResidency| handles.iter().filter(|h| r.view(h).is_some()).count();
    let mut counts = Vec::new();
    for _ in 0..4 {
        r.update(&gpu.device, &gpu.queue, None);
        counts.push(resident(&r));
    }
    assert_eq!(
        counts,
        vec![1, 2, 3, 4],
        "one arrival per frame at a 1-byte budget"
    );
    assert_eq!(r.stats().uploaded_bytes, bytes(64, 0));
}

#[test]
fn acquiring_the_same_key_shares_the_texture() {
    let Some(gpu) = gpu() else { return };
    let (mut r, source) = manager(ResidencyConfig::default());
    let a = r.acquire("128/1,2,3", ColorSpace::Srgb);
    let b = r.acquire("128/1,2,3", ColorSpace::Srgb);
    assert_eq!(a.texture_id(), b.texture_id());
    settle(&gpu, &mut r, |r| r.view(&a).is_some());
    assert_eq!(source.requests.lock().unwrap().len(), 1);
    assert_eq!(r.stats().referenced, 1);
}

fn renderer_with_streaming(gpu: &Gpu) -> Renderer {
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.settings.hdr = HdrSettings {
        fixed_exposure: Some(1.0),
        anti_aliasing: AntiAliasing::None,
        bloom: BloomSettings {
            enabled: false,
            ..BloomSettings::default()
        },
        ..HdrSettings::default()
    };
    renderer.settings.shadows.enabled = false;
    renderer.enable_streaming(Arc::new(Synthetic::default()), ResidencyConfig::default());
    renderer
}

const SIZE: u32 = 64;

fn centre(image: &[u8]) -> [u8; 3] {
    let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [image[i], image[i + 1], image[i + 2]]
}

#[test]
fn mesh_draws_stream_their_textures_and_refine_by_screen_size() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = renderer_with_streaming(&gpu);
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let handle = renderer
        .residency_mut()
        .unwrap()
        .acquire("1024/0,255,0", ColorSpace::Srgb);
    let camera = Camera::default();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw {
        texture: Some(handle.texture_id()),
        ..MeshDraw::at(cube, DVec3::new(0.0, 0.0, 3.0), [1.0; 4])
    });
    let mut image = Vec::new();
    for _ in 0..4000 {
        image = renderer
            .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, 1.0 / 60.0)
            .unwrap();
        let resident = renderer.residency().unwrap().resident_mip(&handle);
        if resident.is_some_and(|m| m < 4) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let [r, g, b] = centre(&image);
    assert!(
        g > 100 && r < 40 && b < 40,
        "streamed green texture: {r} {g} {b}"
    );
    // The cube covers about 40 px of a 64 px frame: mips finer than the 64 px tail are needed.
    let resident = renderer.residency().unwrap().resident_mip(&handle);
    assert!(
        resident.is_some_and(|m| m < 4),
        "refined by screen size: {resident:?}"
    );
}

#[test]
fn removed_meshes_and_textures_go_stale() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = renderer_with_streaming(&gpu);
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let red = renderer
        .upload_texture(
            &gpu,
            &a3_render::TextureData::solid_rgba8([255, 0, 0, 255]),
            ColorSpace::Srgb,
        )
        .unwrap();
    let camera = Camera::default();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw {
        texture: Some(red),
        ..MeshDraw::at(cube, DVec3::new(0.0, 0.0, 3.0), [1.0; 4])
    });
    let shot = |renderer: &mut Renderer, draws: &DrawList| {
        centre(
            &renderer
                .render_to_image(&gpu, SIZE, SIZE, &camera, draws, 1.0 / 60.0)
                .unwrap(),
        )
    };
    let [r, g, _] = shot(&mut renderer, &draws);
    assert!(r > 100 && g < 40, "red texture");

    assert!(renderer.remove_texture(red));
    assert!(!renderer.remove_texture(red), "stale id");
    let [r, g, b] = shot(&mut renderer, &draws);
    assert!(
        r > 100 && g > 100 && b > 100,
        "removed texture draws white: {r} {g} {b}"
    );

    let (meshes, _) = renderer.resource_counts();
    assert!(renderer.remove_mesh(cube));
    // A new mesh may reuse the slot; the old id must not draw it.
    let other = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    assert_ne!(other, cube);
    assert_eq!(renderer.resource_counts().0, meshes);
    let [r, g, b] = shot(&mut renderer, &draws);
    assert!(b > r, "stale mesh is not drawn, sky shows: {r} {g} {b}");
}

/// Whether the frame's centre pixel is roughly `rgb`: each channel high for a bright one, low
/// for a dark one.
fn is_colour(image: &[u8], rgb: [u8; 3]) -> bool {
    centre(image)
        .iter()
        .zip(rgb)
        .all(|(&got, want)| if want > 128 { got > 100 } else { got < 40 })
}

#[test]
fn evicted_textures_reuse_their_slot_without_resurrecting_the_old_id() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = renderer_with_streaming(&gpu);
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let camera = Camera::default();
    let draws = |texture| {
        let mut draws = DrawList::default();
        draws.mesh(MeshDraw {
            texture: Some(texture),
            ..MeshDraw::at(cube, DVec3::new(0.0, 0.0, 3.0), [1.0; 4])
        });
        draws
    };
    let shot = |renderer: &mut Renderer, texture| {
        renderer
            .render_to_image(&gpu, SIZE, SIZE, &camera, &draws(texture), 1.0 / 60.0)
            .unwrap()
    };

    let red = renderer
        .residency_mut()
        .unwrap()
        .acquire("64/255,0,0", ColorSpace::Srgb);
    let red_id = red.texture_id();
    let mut pixels = shot(&mut renderer, red_id);
    for _ in 0..4000 {
        if is_colour(&pixels, [255, 0, 0]) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        pixels = shot(&mut renderer, red_id);
    }
    assert!(is_colour(&pixels, [255, 0, 0]), "streamed red texture");
    drop(red);

    // Released, the texture is evictable; with no budget it goes and frees its slot.
    renderer
        .residency_mut()
        .unwrap()
        .set_config(ResidencyConfig {
            budget_bytes: 0,
            ..ResidencyConfig::default()
        });
    let _ = shot(&mut renderer, red_id);
    assert_eq!(
        renderer.residency().unwrap().stats().evictions,
        1,
        "the released texture was evicted"
    );
    renderer
        .residency_mut()
        .unwrap()
        .set_config(ResidencyConfig::default());

    // The next texture takes the freed slot with a new generation.
    let green = renderer
        .residency_mut()
        .unwrap()
        .acquire("64/0,255,0", ColorSpace::Srgb);
    let green_id = green.texture_id();
    assert_ne!(green_id, red_id, "slot reused, generation bumped");
    for _ in 0..4000 {
        pixels = shot(&mut renderer, green_id);
        if is_colour(&pixels, [0, 255, 0]) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(is_colour(&pixels, [0, 255, 0]), "streamed green texture");

    // The id that pointed at the evicted texture draws white, not its slot's new occupant.
    let pixels = shot(&mut renderer, red_id);
    assert!(
        is_colour(&pixels, [255, 255, 255]),
        "stale streamed id draws white: {:?}",
        centre(&pixels)
    );
}

#[test]
fn real_paa_textures_stream() {
    use a3_render::PaaSource;
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some(gpu) = gpu() else { return };
    let vfs = Arc::new(a3_vfs::Vfs::new());
    vfs.mount_pbo_file(
        &std::path::Path::new(&root)
            .join("Addons")
            .join("data_f.pbo"),
    )
    .expect("data_f.pbo mounts");
    let paas: Vec<String> = vfs
        .walk("")
        .into_iter()
        .map(|p| p.to_string())
        .filter(|p| p.ends_with(".paa"))
        .take(40)
        .collect();
    assert!(!paas.is_empty());
    let reader_vfs = vfs.clone();
    let source = PaaSource::new(
        Arc::new(move |key: &str| reader_vfs.open(key).ok().map(|b| b.to_vec())),
        Renderer::supports_bc(&gpu),
    );
    let mut r = TextureResidency::new(Arc::new(source), ResidencyConfig::default());
    let handles: Vec<TextureHandle> = paas
        .iter()
        .map(|p| r.acquire(p, ColorSpace::Srgb))
        .collect();
    settle(&gpu, &mut r, |r| {
        handles.iter().all(|h| r.view(h).is_some() || r.failed(h))
    });
    let failed: Vec<&String> = paas
        .iter()
        .zip(&handles)
        .filter(|(_, h)| r.failed(h))
        .map(|(p, _)| p)
        .collect();
    assert!(failed.is_empty(), "failed: {failed:?}");
    // Refine the largest to full detail.
    let largest = handles
        .iter()
        .max_by_key(|h| r.info(h).map_or(0, |i| i.width * i.height))
        .unwrap();
    settle(&gpu, &mut r, |r| {
        largest.want_mip(0);
        r.resident_mip(largest) == Some(0)
    });
    eprintln!("{:?}", r.stats());
}
