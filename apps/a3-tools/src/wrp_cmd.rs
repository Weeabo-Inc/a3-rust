//! `a3-tools wrp ...`: inspect binarized terrains (WRP).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use a3_landscape::RoadNetwork;
use a3_vfs::Vfs;
use a3_wrp::{MapShape, MapType, Terrain};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use glam::Vec2;

#[derive(Args)]
pub struct WrpArgs {
    /// Game install folder, used when the terrain path is a VFS path.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: WrpCommand,
}

#[derive(Subcommand)]
enum WrpCommand {
    /// Print a summary of a terrain.
    Info {
        /// A `.wrp` file on disk, or a VFS path such as `a3\map_altis\altis.wrp`.
        path: String,
    },
    /// Export the heightmap: `.png` as 16-bit greyscale scaled to the height range, `.tif` /
    /// `.tiff` as 32-bit float metres. North is up.
    Heightmap {
        /// A `.wrp` file on disk or a VFS path.
        path: String,
        /// Output image.
        out: PathBuf,
    },
    /// Print the placed objects: a count per model, or with `--csv` one CSV row per object.
    Objects {
        /// A `.wrp` file on disk or a VFS path.
        path: String,
        /// One row per object: id, model, x, y, z, heading (degrees), scale.
        #[arg(long)]
        csv: bool,
    },
    /// Render a top-down overview PNG: hillshaded terrain, water, map symbols and, for a VFS
    /// path, the roads of `<terrain folder>\data\roads\roads.shp`.
    Map {
        /// A `.wrp` file on disk or a VFS path.
        path: String,
        /// Output PNG.
        out: PathBuf,
        /// Image edge length in pixels.
        #[arg(long, default_value_t = 2048)]
        size: u32,
    },
}

pub fn run(args: WrpArgs) -> Result<()> {
    match args.command {
        WrpCommand::Info { path } => info(&load(&path, args.game_dir.as_deref())?, &path),
        WrpCommand::Heightmap { path, out } => {
            heightmap(&load(&path, args.game_dir.as_deref())?, &out)
        }
        WrpCommand::Objects { path, csv } => {
            let terrain = load(&path, args.game_dir.as_deref())?;
            match objects(&terrain, &mut std::io::stdout().lock(), csv) {
                // `| head` closes the pipe early; that is not an error.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                other => Ok(other?),
            }
        }
        WrpCommand::Map { path, out, size } => {
            let (terrain, vfs) = load_with_vfs(&path, args.game_dir.as_deref())?;
            let roads = vfs.and_then(|vfs| {
                let shp = a3_core::VfsPath::new(&path)
                    .parent()?
                    .join(r"data\roads\roads.shp");
                match RoadNetwork::load(&vfs, &shp) {
                    Ok(net) => {
                        eprintln!("drawing {} roads from {shp}", net.roads.len());
                        Some(net)
                    }
                    Err(e) => {
                        eprintln!("no roads: {e}");
                        None
                    }
                }
            });
            map(&terrain, roads.as_ref(), &out, size)
        }
    }
}

/// Loads a terrain from an OS file, or else from the VFS of `game_dir`.
fn load(path: &str, game_dir: Option<&Path>) -> Result<Terrain> {
    Ok(load_with_vfs(path, game_dir)?.0)
}

/// Like [`load`], also returning the VFS when the terrain came from it.
fn load_with_vfs(path: &str, game_dir: Option<&Path>) -> Result<(Terrain, Option<Vfs>)> {
    let os_path = Path::new(path);
    let (data, vfs) = if os_path.is_file() {
        (
            std::fs::read(os_path).with_context(|| format!("reading {path}"))?,
            None,
        )
    } else {
        let Some(game_dir) = game_dir else {
            bail!("{path} is not a file; pass --game-dir (or set A3_ROOT) to read it from the VFS");
        };
        let vfs = crate::vfs_cmd::mount(game_dir, &[], true)?;
        (vfs.open(path)?.to_vec(), Some(vfs))
    };
    let start = Instant::now();
    let terrain = Terrain::parse(&data).with_context(|| format!("parsing {path}"))?;
    eprintln!(
        "parsed {path} ({} bytes) in {:.2?}",
        data.len(),
        start.elapsed()
    );
    Ok((terrain, vfs))
}

fn info(t: &Terrain, path: &str) -> Result<()> {
    let (lo, hi) = height_range(t);
    println!("{path}");
    println!("  version       {}", t.version);
    if let Some(app) = t.app_id {
        println!("  app id        {app}");
    }
    println!("  size          {} m", t.world_size());
    println!(
        "  land grid     {}x{} cells of {} m",
        t.land_grid.width, t.land_grid.height, t.land_cell_size
    );
    println!(
        "  heightmap     {}x{} samples every {} m, {lo:.2} to {hi:.2} m",
        t.heightmap.width(),
        t.heightmap.height(),
        t.terrain_cell_size()
    );
    println!(
        "  sound map     {}x{}",
        t.sound_map.width(),
        t.sound_map.height()
    );
    println!("  mountains     {}", t.mountains.len());
    println!("  materials     {}", t.materials.len());
    println!("  models        {}", t.models.len());
    println!(
        "  objects       {} (max id {})",
        t.objects.len(),
        t.max_object_id
    );
    println!("  entities      {}", t.entities.len());
    println!(
        "  road parts    {} ({} models)",
        t.roads.parts.len(),
        t.roads.models.len()
    );
    println!("  map objects   {}", t.map_objects.len());
    let mut kinds: BTreeMap<MapType, usize> = BTreeMap::new();
    for m in &t.map_objects {
        *kinds.entry(m.kind).or_default() += 1;
    }
    for (kind, n) in kinds {
        println!("    {:<16}{n}", kind.name());
    }
    Ok(())
}

fn height_range(t: &Terrain) -> (f32, f32) {
    t.heightmap
        .as_slice()
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)))
}

/// The heightmap rows from north to south.
fn rows_north_up(t: &Terrain) -> impl Iterator<Item = &[f32]> {
    (0..t.heightmap.height()).rev().map(|z| t.heightmap.row(z))
}

fn heightmap(t: &Terrain, out: &Path) -> Result<()> {
    let (w, h) = (t.heightmap.width(), t.heightmap.height());
    let ext = out
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let file = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    match ext.as_str() {
        "png" => {
            let (lo, hi) = height_range(t);
            let scale = if hi > lo { 65535.0 / (hi - lo) } else { 0.0 };
            let mut data = Vec::with_capacity(w as usize * h as usize * 2);
            for row in rows_north_up(t) {
                for &v in row {
                    let q = ((v - lo) * scale).round().clamp(0.0, 65535.0) as u16;
                    data.extend_from_slice(&q.to_be_bytes());
                }
            }
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Sixteen);
            enc.write_header()?.write_image_data(&data)?;
            println!("wrote {w}x{h} 16-bit PNG: 0 = {lo} m, 65535 = {hi} m");
        }
        "tif" | "tiff" => {
            let data: Vec<f32> = rows_north_up(t).flatten().copied().collect();
            let mut enc = tiff::encoder::TiffEncoder::new(file)?;
            enc.write_image::<tiff::encoder::colortype::Gray32Float>(w, h, &data)?;
            println!("wrote {w}x{h} 32-bit float TIFF (metres)");
        }
        _ => bail!("unsupported heightmap format {ext:?}: use .png or .tiff"),
    }
    Ok(())
}

fn objects(t: &Terrain, out: &mut impl Write, csv: bool) -> std::io::Result<()> {
    let mut out = BufWriter::new(out);
    if csv {
        writeln!(out, "id,model,x,y,z,heading,scale")?;
        for o in &t.objects {
            let p = o.transform.position();
            let scale = glam::Vec3::from_slice(&o.transform.0[3..6]).length();
            let model = t.model_of(o).map_or("", |m| m.as_str());
            writeln!(
                out,
                "{},{},{:.3},{:.3},{:.3},{:.2},{:.4}",
                o.id,
                model,
                p.x,
                p.y,
                p.z,
                o.transform.heading_degrees(),
                scale
            )?;
        }
    } else {
        let mut counts = vec![0usize; t.models.len()];
        for o in &t.objects {
            if let Some(c) = counts.get_mut(o.model_index as usize) {
                *c += 1;
            }
        }
        let mut by_count: Vec<(usize, &str)> = counts
            .iter()
            .zip(&t.models)
            .map(|(&n, m)| (n, m.as_str()))
            .collect();
        by_count.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        for (n, model) in by_count {
            writeln!(out, "{n:>9}  {model}")?;
        }
        writeln!(out, "{:>9}  objects total", t.objects.len())?;
    }
    out.flush()?;
    Ok(())
}

/// An RGB image with world-to-pixel mapping (north up).
struct Canvas {
    size: u32,
    world: f32,
    rgb: Vec<u8>,
}

impl Canvas {
    fn pixel(&self, p: Vec2) -> (i64, i64) {
        let s = self.size as f32 / self.world;
        (
            (p.x * s).floor() as i64,
            (self.size as f32 - p.y * s).floor() as i64,
        )
    }

    fn put(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if (0..self.size as i64).contains(&x) && (0..self.size as i64).contains(&y) {
            let i = (y as usize * self.size as usize + x as usize) * 3;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    fn dot(&mut self, p: Vec2, c: [u8; 3]) {
        let (x, y) = self.pixel(p);
        self.put(x, y, c);
    }

    fn line(&mut self, a: Vec2, b: Vec2, c: [u8; 3]) {
        let (pa, pb) = (self.pixel(a), self.pixel(b));
        let steps = (pb.0 - pa.0).abs().max((pb.1 - pa.1).abs()).max(1);
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            let x = pa.0 as f32 + (pb.0 - pa.0) as f32 * f;
            let y = pa.1 as f32 + (pb.1 - pa.1) as f32 * f;
            self.put(x.round() as i64, y.round() as i64, c);
        }
    }

    /// Fills a convex quad (corners in order).
    fn quad(&mut self, corners: &[Vec2; 4], c: [u8; 3]) {
        let px: Vec<Vec2> = corners
            .iter()
            .map(|&p| {
                let (x, y) = self.pixel(p);
                Vec2::new(x as f32, y as f32)
            })
            .collect();
        let (min, max) = px.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), &p| {
            (lo.min(p), hi.max(p))
        });
        if (max - min).max_element() > 256.0 {
            return;
        }
        let side = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
        for y in min.y as i64..=max.y as i64 {
            for x in min.x as i64..=max.x as i64 {
                let p = Vec2::new(x as f32, y as f32);
                let s: Vec<f32> = (0..4).map(|i| side(px[i], px[(i + 1) % 4], p)).collect();
                if s.iter().all(|&v| v >= -0.5) || s.iter().all(|&v| v <= 0.5) {
                    self.put(x, y, c);
                }
            }
        }
        if min == max {
            self.put(min.x as i64, min.y as i64, c);
        }
    }
}

fn terrain_color(h: f32, shade: f32) -> [u8; 3] {
    let base = if h < 0.0 {
        let d = (-h / 60.0).min(1.0);
        [90.0 - 60.0 * d, 150.0 - 80.0 * d, 200.0 - 60.0 * d]
    } else {
        let stops = [
            (0.0, [190.0, 200.0, 150.0]),
            (60.0, [150.0, 175.0, 110.0]),
            (200.0, [170.0, 150.0, 110.0]),
            (400.0, [210.0, 200.0, 190.0]),
        ];
        let mut c = stops[stops.len() - 1].1;
        for w in stops.windows(2) {
            if h < w[1].0 {
                let f = (h - w[0].0) / (w[1].0 - w[0].0);
                c = std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * f);
                break;
            }
        }
        c
    };
    let shade = if h < 0.0 { 0.85 + 0.15 * shade } else { shade };
    base.map(|v| (v * shade).clamp(0.0, 255.0) as u8)
}

fn map(t: &Terrain, roads: Option<&RoadNetwork>, out: &Path, size: u32) -> Result<()> {
    if !(16..=16384).contains(&size) {
        bail!("--size must be between 16 and 16384");
    }
    let world = t.world_size();
    let px = world / size as f32;
    let mut canvas = Canvas {
        size,
        world,
        rgb: vec![0; size as usize * size as usize * 3],
    };
    // Hillshade, light from the north-west.
    let light = glam::Vec3::new(-1.0, 1.0, 1.0).normalize();
    for y in 0..size {
        let z = world - (y as f32 + 0.5) * px;
        for x in 0..size {
            let wx = (x as f32 + 0.5) * px;
            let h = t.surface_height(wx, z);
            let dx = t.surface_height(wx + px, z) - t.surface_height(wx - px, z);
            let dz = t.surface_height(wx, z + px) - t.surface_height(wx, z - px);
            let normal = glam::Vec3::new(-dx, 2.0 * px, -dz).normalize();
            let lit = glam::Vec3::new(light.x, light.z, light.y);
            let shade = 0.55 + 0.6 * normal.dot(lit).max(0.0);
            canvas.put(x as i64, y as i64, terrain_color(h, shade));
        }
    }
    // Map symbols.
    let green = [40, 100, 40];
    let dark_green = [20, 75, 30];
    for m in &t.map_objects {
        match (&m.shape, m.kind) {
            (MapShape::Point(p), MapType::Bush) => canvas.dot(*p, [70, 120, 50]),
            (MapShape::Point(p), MapType::Tree | MapType::SmallTree) => canvas.dot(*p, green),
            (MapShape::Point(p), MapType::Rock) => canvas.dot(*p, [110, 110, 110]),
            (MapShape::Point(p), _) => canvas.dot(*p, [200, 30, 30]),
            (MapShape::RectColored { corners, .. }, MapType::Fence | MapType::Wall) => {
                canvas.line(corners[0], corners[2], [90, 80, 70])
            }
            (MapShape::RectColored { corners, .. }, _) => canvas.quad(corners, [60, 50, 50]),
            (MapShape::Rect(corners), _) => canvas.quad(corners, [230, 190, 60]),
            (MapShape::Line(a, b), _) => canvas.line(*a, *b, [120, 120, 140]),
            (MapShape::RailWay { values: v, .. }, _) => {
                canvas.line(Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), [40, 40, 40])
            }
            (MapShape::River(points), _) => {
                for w in points.windows(2) {
                    canvas.line(w[0], w[1], [50, 90, 200]);
                }
            }
            (MapShape::Forest { .. }, _) => {}
            _ => {}
        }
    }
    // Forest cells from the geography grid.
    let lc = t.land_cell_size;
    for z in 0..t.land_grid.height {
        for x in 0..t.land_grid.width {
            if t.geography.get(x, z).is_some_and(|g| g.forest()) {
                let c = Vec2::new((x as f32 + 0.5) * lc, (z as f32 + 0.5) * lc);
                canvas.dot(c, dark_green);
            }
        }
    }
    // Roads from the shapefile, coloured by how the map draws their type.
    if let Some(net) = roads {
        for road in &net.roads {
            let color = match net.road_type(road).map_type {
                Some(MapType::MainRoad) => [200, 60, 40],
                Some(MapType::Road) => [230, 150, 40],
                Some(MapType::Trail) => [150, 120, 90],
                _ => [120, 95, 70],
            };
            for w in road.points.windows(2) {
                canvas.line(w[0], w[1], color);
            }
        }
    }
    // Road parts (bridges, runways).
    for part in &t.roads.parts {
        let ends = t.roads.connections_of(part);
        for w in ends.windows(2) {
            let (a, b) = (w[0].position, w[1].position);
            canvas.line(Vec2::new(a.x, a.z), Vec2::new(b.x, b.z), [230, 120, 30]);
        }
        let p = part.transform.position();
        canvas.dot(Vec2::new(p.x, p.z), [230, 120, 30]);
    }
    let file = BufWriter::new(File::create(out).with_context(|| out.display().to_string())?);
    let mut enc = png::Encoder::new(file, size, size);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&canvas.rgb)?;
    println!(
        "wrote {size}x{size} map ({px:.2} m per pixel) to {}",
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_wrp::{TerrainBuilder, Transform};

    fn sample() -> Terrain {
        TerrainBuilder::new(4, 8, 25.0)
            .heights(|i, j| i as f32 * 3.0 - j as f32 - 4.0)
            .object(
                r"a3\plants_f\tree.p3d",
                Transform::from_position(glam::Vec3::new(10.0, 1.0, 20.0)),
            )
            .build()
    }

    #[test]
    fn objects_csv_has_one_row_per_object() {
        let mut out = Vec::new();
        objects(&sample(), &mut out, true).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "id,model,x,y,z,heading,scale");
        assert_eq!(
            lines[1],
            r"0,a3\plants_f\tree.p3d,10.000,1.000,20.000,0.00,1.0000"
        );
    }

    #[test]
    fn heightmap_and_map_write_images() {
        let dir = tempfile::tempdir().unwrap();
        let terrain = sample();
        let png_path = dir.path().join("h.png");
        heightmap(&terrain, &png_path).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(File::open(&png_path).unwrap()));
        let reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().width, 8);
        assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
        heightmap(&terrain, &dir.path().join("h.tiff")).unwrap();
        assert!(heightmap(&terrain, &dir.path().join("h.bmp")).is_err());

        let map_path = dir.path().join("map.png");
        let net = a3_landscape::RoadNetwork {
            roads: vec![a3_landscape::Road {
                record: 0,
                id: 1,
                order: 0,
                mask: 0,
                points: vec![Vec2::new(1.0, 1.0), Vec2::new(90.0, 90.0)],
            }],
            library: Default::default(),
        };
        map(&terrain, Some(&net), &map_path, 64).unwrap();
        let reader = png::Decoder::new(std::io::BufReader::new(File::open(&map_path).unwrap()))
            .read_info()
            .unwrap();
        assert_eq!((reader.info().width, reader.info().height), (64, 64));
    }
}
