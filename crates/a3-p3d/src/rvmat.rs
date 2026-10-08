//! `.rvmat` material files: a config (rapified in shipped data) naming shaders, colours and
//! texture stages.

use a3_config::{Config, ConfigClass, EntryKind, Value};
use glam::Vec4;

use crate::error::{Error, Result};

/// A material read from an `.rvmat` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RvMat {
    /// `PixelShaderID` (e.g. `Super`, `NormalMap`).
    pub pixel_shader: String,
    /// `VertexShaderID`.
    pub vertex_shader: String,
    /// `ambient[]`.
    pub ambient: Vec4,
    /// `diffuse[]`.
    pub diffuse: Vec4,
    /// `forcedDiffuse[]`.
    pub forced_diffuse: Vec4,
    /// `emmisive[]` (spelled that way in the files).
    pub emissive: Vec4,
    /// `specular[]`.
    pub specular: Vec4,
    /// `specularPower`.
    pub specular_power: f32,
    /// `surfaceInfo` (a `.bisurf` path).
    pub surface: String,
    /// `StageN` classes in index order.
    pub stages: Vec<RvMatStage>,
    /// `StageTI` (thermal imaging).
    pub ti_stage: Option<RvMatStage>,
}

/// One `StageN` class of an [`RvMat`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RvMatStage {
    /// The `N` of `StageN` (0 for `StageTI`).
    pub index: u32,
    /// `texture`: a path or a procedural texture such as `#(argb,8,8,3)color(...)`.
    pub texture: String,
    /// `uvSource` (`tex`, `tex1`, `none`, ...).
    pub uv_source: String,
    /// `uvTransform` rows `aside`, `up`, `dir`, `pos`.
    pub uv_transform: Option<[[f32; 3]; 4]>,
}

impl RvMat {
    /// Reads an `.rvmat` file, rapified or text (text must need no preprocessing).
    pub fn from_bytes(bytes: &[u8]) -> Result<RvMat> {
        let config = if a3_config::is_rap(bytes) {
            a3_config::read_rap(bytes).map_err(|e| Error::Rvmat(e.to_string()))?
        } else {
            let text = String::from_utf8_lossy(bytes);
            a3_config::parse_text(&text).map_err(|e| Error::Rvmat(e.to_string()))?
        };
        Ok(Self::from_config(&config))
    }

    /// Reads the material entries of a parsed config. Missing entries keep their defaults.
    pub fn from_config(config: &Config) -> RvMat {
        let root = &config.root;
        let mut stages: Vec<RvMatStage> = root
            .entries
            .iter()
            .filter_map(|e| {
                let EntryKind::Class(class) = &e.kind else {
                    return None;
                };
                let lower = e.name.to_ascii_lowercase();
                let index = lower.strip_prefix("stage")?.parse::<u32>().ok()?;
                Some(stage(index, class))
            })
            .collect();
        stages.sort_by_key(|s| s.index);
        RvMat {
            pixel_shader: text(root, "PixelShaderID"),
            vertex_shader: text(root, "VertexShaderID"),
            ambient: vec4(root, "ambient"),
            diffuse: vec4(root, "diffuse"),
            forced_diffuse: vec4(root, "forcedDiffuse"),
            emissive: vec4(root, "emmisive"),
            specular: vec4(root, "specular"),
            specular_power: number(value(root, "specularPower")).unwrap_or(0.0),
            surface: text(root, "surfaceInfo"),
            stages,
            ti_stage: root.class("StageTI").map(|c| stage(0, c)),
        }
    }
}

fn stage(index: u32, class: &ConfigClass) -> RvMatStage {
    let uv_transform = class.class("uvTransform").map(|t| {
        ["aside", "up", "dir", "pos"].map(|row| {
            let v = numbers(value(t, row));
            [0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0.0))
        })
    });
    RvMatStage {
        index,
        texture: text(class, "texture"),
        uv_source: text(class, "uvSource"),
        uv_transform,
    }
}

fn value<'a>(class: &'a ConfigClass, name: &str) -> Option<&'a Value> {
    match &class.get(name)?.kind {
        EntryKind::Value(v) => Some(v),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
        Value::Array(_) => None,
    }
}

fn numbers(v: Option<&Value>) -> Vec<f32> {
    match v {
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| number(Some(i)).unwrap_or(0.0))
            .collect(),
        _ => Vec::new(),
    }
}

fn vec4(class: &ConfigClass, name: &str) -> Vec4 {
    let v = numbers(value(class, name));
    Vec4::from_array([0, 1, 2, 3].map(|i| v.get(i).copied().unwrap_or(0.0)))
}

fn text(class: &ConfigClass, name: &str) -> String {
    match value(class, name) {
        Some(Value::String(s) | Value::Expression(s)) => s.clone(),
        Some(other) => number(Some(other))
            .map(|n| n.to_string())
            .unwrap_or_default(),
        None => String::new(),
    }
}
