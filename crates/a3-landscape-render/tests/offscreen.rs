//! Renders a small synthetic island offscreen and checks what lands where. Skips when no GPU
//! adapter (not even a software one) is available, unless `A3_REQUIRE_GPU` is set (CI).

use std::sync::{Mutex, MutexGuard, OnceLock};

use a3_landscape_render::detail::DetailLayers;
use a3_landscape_render::landscape::TerrainShading;
use a3_landscape_render::{HeightField, Landscape, NO_TILE, TerrainRenderer, TileTable};
use a3_render::{Camera, DrawList, Gpu, Renderer, TextureData};
use a3_wrp::{Grid, GridSize};
use glam::DVec3;

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

/// 128 cells of 10 m (1280 m): a round island 60 m high in the middle, sea floor at -20 m.
fn island() -> Landscape {
    let size = 128u32;
    let heights = (0..size * size)
        .map(|k| {
            let (x, z) = ((k % size) as f32 - 64.0, (k / size) as f32 - 64.0);
            let r = (x * x + z * z).sqrt();
            if r < 30.0 {
                60.0 * (1.0 - r / 30.0) + 2.0
            } else {
                -20.0
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
    let terrain = TerrainRenderer::new(&gpu, &renderer, &island(), None);
    let stats = terrain.stats();
    renderer.add_feature(Box::new(terrain));
    let image = renderer
        .render_to_image(&gpu, W, H, camera, &DrawList::default(), 1.0 / 60.0)
        .expect("render");
    assert!(stats.lock().unwrap().nodes > 0);
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
    let bottom = pixel(&image, W / 2, H - 2);
    assert!(
        !is_land(bottom) && bottom[2] >= bottom[0],
        "sea between camera and island: {bottom:?}"
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
