//! Renders a small synthetic island offscreen and checks what lands where. Skips when no GPU
//! adapter (not even a software one) is available, unless `A3_REQUIRE_GPU` is set (CI).

use std::sync::{Mutex, MutexGuard, OnceLock};

use a3_landscape::{SeaWaves, WaterExPars};
use a3_landscape_render::detail::DetailLayers;
use a3_landscape_render::landscape::TerrainShading;
use a3_landscape_render::{
    HeightField, Landscape, NO_TILE, SeaConfig, SeaRenderer, TerrainRenderer, TileTable,
};
use a3_render::{Camera, DrawList, Gpu, Renderer, TextureData, WaterFog};
use a3_wrp::{Grid, GridSize};
use glam::{DVec3, Vec3};

const W: u32 = 96;
const H: u32 = 64;

/// One device shared by the tests of this binary, used in turns.
fn gpu() -> Option<MutexGuard<'static, Gpu>> {
    static GPU: OnceLock<Option<Mutex<Gpu>>> = OnceLock::new();
    let require = std::env::var_os("A3_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
    let gpu = GPU.get_or_init(|| match Gpu::headless() {
        Ok(gpu) => {
            eprintln!("adapter: {}", gpu.adapter_name());
            Some(Mutex::new(gpu))
        }
        Err(e) if require => panic!("A3_REQUIRE_GPU is set but no GPU adapter exists: {e}"),
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            None
        }
    });
    gpu.as_ref()
        .map(|m| m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// 128 cells of 10 m (1280 m): a round island 60 m high in the middle, sea floor at -60 m.
fn island() -> Landscape {
    let size = 128u32;
    let heights = (0..size * size)
        .map(|k| {
            let (x, z) = ((k % size) as f32 - 64.0, (k / size) as f32 - 64.0);
            let r = (x * x + z * z).sqrt();
            if r < 30.0 {
                60.0 * (1.0 - r / 30.0) + 2.0
            } else {
                -60.0
            }
        })
        .collect();
    let land = GridSize {
        width: 32,
        height: 32,
    };
    Landscape {
        heights: HeightField {
            size,
            cell: 10.0,
            heights,
        },
        land_cell: 40.0,
        land_cells: 32,
        world_size: 1280.0,
        tiles: TileTable {
            grid: None,
            tiles: Vec::new(),
            cell_tiles: Grid::filled(land, NO_TILE),
            material_tiles: Vec::new(),
        },
        // Bright green land, so terrain pixels are easy to tell from sea and sky.
        overview: Some(TextureData::solid_rgba8([40, 220, 40, 255])),
        material_indices: Grid::filled(land, 0),
        detail: DetailLayers::default(),
        shading: TerrainShading::default(),
    }
}

fn render(camera: &Camera) -> Option<Vec<u8>> {
    let gpu = gpu()?;
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.settings.water = Some(WaterFog {
        height: 0.0,
        density: 0.07,
        color: Vec3::new(0.03, 0.07, 0.09),
        gradient: Vec3::new(0.35, 1.0, 1.7),
        light_extinction: Vec3::new(0.1814, 0.0159, 0.0111),
        diffuse_extinction: Vec3::new(0.3814, 0.2159, 0.2111),
    });
    // The sky as the game draws it: in the scene, so the sea refracts it.
    let (sky, _) = a3_render::sky::SkyFeature::new(&gpu, &renderer, None);
    renderer.add_feature(Box::new(sky));
    renderer.settings.procedural_sky = false;
    let island = island();
    let terrain = TerrainRenderer::new(&gpu, &renderer, &island, None);
    let stats = terrain.stats();
    renderer.add_feature(Box::new(terrain));
    // Altis' water: dense blue-green fog under the surface.
    let water = WaterExPars {
        fog_density: Some(0.07),
        fog_color: Some(Vec3::new(0.03, 0.07, 0.09)),
        fog_gradient_coefs: Some(Vec3::new(0.35, 1.0, 1.7)),
        refraction_min_coef: Some(0.03),
        refraction_max_coef: Some(0.14),
        refraction_max_dist: Some(5.1),
        ..WaterExPars::default()
    };
    let config = SeaConfig::new(SeaWaves::default(), water);
    let (sea, params) = SeaRenderer::new(
        &gpu,
        &renderer,
        &island.heights,
        island.world_size,
        config,
        None,
    );
    {
        let mut p = params.lock().unwrap();
        p.sea_level = 0.0;
        p.waves = 0.5;
        p.view_distance = 3000.0;
    }
    let sea_stats = sea.stats();
    renderer.add_feature(Box::new(sea));
    let image = renderer
        .render_to_image(&gpu, W, H, camera, &DrawList::default(), 1.0 / 60.0)
        .expect("render");
    if let Some(path) = std::env::var_os("A3_DUMP_PPM") {
        let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
        ppm.extend(image.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
        let name = format!("{}_{:.0}.ppm", path.to_string_lossy(), camera.pitch * 100.0);
        std::fs::write(name, ppm).unwrap();
    }
    let looks_down = camera.pitch < 0.0;
    if looks_down {
        assert!(stats.lock().unwrap().nodes > 0, "terrain drawn");
    }
    let patches: u32 = sea_stats.lock().unwrap().patches.iter().sum();
    assert!(patches > 0, "sea patches drawn");
    Some(image)
}

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2]]
}

fn is_land([r, g, b]: [u8; 3]) -> bool {
    g > r + 20 && g > b + 20
}

#[test]
fn island_seen_from_the_south_shows_land_sea_and_sky() {
    // South of the island, 120 m up, looking north and slightly down at its peak.
    let camera = Camera {
        position: DVec3::new(640.0, 120.0, 200.0),
        pitch: -0.2,
        ..Camera::default()
    };
    let Some(image) = render(&camera) else { return };
    let centre = pixel(&image, W / 2, H / 2);
    assert!(is_land(centre), "the island is straight ahead: {centre:?}");
    // Bottom left, beside the island's shore.
    let bottom = pixel(&image, 2, H - 2);
    assert!(
        !is_land(bottom) && bottom[2] > bottom[0] && bottom[1] > bottom[0],
        "blue-green sea between camera and island: {bottom:?}"
    );
    let top = pixel(&image, W / 2, 1);
    assert!(top[2] > 150, "sky above: {top:?}");
}

#[test]
fn looking_down_from_above_the_peak_sees_lit_land() {
    let camera = Camera {
        position: DVec3::new(645.0, 400.0, 645.0),
        pitch: -1.5,
        ..Camera::default()
    };
    let Some(image) = render(&camera) else { return };
    let centre = pixel(&image, W / 2, H / 2);
    assert!(
        is_land(centre),
        "terrain faces the camera (winding) and is green: {centre:?}"
    );
}

#[test]
fn a_camera_under_the_sea_sees_only_water_fog() {
    // 5 m under the surface south of the island, looking north at it, 440 m away.
    let camera = Camera {
        position: DVec3::new(640.0, -5.0, 200.0),
        ..Camera::default()
    };
    let Some(image) = render(&camera) else { return };
    let centre = pixel(&image, W / 2, H / 2);
    assert!(
        !is_land(centre) && centre[2] > centre[0] && centre[1] > centre[0],
        "the island is lost in blue-green water fog: {centre:?}"
    );
    // Looking up at the surface: steeply up, the sky shows through it (Snell's window);
    // towards the horizon the surface reflects the dark water below (total internal
    // reflection).
    let up = Camera {
        position: DVec3::new(640.0, -5.0, 200.0),
        pitch: 0.9,
        ..Camera::default()
    };
    let Some(image) = render(&up) else { return };
    let sum = |p: [u8; 3]| p.iter().map(|&c| u32::from(c)).sum::<u32>();
    let window = pixel(&image, W / 2, 1);
    let mirror = pixel(&image, W / 2, H - 2);
    assert!(
        sum(window) > sum(mirror) + 60,
        "the sky through the surface ({window:?}) is brighter than the reflected deep ({mirror:?})"
    );
}
