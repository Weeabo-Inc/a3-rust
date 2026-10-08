//! Export one P3D LOD to Wavefront OBJ or glTF 2.0 (`.gltf` + `.bin`, or `.glb`).
//!
//! The engine's space is left-handed (x right, y up, z forward) with faces wound clockwise seen
//! from outside. glTF and OBJ viewers expect right-handed space with counter-clockwise faces, so
//! the export negates z and reverses each triangle.

use std::fmt::Write as _;
use std::path::Path;

use a3_p3d::{Lod, Section};
use anyhow::{Context, bail};
use glam::{Vec2, Vec3};

/// Output format, chosen by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Obj,
    Gltf,
    Glb,
}

impl Format {
    pub fn from_path(path: &Path) -> anyhow::Result<Format> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("obj") => Ok(Format::Obj),
            Some("gltf") => Ok(Format::Gltf),
            Some("glb") => Ok(Format::Glb),
            _ => bail!(
                "unknown export format for {} (use .obj, .gltf or .glb)",
                path.display()
            ),
        }
    }
}

/// A LOD converted to right-handed, counter-clockwise geometry. Proxy sections are left out.
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    /// Per primitive: name (texture or material path) and triangle indices.
    pub primitives: Vec<(String, Vec<u32>)>,
}

impl Mesh {
    pub fn from_lod(lod: &Lod) -> Mesh {
        // `+ 0.0` turns -0.0 into 0.0.
        let flip = |v: &Vec3| Vec3::new(v.x, v.y, -v.z + 0.0);
        let v = &lod.vertices;
        let normals = if v.normals.len() == v.len() {
            v.normals.iter().map(flip).collect()
        } else {
            Vec::new()
        };
        let uvs = v
            .uv_sets
            .first()
            .filter(|uv| uv.len() == v.len())
            .cloned()
            .unwrap_or_default();
        let reverse = |tris: Vec<u32>| -> Vec<u32> {
            tris.chunks_exact(3)
                .flat_map(|t| [t[0], t[2], t[1]])
                .collect()
        };
        let mut primitives: Vec<(String, Vec<u32>)> = lod
            .sections
            .iter()
            .filter(|s| !s.is_proxy())
            .map(|s| (section_name(lod, s), reverse(lod.section_triangles(s))))
            .filter(|(_, tris)| !tris.is_empty())
            .collect();
        if lod.sections.is_empty() && !lod.faces.is_empty() {
            primitives.push(("faces".into(), reverse(lod.triangles())));
        }
        Mesh {
            positions: v.positions.iter().map(flip).collect(),
            normals,
            uvs,
            primitives,
        }
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))),
        )
    }
}

fn section_name(lod: &Lod, s: &Section) -> String {
    let texture = s.texture.and_then(|i| lod.textures.get(i as usize));
    let material = s.material.and_then(|i| lod.materials.get(i as usize));
    match (texture, material) {
        (Some(t), _) if !t.is_empty() => t.clone(),
        (_, Some(m)) => m.name.clone(),
        _ => "untextured".into(),
    }
}

pub fn write(mesh: &Mesh, out: &Path) -> anyhow::Result<()> {
    match Format::from_path(out)? {
        Format::Obj => std::fs::write(out, obj(mesh)),
        Format::Gltf => {
            let bin_path = out.with_extension("bin");
            let bin_name = bin_path
                .file_name()
                .and_then(|n| n.to_str())
                .context("output name is not valid UTF-8")?
                .to_owned();
            let (json, bin) = gltf(mesh, Some(&bin_name));
            std::fs::write(&bin_path, bin)
                .with_context(|| format!("writing {}", bin_path.display()))?;
            std::fs::write(out, json)
        }
        Format::Glb => std::fs::write(out, glb(mesh)),
    }
    .with_context(|| format!("writing {}", out.display()))
}

/// Wavefront OBJ text: `v`, `vt`, `vn`, then one `usemtl` group per primitive.
pub fn obj(mesh: &Mesh) -> String {
    let mut s = String::from("# exported by a3-tools p3d export\n");
    for p in &mesh.positions {
        let _ = writeln!(s, "v {} {} {}", p.x, p.y, p.z);
    }
    for t in &mesh.uvs {
        // OBJ's v axis points up; the engine's (like D3D's) points down.
        let _ = writeln!(s, "vt {} {}", t.x, 1.0 - t.y);
    }
    for n in &mesh.normals {
        let _ = writeln!(s, "vn {} {} {}", n.x, n.y, n.z);
    }
    let corner = |i: u32| {
        let i = i + 1;
        match (mesh.uvs.is_empty(), mesh.normals.is_empty()) {
            (true, true) => format!("{i}"),
            (false, true) => format!("{i}/{i}"),
            (true, false) => format!("{i}//{i}"),
            (false, false) => format!("{i}/{i}/{i}"),
        }
    };
    for (name, tris) in &mesh.primitives {
        let _ = writeln!(s, "usemtl {name}");
        for t in tris.chunks_exact(3) {
            let _ = writeln!(s, "f {} {} {}", corner(t[0]), corner(t[1]), corner(t[2]));
        }
    }
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// glTF JSON and its binary buffer. `uri` names the external buffer; `None` for GLB.
pub fn gltf(mesh: &Mesh, uri: Option<&str>) -> (String, Vec<u8>) {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut add = |bin: &mut Vec<u8>, data: Vec<u8>, target: u32, accessor: String| {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{},"target":{target}}}"#,
            bin.len(),
            data.len()
        ));
        bin.extend_from_slice(&data);
        let view = views.len() - 1;
        accessors.push(accessor.replace("VIEW", &view.to_string()));
        accessors.len() - 1
    };
    let floats =
        |v: &mut dyn Iterator<Item = f32>| -> Vec<u8> { v.flat_map(f32::to_le_bytes).collect() };

    let (lo, hi) = mesh.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let n = mesh.positions.len();
    let position = add(
        &mut bin,
        floats(&mut mesh.positions.iter().flat_map(|p| p.to_array())),
        34962,
        format!(
            r#"{{"bufferView":VIEW,"componentType":5126,"count":{n},"type":"VEC3","min":[{},{},{}],"max":[{},{},{}]}}"#,
            lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
        ),
    );
    let mut attributes = format!(r#""POSITION":{position}"#);
    if !mesh.normals.is_empty() {
        let normal = add(
            &mut bin,
            floats(&mut mesh.normals.iter().flat_map(|p| p.to_array())),
            34962,
            format!(r#"{{"bufferView":VIEW,"componentType":5126,"count":{n},"type":"VEC3"}}"#),
        );
        let _ = write!(attributes, r#","NORMAL":{normal}"#);
    }
    if !mesh.uvs.is_empty() {
        let uv = add(
            &mut bin,
            floats(&mut mesh.uvs.iter().flat_map(|p| p.to_array())),
            34962,
            format!(r#"{{"bufferView":VIEW,"componentType":5126,"count":{n},"type":"VEC2"}}"#),
        );
        let _ = write!(attributes, r#","TEXCOORD_0":{uv}"#);
    }
    let mut materials = Vec::new();
    let mut primitives = Vec::new();
    for (name, tris) in &mesh.primitives {
        let indices = add(
            &mut bin,
            tris.iter().flat_map(|i| i.to_le_bytes()).collect(),
            34963,
            format!(
                r#"{{"bufferView":VIEW,"componentType":5125,"count":{},"type":"SCALAR"}}"#,
                tris.len()
            ),
        );
        let material = match materials.iter().position(|m| m == name) {
            Some(i) => i,
            None => {
                materials.push(name.clone());
                materials.len() - 1
            }
        };
        primitives.push(format!(
            r#"{{"attributes":{{{attributes}}},"indices":{indices},"material":{material}}}"#
        ));
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let materials: Vec<String> = materials
        .iter()
        .map(|m| {
            format!(
                r#"{{"name":{},"pbrMetallicRoughness":{{"metallicFactor":0,"roughnessFactor":1}},"doubleSided":false}}"#,
                json_str(m)
            )
        })
        .collect();
    let buffer = match uri {
        Some(uri) => format!(r#"{{"byteLength":{},"uri":{}}}"#, bin.len(), json_str(uri)),
        None => format!(r#"{{"byteLength":{}}}"#, bin.len()),
    };
    // glTF arrays may not be empty: leave `materials` out when there are none.
    let materials = if materials.is_empty() {
        String::new()
    } else {
        format!(r#","materials":[{}]"#, materials.join(","))
    };
    let json = format!(
        concat!(
            r#"{{"asset":{{"version":"2.0","generator":"a3-tools p3d export"}},"#,
            r#""scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"#,
            r#""meshes":[{{"primitives":[{}]}}]{},"#,
            r#""buffers":[{}],"bufferViews":[{}],"accessors":[{}]}}"#
        ),
        primitives.join(","),
        materials,
        buffer,
        views.join(","),
        accessors.join(",")
    );
    (json, bin)
}

/// Binary glTF: header, JSON chunk (space-padded), BIN chunk (zero-padded).
pub fn glb(mesh: &Mesh) -> Vec<u8> {
    let (json, bin) = gltf(mesh, None);
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{Face, Vertices};

    fn quad_lod() -> Lod {
        Lod {
            vertices: Vertices {
                positions: vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 1.0), Vec3::Y],
                normals: vec![Vec3::NEG_Z; 4],
                uv_sets: vec![vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]],
                ..Vertices::default()
            },
            faces: vec![Face::quad(0, 1, 2, 3)],
            sections: vec![Section {
                faces: 0..1,
                texture: Some(0),
                ..Section::default()
            }],
            textures: vec![r"a3\data\box_co.paa".into()],
            ..Lod::default()
        }
    }

    #[test]
    fn converts_to_right_handed_counter_clockwise() {
        let mesh = Mesh::from_lod(&quad_lod());
        assert_eq!(mesh.positions[2], Vec3::new(1.0, 1.0, -1.0), "z negated");
        assert_eq!(mesh.normals[0], Vec3::Z);
        assert_eq!(
            mesh.primitives,
            [(r"a3\data\box_co.paa".to_string(), vec![0, 2, 1, 0, 3, 2])],
            "each triangle reversed"
        );
    }

    #[test]
    fn leaves_out_proxy_sections() {
        let mut lod = quad_lod();
        lod.sections[0].flags = Section::PROXY_FLAG;
        assert!(Mesh::from_lod(&lod).primitives.is_empty());
    }

    #[test]
    fn writes_obj_with_one_based_corners() {
        let text = obj(&Mesh::from_lod(&quad_lod()));
        assert!(text.contains("v 1 1 -1\n"), "{text}");
        assert!(text.contains("vt 0 1\n"), "{text}");
        assert!(
            text.contains("usemtl a3\\data\\box_co.paa\nf 1/1/1 3/3/3 2/2/2\n"),
            "{text}"
        );
    }

    #[test]
    fn writes_a_well_formed_glb() {
        let mesh = Mesh::from_lod(&quad_lod());
        let glb = glb(&mesh);
        assert_eq!(&glb[..4], b"glTF");
        let total = u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize;
        assert_eq!(total, glb.len());
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(json.contains(r#""min":[0,0,-1],"max":[1,1,0]"#), "{json}");
        assert!(json.contains(r#""name":"a3\\data\\box_co.paa""#), "{json}");
        let bin_len = u32::from_le_bytes(glb[20 + json_len..24 + json_len].try_into().unwrap());
        // 4 positions + 4 normals (48 bytes each) + 4 UVs (32) + 6 indices (24).
        assert_eq!(bin_len, 48 + 48 + 32 + 24);
    }
}
