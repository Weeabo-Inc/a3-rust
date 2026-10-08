//! Detail layers: which surface textures each layer material blends, as the GPU sees them.
//!
//! Every layer material (one per WRP material index) has up to five layer slots. Slot `k`
//! names a detail colour texture (`gdt_*_co.paa`) and its normal/parallax map
//! (`gdt_*_nopx.paa`); the tile's mask decides how much of each slot shows
//! (`docs/re/render-terrain.md`, section 3.2). The textures are shared by many materials, so
//! they are collected once into [`DetailLayers::textures`] and the materials refer to them by
//! index.

use std::collections::HashMap;

use a3_core::VfsPath;
use a3_landscape::LayerMaterial;

use crate::satellite::NO_TILE;

/// Layer slots per layer material.
pub const SLOTS: usize = 5;
/// An empty slot in [`MaterialSlots::layers`].
pub const NO_LAYER: u16 = u16::MAX;

/// One surface's detail textures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DetailTexture {
    /// The `_co` detail colour texture.
    pub color: VfsPath,
    /// The `_nopx` normal (rgb) and parallax height (alpha) map.
    pub normal: Option<VfsPath>,
}

/// What one layer material draws near the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialSlots {
    /// The material's satellite tile (index into the tile table) or [`NO_TILE`].
    pub tile: u16,
    /// Per slot: index into [`DetailLayers::textures`] or [`NO_LAYER`].
    pub layers: [u16; SLOTS],
}

impl MaterialSlots {
    pub const EMPTY: MaterialSlots = MaterialSlots {
        tile: NO_TILE,
        layers: [NO_LAYER; SLOTS],
    };
}

/// The detail textures of a terrain and every material's slots.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailLayers {
    /// Unique detail textures, in first-use order.
    pub textures: Vec<DetailTexture>,
    /// Indexed like the WRP material list.
    pub materials: Vec<MaterialSlots>,
}

impl DetailLayers {
    /// Collect the detail textures of `materials` (indexed like the WRP material list);
    /// `material_tiles` gives each material's satellite tile.
    pub fn build(materials: &[Option<LayerMaterial>], material_tiles: &[u16]) -> DetailLayers {
        let mut out = DetailLayers::default();
        let mut index: HashMap<DetailTexture, u16> = HashMap::new();
        let path = |t: &str| (!t.is_empty() && !t.starts_with('#')).then(|| VfsPath::new(t));
        for (m, material) in materials.iter().enumerate() {
            let mut slots = MaterialSlots {
                tile: material_tiles.get(m).copied().unwrap_or(NO_TILE),
                ..MaterialSlots::EMPTY
            };
            for layer in material.iter().flat_map(|m| &m.layers) {
                let Some(color) = path(&layer.color.texture) else {
                    continue;
                };
                if layer.slot >= SLOTS {
                    continue;
                }
                let texture = DetailTexture {
                    color,
                    normal: path(&layer.normal.texture),
                };
                let next = out.textures.len() as u16;
                let id = *index.entry(texture.clone()).or_insert_with(|| {
                    out.textures.push(texture);
                    next
                });
                slots.layers[layer.slot] = id;
            }
            out.materials.push(slots);
        }
        out
    }
}

/// Weights of the five layer slots from a mask sample `m` (RGBA, 0..1) and the slots present
/// in the material, as `PSTerrainSNX` computes them: each present layer paints over the
/// earlier ones with coverage `saturate(3 * channel)`; the first present layer is the base.
/// Slot 1 follows red, 2 green, 3 blue and 4 blue where alpha is low. The weights sum to 1
/// when any layer is present. `terrain.wgsl` (`layer_weights`) computes exactly this.
pub fn layer_weights(m: [f32; 4], present: [bool; SLOTS]) -> [f32; SLOTS] {
    let l = present.map(|p| if p { 1.0f32 } else { 0.0 });
    let sat = |x: f32| x.clamp(0.0, 1.0);
    let mut w = [0.0f32; SLOTS];
    let mut r = (1.0 - l[0]).min(1.0);
    w[0] = sat(3.0 * l[0]);
    let channels = [m[0], m[1], m[2], m[2] * sat(2.0 * (1.0 - m[3]))];
    for k in 1..SLOTS {
        let t = r.max(channels[k - 1]) * l[k];
        r = r.min(1.0 - l[k]);
        let s = sat(3.0 * t);
        for wj in w.iter_mut().take(k) {
            *wj *= 1.0 - s;
        }
        w[k] = s;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(path: &str, slots: &[(usize, &str)]) -> LayerMaterial {
        let mut text = String::from(
            r#"class Stage0 { texture = "s.paa"; texGen = 3; };
               class Stage1 { texture = "m.paa"; texGen = 3; };
               class TexGen3 { uvSource = "worldPos"; };"#,
        );
        for (slot, name) in slots {
            let (n, c) = (3 + 2 * slot, 4 + 2 * slot);
            text += &format!(
                r#"class Stage{n} {{ texture = "a3\map_data\{name}_nopx.paa"; texGen = 1; }};
                   class Stage{c} {{ texture = "a3\map_data\{name}_co.paa"; texGen = 2; }};"#
            );
        }
        // Empty slots up to the last one, as shipped rvmats have them.
        for slot in 0..SLOTS {
            if !slots.iter().any(|(s, _)| *s == slot) {
                let (n, c) = (3 + 2 * slot, 4 + 2 * slot);
                text += &format!(
                    r#"class Stage{n} {{ texture = ""; texGen = 1; }};
                       class Stage{c} {{ texture = ""; texGen = 2; }};"#
                );
            }
        }
        let config = a3_config::parse_text(&text).unwrap();
        LayerMaterial::from_config(path, &config).unwrap()
    }

    #[test]
    fn materials_share_detail_textures_by_slot() {
        let materials = vec![
            None,
            Some(material(
                "p_000-000_l00_n_l02.rvmat",
                &[(0, "gdt_soil"), (2, "gdt_rock")],
            )),
            Some(material("p_001-000_n_l02.rvmat", &[(1, "gdt_rock")])),
        ];
        let layers = DetailLayers::build(&materials, &[NO_TILE, 4, 5]);
        assert_eq!(layers.textures.len(), 2, "gdt_rock is shared");
        assert_eq!(
            layers.textures[1],
            DetailTexture {
                color: VfsPath::new(r"a3\map_data\gdt_rock_co.paa"),
                normal: Some(VfsPath::new(r"a3\map_data\gdt_rock_nopx.paa")),
            }
        );
        assert_eq!(layers.materials[0], MaterialSlots::EMPTY);
        assert_eq!(
            layers.materials[1],
            MaterialSlots {
                tile: 4,
                layers: [0, NO_LAYER, 1, NO_LAYER, NO_LAYER]
            }
        );
        assert_eq!(layers.materials[2].layers[1], 1);
        assert_eq!(layers.materials[2].tile, 5);
    }

    fn close(a: [f32; SLOTS], b: [f32; SLOTS]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn the_first_present_layer_is_the_base() {
        let all = [true; SLOTS];
        assert!(close(
            layer_weights([0.0; 4], all),
            [1.0, 0.0, 0.0, 0.0, 0.0]
        ));
        // Without slot 0 the first present slot takes over where its channel is empty.
        let w = layer_weights([0.0, 0.0, 0.0, 1.0], [false, false, true, true, false]);
        assert!(close(w, [0.0, 0.0, 1.0, 0.0, 0.0]), "{w:?}");
    }

    #[test]
    fn mask_channels_paint_over_earlier_layers() {
        let all = [true; SLOTS];
        // Full red: slot 1 covers the base.
        assert!(close(
            layer_weights([1.0, 0.0, 0.0, 1.0], all),
            [0.0, 1.0, 0.0, 0.0, 0.0]
        ));
        // Green over red.
        assert!(close(
            layer_weights([1.0, 1.0, 0.0, 1.0], all),
            [0.0, 0.0, 1.0, 0.0, 0.0]
        ));
        // Blue with opaque alpha is slot 3; blue with zero alpha is slot 4.
        assert!(close(
            layer_weights([0.0, 0.0, 1.0, 1.0], all),
            [0.0, 0.0, 0.0, 1.0, 0.0]
        ));
        assert!(close(
            layer_weights([0.0, 0.0, 1.0, 0.0], all),
            [0.0, 0.0, 0.0, 0.0, 1.0]
        ));
        // A sixth of red is half coverage.
        let w = layer_weights([1.0 / 6.0, 0.0, 0.0, 1.0], all);
        assert!(close(w, [0.5, 0.5, 0.0, 0.0, 0.0]), "{w:?}");
    }

    #[test]
    fn weights_of_present_layers_sum_to_one() {
        for m in [
            [0.3, 0.1, 0.05, 0.7],
            [0.0, 0.2, 0.9, 0.2],
            [0.1, 0.1, 0.1, 0.9],
        ] {
            for present in [
                [true; SLOTS],
                [true, false, true, false, true],
                [false, true, true, false, false],
            ] {
                let w = layer_weights(m, present);
                let sum: f32 = w.iter().sum();
                assert!((sum - 1.0).abs() < 1e-5, "{m:?} {present:?}: {w:?}");
                for (k, p) in present.iter().enumerate() {
                    assert!(*p || w[k] == 0.0, "absent slot {k} has weight");
                }
            }
        }
    }
}
