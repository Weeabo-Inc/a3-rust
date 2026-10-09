//! `a3-tools p3d ...`: inspect P3D models.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use a3_p3d::{Lod, Model};
use anyhow::{Context, bail};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct P3dArgs {
    /// Game install folder, for models given as VFS paths.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: P3dCommand,
}

#[derive(Subcommand)]
enum P3dCommand {
    /// Print the model info, skeleton, animations and LOD table.
    Info {
        /// A `.p3d` file on disk, or a VFS path such as `a3\weapons_f\rifles\mx\mx_f.p3d`.
        model: String,
        /// Also list the textures, materials, selections, properties and proxies of LOD N.
        #[arg(long, value_name = "N")]
        lod: Option<usize>,
    },
    /// Export one LOD to Wavefront OBJ or glTF 2.0 (`.obj`, `.gltf` + `.bin`, `.glb`).
    Export {
        /// A `.p3d` file on disk, or a VFS path.
        model: String,
        /// Output file; the extension picks the format.
        out: PathBuf,
        /// Index of the LOD to export (see `p3d info`).
        #[arg(long, default_value_t = 0)]
        lod: usize,
        /// Pose the model first: set animation source NAME to VALUE (e.g. `door_lf=1`);
        /// repeatable. Sources not given are 0. Hidden sections are left out.
        #[arg(long = "source", value_name = "NAME=VALUE")]
        sources: Vec<String>,
        /// Pose the skeleton with an RTM animation (file or VFS path).
        #[arg(long, value_name = "RTM")]
        rtm: Option<String>,
        /// Phase of the RTM to sample, 0..1.
        #[arg(long, default_value_t = 0.0)]
        phase: f32,
        /// A second RTM to blend with the first (at its own `--phase`).
        #[arg(long, value_name = "RTM", requires = "rtm")]
        blend_rtm: Option<String>,
        /// Blend weight of `--blend-rtm`, 0..1.
        #[arg(long, default_value_t = 0.5)]
        blend: f32,
        /// The skeleton's pivots model (CfgSkeletonParameters `pivotsModel`) for `--rtm`.
        #[arg(long, default_value = r"a3\anims_f\data\skeleton\skeletonpivots.p3d")]
        pivots: String,
    },
}

pub fn run(args: P3dArgs) -> anyhow::Result<()> {
    match args.command {
        P3dCommand::Info { model, lod } => {
            let bytes = load(&model, args.game_dir.as_deref())?;
            let model =
                Model::from_bytes(&bytes).with_context(|| format!("decoding model {model}"))?;
            print!("{}", info(&model));
            if let Some(i) = lod {
                let Some(l) = model.lods.get(i) else {
                    bail!("model has {} LODs, no LOD {i}", model.lods.len());
                };
                print!("{}", lod_details(i, l));
            }
        }
        P3dCommand::Export {
            model,
            out,
            lod,
            sources,
            rtm,
            phase,
            blend_rtm,
            blend,
            pivots,
        } => {
            let bytes = load(&model, args.game_dir.as_deref())?;
            let model =
                Model::from_bytes(&bytes).with_context(|| format!("decoding model {model}"))?;
            let Some(l) = model.lods.get(lod) else {
                bail!("model has {} LODs, no LOD {lod}", model.lods.len());
            };
            let posed;
            let l = if sources.is_empty() && rtm.is_none() {
                l
            } else {
                let mut pose = a3_anim::pose(&model, lod, &parse_sources(&sources)?);
                if let Some(rtm) = rtm {
                    let skeleton = model
                        .skeleton
                        .as_ref()
                        .context("--rtm needs a model with a skeleton")?;
                    let read = |path: &str| -> anyhow::Result<a3_rtm::Animation> {
                        let bytes = load(path, args.game_dir.as_deref())?;
                        a3_rtm::Animation::read(&bytes).with_context(|| format!("decoding {path}"))
                    };
                    let pivots_model = Model::from_bytes(&load(&pivots, args.game_dir.as_deref())?)
                        .with_context(|| format!("decoding pivots model {pivots}"))?;
                    let pivots = a3_anim::SkeletonPivots::from_model(skeleton, &pivots_model, "");
                    let first = read(&rtm)?;
                    let binding = a3_anim::RtmBinding::new(skeleton, &first);
                    eprintln!(
                        "{rtm}: {} of {} skeleton bones animated",
                        binding.bound(),
                        skeleton.bones.len()
                    );
                    let mut frames = binding.frames(&first, phase, &pivots);
                    if let Some(second) = blend_rtm {
                        let other = read(&second)?;
                        let b = a3_anim::RtmBinding::new(skeleton, &other)
                            .frames(&other, phase, &pivots);
                        frames = a3_anim::blend(&frames, &b, blend);
                    }
                    let offset = if pivots_model.info.auto_center {
                        model.info.bounding_center - pivots_model.info.bounding_center
                    } else {
                        model.info.bounding_center
                    };
                    pose = a3_anim::Pose::from_rtm_frames(&frames, &pivots, offset).compose(&pose);
                }
                posed = posed_lod(&model.lods[lod], &pose);
                &posed
            };
            let mesh = crate::p3d_export::Mesh::from_lod(l);
            if mesh.primitives.is_empty() {
                bail!("LOD {lod} ({}) has no faces to export", l.resolution);
            }
            crate::p3d_export::write(&mesh, &out)?;
            eprint!("{}", export_report(&model, l, &mesh));
        }
    }
    Ok(())
}

/// Parses `NAME=VALUE` source assignments.
fn parse_sources(items: &[String]) -> anyhow::Result<a3_anim::Sources> {
    let mut sources = a3_anim::Sources::new();
    for item in items {
        let Some((name, value)) = item.split_once('=') else {
            bail!("--source expects NAME=VALUE, got {item:?}");
        };
        let value: f32 = value
            .trim()
            .parse()
            .with_context(|| format!("--source {item}: not a number"))?;
        sources.set(name.trim(), value);
    }
    Ok(sources)
}

/// A copy of `lod` posed by `pose`: vertices skinned, sections the pose hides removed.
fn posed_lod(lod: &Lod, pose: &a3_anim::Pose) -> Lod {
    let mut l = lod.clone();
    let skinning = pose.skinning(&l);
    let skinned = a3_anim::skin(&l, &skinning);
    let hidden = a3_anim::hidden_sections(&l, &skinning);
    l.vertices.positions = skinned.positions;
    if !skinned.normals.is_empty() {
        l.vertices.normals = skinned.normals;
    }
    let mut keep = hidden.iter().map(|h| !h);
    l.sections.retain(|_| keep.next().unwrap_or(true));
    l
}

/// Sanity figures for an exported LOD, in engine space.
fn export_report(model: &Model, lod: &Lod, mesh: &crate::p3d_export::Mesh) -> String {
    let mut s = String::new();
    let v = &lod.vertices;
    let tris: usize = mesh.primitives.iter().map(|(_, t)| t.len() / 3).sum();
    let _ = writeln!(
        s,
        "exported LOD {}: {} vertices, {tris} triangles, {} primitives",
        lod.resolution,
        v.len(),
        mesh.primitives.len()
    );
    if let Some((lo, hi)) = v
        .positions
        .iter()
        .copied()
        .map(|p| (p, p))
        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    {
        let i = &model.info;
        let inside = lo.cmpge(i.bbox_min - 1e-3).all() && hi.cmple(i.bbox_max + 1e-3).all();
        let _ = writeln!(
            s,
            "bounds {} .. {} (model bbox {} .. {}: {})",
            vec3(lo),
            vec3(hi),
            vec3(i.bbox_min),
            vec3(i.bbox_max),
            if inside { "inside" } else { "OUTSIDE" }
        );
    }
    if let Some(uv) = v.uv_sets.first().filter(|uv| !uv.is_empty()) {
        let (lo, hi) = uv
            .iter()
            .fold((uv[0], uv[0]), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
        let _ = writeln!(
            s,
            "uv0 range [{:.3}, {:.3}] .. [{:.3}, {:.3}]",
            lo.x, lo.y, hi.x, hi.y
        );
    }
    if !v.normals.is_empty() {
        let lens: Vec<f32> = v.normals.iter().map(|n| n.length()).collect();
        let unit = lens.iter().filter(|l| (**l - 1.0).abs() < 0.01).count();
        let _ = writeln!(s, "normals: {unit} of {} unit length", lens.len());
    }
    s
}

/// Reads `model` from disk if such a file exists, else from the VFS of the game in `game_dir`.
fn load(model: &str, game_dir: Option<&Path>) -> anyhow::Result<Vec<u8>> {
    crate::vfs_cmd::read_file_or_vfs(model, game_dir)
}

fn vec3(v: glam::Vec3) -> String {
    format!("[{:.3}, {:.3}, {:.3}]", v.x, v.y, v.z)
}

/// The summary printed by `p3d info`.
pub fn info(model: &Model) -> String {
    let mut s = String::new();
    let i = &model.info;
    let _ = writeln!(s, "encoding     {:?} v{}", model.encoding, model.version);
    let _ = writeln!(
        s,
        "bbox         {} .. {}",
        vec3(i.bbox_min),
        vec3(i.bbox_max)
    );
    let _ = writeln!(
        s,
        "visual bbox  {} .. {}",
        vec3(i.bbox_visual_min),
        vec3(i.bbox_visual_max)
    );
    let _ = writeln!(
        s,
        "sphere       {:.3} (geometry {:.3})",
        i.bounding_sphere, i.geometry_sphere
    );
    let _ = writeln!(
        s,
        "mass         {:.3}, center of mass {}",
        i.mass,
        vec3(i.center_of_mass)
    );
    let _ = writeln!(s, "armor        {:.3}", i.armor);
    let _ = writeln!(s, "class        {:?} damage {:?}", i.class, i.damage);
    if !i.muzzle_flash.is_empty() {
        let _ = writeln!(s, "muzzle flash {}", i.muzzle_flash);
    }
    match &model.skeleton {
        Some(sk) => {
            let _ = writeln!(s, "skeleton     {} ({} bones)", sk.name, sk.bones.len());
        }
        None => {
            let _ = writeln!(s, "skeleton     none");
        }
    }
    let _ = writeln!(s, "animations   {}", model.animations.len());
    for a in &model.animations {
        let _ = writeln!(
            s,
            "    {:<32} source {:<20} {:?}",
            a.name, a.source, a.transform
        );
    }
    let _ = writeln!(
        s,
        "lodDensityCoef {:.3}, drawImportance {:.3}",
        i.lod_density_coef, i.draw_importance
    );
    let _ = writeln!(
        s,
        "min shadow   {}, sbsource {:?}",
        i.min_shadow, i.shadow_source
    );
    let _ = writeln!(s, "LODs         {}", model.lods.len());
    let _ = writeln!(
        s,
        "    {:>3}  {:<28} {:>8} {:>8} {:>5} {:>5} {:>5} {:>5} {:>5}",
        "#", "resolution", "vertices", "faces", "sect", "sel", "tex", "sb", "sv"
    );
    // A LOD's preferred shadow-buffer / shadow-volume LOD index, "-" when unset.
    let shadow = |list: &[i32], n: usize| match list.get(n) {
        Some(&l) if l >= 0 => l.to_string(),
        _ => "-".to_owned(),
    };
    for (n, lod) in model.lods.iter().enumerate() {
        let (vertices, faces) = counts(lod);
        let _ = writeln!(
            s,
            "    {n:>3}  {:<28} {vertices:>8} {faces:>8} {:>5} {:>5} {:>5} {:>5} {:>5}",
            lod.resolution.to_string(),
            lod.sections.len(),
            lod.named_selections.len(),
            lod.textures.len(),
            shadow(&i.preferred_shadow_buffer_lod, n),
            shadow(&i.preferred_shadow_volume_lod, n)
        );
    }
    s
}

/// Vertex and face counts, from the ODOL LOD summary when the geometry is not decoded.
fn counts(lod: &Lod) -> (usize, usize) {
    match lod.odol.as_ref().and_then(|o| o.summary) {
        Some(summary) if lod.vertices.is_empty() => {
            (summary.vertices as usize, summary.faces as usize)
        }
        _ => (lod.vertices.len(), lod.faces.len()),
    }
}

/// The per-LOD listing printed by `p3d info --lod N`.
pub fn lod_details(index: usize, lod: &Lod) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "\nLOD {index}: {}", lod.resolution);
    let _ = writeln!(s, "textures ({})", lod.textures.len());
    for t in &lod.textures {
        let _ = writeln!(s, "    {t}");
    }
    let _ = writeln!(s, "materials ({})", lod.materials.len());
    for m in &lod.materials {
        let _ = writeln!(s, "    {}", m.name);
    }
    let _ = writeln!(s, "sections ({})", lod.sections.len());
    for (n, sec) in lod.sections.iter().enumerate() {
        let name = |list: &[String], i: Option<u32>| {
            i.and_then(|i| list.get(i as usize).cloned())
                .unwrap_or_else(|| "-".into())
        };
        let materials: Vec<String> = lod.materials.iter().map(|m| m.name.clone()).collect();
        let _ = writeln!(
            s,
            "    {n:>3}  faces {:>6}..{:<6} flags {:#010x}  {}  {}",
            sec.faces.start,
            sec.faces.end,
            sec.flags,
            name(&lod.textures, sec.texture),
            name(&materials, sec.material)
        );
    }
    let _ = writeln!(s, "named selections ({})", lod.named_selections.len());
    for sel in &lod.named_selections {
        let _ = writeln!(
            s,
            "    {:<32} {} vertices, {} faces",
            sel.name,
            sel.vertices.len(),
            sel.faces.len()
        );
    }
    let _ = writeln!(s, "properties ({})", lod.properties.len());
    for (k, v) in &lod.properties {
        let _ = writeln!(s, "    {k} = {v}");
    }
    let _ = writeln!(s, "proxies ({})", lod.proxies.len());
    for p in &lod.proxies {
        let _ = writeln!(
            s,
            "    {} at {} (selection {}, section {})",
            p.model,
            vec3(p.position),
            p.named_selection,
            p.section
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{Encoding, LodResolution, ModelInfo, NamedSelection};

    #[test]
    fn info_lists_every_lod_by_name() {
        let lod = |r| Lod {
            resolution: LodResolution(r),
            named_selections: vec![NamedSelection {
                name: "door".into(),
                ..NamedSelection::default()
            }],
            ..Lod::default()
        };
        let model = Model {
            encoding: Encoding::Odol,
            version: 73,
            info: ModelInfo::default(),
            skeleton: None,
            animations: vec![],
            lods: vec![lod(1.0), lod(1e15)],
        };
        let text = info(&model);
        assert!(text.contains("Odol v73"), "{text}");
        assert!(text.contains("1.000"), "{text}");
        assert!(text.contains("Memory"), "{text}");
        assert!(lod_details(1, &model.lods[1]).contains("door"));
    }

    #[test]
    fn info_shows_what_lod_and_shadow_selection_read() {
        let model = Model {
            encoding: Encoding::Odol,
            version: 73,
            info: ModelInfo {
                lod_density_coef: 1.5,
                draw_importance: 0.25,
                min_shadow: 1,
                preferred_shadow_buffer_lod: vec![2, -1],
                preferred_shadow_volume_lod: vec![-1, 3],
                ..ModelInfo::default()
            },
            skeleton: None,
            animations: vec![],
            lods: vec![Lod::default(), Lod::default()],
        };
        let text = info(&model);
        assert!(text.contains("lodDensityCoef 1.500"), "{text}");
        assert!(text.contains("drawImportance 0.250"), "{text}");
        assert!(text.contains("min shadow   1"), "{text}");
        // The per-LOD preferred shadow-buffer and shadow-volume LODs, "-" when unset.
        let rows: Vec<&str> = text
            .lines()
            .filter(|l| l.trim_start().starts_with(['0', '1']) && l.contains("0.000"))
            .collect();
        assert!(rows[0].trim_end().ends_with("2     -"), "{text}");
        assert!(rows[1].trim_end().ends_with("-     3"), "{text}");
    }
}
