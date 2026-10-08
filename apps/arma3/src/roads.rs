//! A World's roads for the renderer: the roads shapefile joined into the engine's curves,
//! tessellated on the terrain, with the RoadsLib textures.

use std::time::Instant;

use a3_landscape::{RoadGraph, RoadNetwork, WorldConfig};
use a3_render::roads::{
    RoadChunk, RoadFeature, RoadMaterial, RoadMeshSettings, RoadSegment, build_road_meshes,
};
use a3_render::{Gpu, Renderer, TextureData, TextureFormat};
use a3_vfs::Vfs;
use a3_wrp::Terrain;
use anyhow::Context as _;

/// Road meshes and textures, loaded off the GPU.
pub struct LoadedRoads {
    /// Per road material: straight and end texture (`None` when missing or unreadable).
    pub textures: Vec<[Option<a3_paa::Texture>; 2]>,
    pub chunks: Vec<RoadChunk>,
}

/// Load the roads of `config`'s World and tessellate them on `terrain`.
pub fn load(vfs: &Vfs, config: &WorldConfig, terrain: &Terrain) -> anyhow::Result<LoadedRoads> {
    let start = Instant::now();
    // VR names no shapefile in its config but ships one next to its WRP.
    let shp = config.roads_shape.clone().unwrap_or_else(|| {
        config
            .wrp
            .parent()
            .unwrap_or_default()
            .join(r"data\roads\roads.shp")
    });
    let network = RoadNetwork::load(vfs, &shp).with_context(|| format!("cannot load {shp}"))?;
    let graph = RoadGraph::new(&network);

    // One material per road type in use.
    let mut ids: Vec<u32> = network.roads.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let read = |path: &str| {
        let texture = vfs
            .open(path)
            .ok()
            .and_then(|bytes| a3_paa::Texture::read(&bytes).ok());
        if texture.is_none() {
            log::warn!("road texture {path} is missing or unreadable");
        }
        texture
    };
    let textures = ids
        .iter()
        .map(|&id| {
            let road = network
                .roads
                .iter()
                .find(|r| r.id == id)
                .expect("id in use");
            let road_type = network.road_type(road);
            [
                read(&road_type.straight_texture),
                read(&road_type.end_texture),
            ]
        })
        .collect();

    let mut segments = Vec::new();
    for (i, road) in network.roads.iter().enumerate() {
        let width = network.road_type(road).width;
        let material = ids.binary_search(&road.id).expect("listed") as u32;
        segments.extend(graph.curve(i).into_iter().map(|c| RoadSegment {
            road: i as u32,
            p0: c.p0,
            c1: c.c1,
            c2: c.c2,
            p1: c.p1,
            width,
            material,
            open_start: c.open_start,
            open_end: c.open_end,
        }));
    }
    let chunks = build_road_meshes(
        &segments,
        |x, z| terrain.surface_height(x, z),
        &RoadMeshSettings::default(),
    );
    log::info!(
        "roads {shp}: {} roads, {} pieces in {} chunks, {} materials, {:.2?}",
        network.roads.len(),
        segments.len(),
        chunks.len(),
        ids.len(),
        start.elapsed()
    );
    Ok(LoadedRoads { textures, chunks })
}

/// Upload `roads` as a [`RoadFeature`] for `renderer`.
pub fn feature(gpu: &Gpu, renderer: &Renderer, roads: &LoadedRoads) -> anyhow::Result<RoadFeature> {
    let bc = Renderer::supports_bc(gpu);
    let materials: Vec<RoadMaterial> = roads
        .textures
        .iter()
        .map(|[straight, end]| RoadMaterial {
            straight: texture_data(straight.as_ref(), bc),
            end: texture_data(end.as_ref(), bc),
        })
        .collect();
    RoadFeature::new(&gpu.device, &gpu.queue, renderer, &materials, &roads.chunks)
        .context("cannot upload road textures")
}

/// A PAA texture as [`TextureData`]: BC blocks when the adapter has them, RGBA8 otherwise;
/// grey when missing or when its mips do not fit.
fn texture_data(texture: Option<&a3_paa::Texture>, bc: bool) -> TextureData {
    let fallback = || TextureData::solid_rgba8([90, 90, 90, 255]);
    let Some(texture) = texture else {
        return fallback();
    };
    let width = u32::from(texture.width());
    let height = u32::from(texture.height());
    let format = match texture.format {
        a3_paa::PixelFormat::Dxt1 => Some(TextureFormat::Bc1),
        a3_paa::PixelFormat::Dxt3 => Some(TextureFormat::Bc2),
        a3_paa::PixelFormat::Dxt5 => Some(TextureFormat::Bc3),
        _ => None,
    };
    let data = match format {
        Some(format) if bc => TextureData {
            format,
            width,
            height,
            mips: texture.mips.iter().map(|m| m.data.clone()).collect(),
        },
        _ => TextureData {
            format: TextureFormat::Rgba8,
            width,
            height,
            mips: texture
                .mips
                .iter()
                .map_while(|m| a3_paa::decode_rgba8(texture.format, m).ok())
                .collect(),
        },
    };
    if data.validate().is_err() {
        return fallback();
    }
    data
}
