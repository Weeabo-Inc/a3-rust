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
    }
    Ok(())
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
    let _ = writeln!(s, "LODs         {}", model.lods.len());
    let _ = writeln!(
        s,
        "    {:>3}  {:<28} {:>8} {:>8} {:>5} {:>5} {:>5}",
        "#", "resolution", "vertices", "faces", "sect", "sel", "tex"
    );
    for (n, lod) in model.lods.iter().enumerate() {
        let (vertices, faces) = counts(lod);
        let _ = writeln!(
            s,
            "    {n:>3}  {:<28} {vertices:>8} {faces:>8} {:>5} {:>5} {:>5}",
            lod.resolution.to_string(),
            lod.sections.len(),
            lod.named_selections.len(),
            lod.textures.len()
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
        let _ = writeln!(s, "    {} at {}", p.model, vec3(p.position));
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
}
