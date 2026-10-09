//! Per-frame draw submissions: meshes, debug lines and screen text.

use glam::{DAffine3, DVec3, Vec3};

/// Handle of a mesh uploaded with [`Renderer::upload_mesh`](crate::Renderer::upload_mesh).
/// Stale after [`Renderer::remove_mesh`](crate::Renderer::remove_mesh): draws with it are
/// skipped, even once the slot is reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// A texture for [`MeshDraw`]s: uploaded with
/// [`Renderer::upload_texture`](crate::Renderer::upload_texture), or streamed
/// ([`TextureHandle::texture_id`](crate::TextureHandle::texture_id)). Stale or not yet loaded
/// textures draw white.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub(crate) TextureRef);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum TextureRef {
    Uploaded { index: u32, generation: u32 },
    Streamed { slot: u32, generation: u32 },
}

impl TextureId {
    pub(crate) fn uploaded(index: u32, generation: u32) -> TextureId {
        TextureId(TextureRef::Uploaded { index, generation })
    }

    pub(crate) fn streamed(slot: u32, generation: u32) -> TextureId {
        TextureId(TextureRef::Streamed { slot, generation })
    }
}

/// Generational storage: freed slots are reused with a bumped generation, so old ids stay
/// invalid.
#[derive(Debug)]
pub(crate) struct Slots<T> {
    items: Vec<Option<T>>,
    generations: Vec<u32>,
    free: Vec<u32>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Slots {
            items: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slots<T> {
    /// Store `item`; returns (index, generation).
    pub fn insert(&mut self, item: T) -> (u32, u32) {
        let index = self.free.pop().unwrap_or_else(|| {
            self.items.push(None);
            self.generations.push(0);
            self.items.len() as u32 - 1
        });
        self.items[index as usize] = Some(item);
        (index, self.generations[index as usize])
    }

    pub fn get(&self, index: u32, generation: u32) -> Option<&T> {
        (self.generations.get(index as usize) == Some(&generation))
            .then(|| self.items[index as usize].as_ref())
            .flatten()
    }

    /// Remove and return the item if `(index, generation)` is current.
    pub fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        self.get(index, generation)?;
        let item = self.items[index as usize].take();
        self.generations[index as usize] = generation.wrapping_add(1);
        self.free.push(index);
        item
    }

    pub fn len(&self) -> usize {
        self.items.iter().flatten().count()
    }
}

/// Linear RGBA colour.
pub type Color = [f32; 4];

/// One mesh instance to draw this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDraw {
    pub mesh: MeshId,
    /// Base colour texture; `None` samples white.
    pub texture: Option<TextureId>,
    /// Model-to-world transform in `f64` world space.
    pub transform: DAffine3,
    /// Multiplied with the texture colour.
    pub color: Color,
    /// Draw in the alpha phase: blended, sorted back to front, no depth writes.
    pub transparent: bool,
}

impl MeshDraw {
    /// Opaque, untextured instance at `position`.
    pub fn at(mesh: MeshId, position: DVec3, color: Color) -> MeshDraw {
        MeshDraw {
            mesh,
            texture: None,
            transform: DAffine3::from_translation(position),
            color,
            transparent: false,
        }
    }
}

/// A run of screen text in pixels from the top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    /// Integer pixel scale of the 5x7 debug font.
    pub scale: f32,
    pub color: Color,
    pub text: String,
}

/// World-space line segments for debugging.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DebugLines {
    pub(crate) segments: Vec<(DVec3, DVec3, Color)>,
}

impl DebugLines {
    pub fn line(&mut self, a: DVec3, b: DVec3, color: Color) {
        self.segments.push((a, b, color));
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Edges of an axis-aligned box.
    pub fn aabb(&mut self, min: DVec3, max: DVec3, color: Color) {
        let c = |x: bool, y: bool, z: bool| {
            DVec3::new(
                if x { max.x } else { min.x },
                if y { max.y } else { min.y },
                if z { max.z } else { min.z },
            )
        };
        for a in [false, true] {
            for b in [false, true] {
                self.line(c(false, a, b), c(true, a, b), color);
                self.line(c(a, false, b), c(a, true, b), color);
                self.line(c(a, b, false), c(a, b, true), color);
            }
        }
    }

    /// Three great circles of a sphere.
    pub fn wire_sphere(&mut self, center: DVec3, radius: f64, segments: u32, color: Color) {
        let segments = segments.max(3);
        let point = |axis: usize, t: f64| {
            let (s, c) = t.sin_cos();
            let v = match axis {
                0 => DVec3::new(0.0, s, c),
                1 => DVec3::new(s, 0.0, c),
                _ => DVec3::new(s, c, 0.0),
            };
            center + v * radius
        };
        for axis in 0..3 {
            for i in 0..segments {
                let t0 = std::f64::consts::TAU * f64::from(i) / f64::from(segments);
                let t1 = std::f64::consts::TAU * f64::from(i + 1) / f64::from(segments);
                self.line(point(axis, t0), point(axis, t1), color);
            }
        }
    }

    /// World axes at `origin`: X east red, Y up green, Z north blue.
    pub fn axes(&mut self, origin: DVec3, length: f64) {
        self.line(origin, origin + DVec3::X * length, [1.0, 0.1, 0.1, 1.0]);
        self.line(origin, origin + DVec3::Y * length, [0.1, 1.0, 0.1, 1.0]);
        self.line(origin, origin + DVec3::Z * length, [0.2, 0.4, 1.0, 1.0]);
    }

    /// Horizontal grid at height `y`, `2 * half_cells` cells of `spacing` per side, snapped to
    /// multiples of `spacing` around `center` so it stays put as the camera moves.
    pub fn grid(&mut self, center: DVec3, y: f64, spacing: f64, half_cells: u32, color: Color) {
        let snap = |v: f64| (v / spacing).round() * spacing;
        let (cx, cz) = (snap(center.x), snap(center.z));
        let extent = spacing * f64::from(half_cells);
        for i in 0..=2 * half_cells {
            let o = -extent + spacing * f64::from(i);
            self.line(
                DVec3::new(cx + o, y, cz - extent),
                DVec3::new(cx + o, y, cz + extent),
                color,
            );
            self.line(
                DVec3::new(cx - extent, y, cz + o),
                DVec3::new(cx + extent, y, cz + o),
                color,
            );
        }
    }
}

/// Everything to draw in one frame, collected by the game and consumed by
/// [`Renderer::render`](crate::Renderer::render).
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    pub meshes: Vec<MeshDraw>,
    pub lines: DebugLines,
    pub text: Vec<TextRun>,
}

impl DrawList {
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.lines.segments.clear();
        self.text.clear();
    }

    pub fn mesh(&mut self, draw: MeshDraw) {
        self.meshes.push(draw);
    }

    pub fn text(&mut self, x: f32, y: f32, scale: f32, color: Color, text: impl Into<String>) {
        self.text.push(TextRun {
            x,
            y,
            scale,
            color,
            text: text.into(),
        });
    }
}

/// Camera-relative `f32` model matrix columns for a world transform.
pub(crate) fn relative_model(transform: &DAffine3, camera: DVec3) -> [[f32; 4]; 4] {
    let m = transform.matrix3.as_mat3();
    let t: Vec3 = (transform.translation - camera).as_vec3();
    [
        m.x_axis.extend(0.0).to_array(),
        m.y_axis.extend(0.0).to_array(),
        m.z_axis.extend(0.0).to_array(),
        t.extend(1.0).to_array(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_reuse_indices_and_reject_stale_ids() {
        let mut s = Slots::default();
        let a = s.insert("a");
        let b = s.insert("b");
        assert_eq!(s.get(a.0, a.1), Some(&"a"));
        assert_eq!(s.remove(a.0, a.1), Some("a"));
        assert_eq!(s.get(a.0, a.1), None);
        assert_eq!(s.remove(a.0, a.1), None, "double remove");
        let c = s.insert("c");
        assert_eq!(c.0, a.0, "slot reused");
        assert_ne!(c.1, a.1, "with a new generation");
        assert_eq!(s.get(a.0, a.1), None, "old id stays stale");
        assert_eq!(s.get(c.0, c.1), Some(&"c"));
        assert_eq!(s.len(), 2);
        let _ = b;
    }

    #[test]
    fn aabb_has_twelve_edges() {
        let mut l = DebugLines::default();
        l.aabb(DVec3::ZERO, DVec3::ONE, [1.0; 4]);
        assert_eq!(l.len(), 12);
        for (a, b, _) in &l.segments {
            assert!(((*a - *b).length() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn grid_snaps_to_spacing() {
        let mut l = DebugLines::default();
        l.grid(DVec3::new(1234.0, 0.0, 87.0), 0.0, 100.0, 1, [1.0; 4]);
        assert_eq!(l.len(), 6);
        assert!(l.segments.iter().any(|(a, _, _)| a.x == 1100.0));
        assert!(
            l.segments
                .iter()
                .all(|(a, b, _)| a.z.min(b.z) >= 0.0 && a.z.max(b.z) <= 200.0)
        );
    }

    #[test]
    fn relative_model_moves_translation_next_to_camera() {
        let t = DAffine3::from_translation(DVec3::new(30_000.5, 10.0, 30_000.25));
        let m = relative_model(&t, DVec3::new(30_000.0, 0.0, 30_000.0));
        assert_eq!(m[3], [0.5, 10.0, 0.25, 1.0]);
        assert_eq!(m[0], [1.0, 0.0, 0.0, 0.0]);
    }
}
