//! Turning the open displays into textured, coloured quads in screen pixels.
//!
//! The output is renderer-independent: quads name their texture by VFS path (or procedural
//! `#(argb,...)` string) through [`TextureKey`]; the renderer loads and caches them. Glyph
//! quads give their texture coordinates in texels ([`Uv::Texels`]) because the page size is
//! only known once the texture is loaded.

use std::collections::HashMap;

use crate::kinds::{ControlType, style};
use crate::model::{Control, Rgba};
use crate::text::Fonts;
use crate::ui::Ui;

/// An interned texture path in a [`DrawList`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureKey(pub u32);

/// Texture coordinates of a quad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Uv {
    /// `[u0, v0, u1, v1]` in 0..1.
    Normalized([f32; 4]),
    /// `[x, y, w, h]` in texels of the texture.
    Texels([f32; 4]),
}

/// The whole texture.
pub const FULL: Uv = Uv::Normalized([0.0, 0.0, 1.0, 1.0]);

/// One quad to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    /// `[x, y, w, h]` in screen pixels.
    pub rect: [f32; 4],
    pub uv: Uv,
    /// Multiplied with the texture (or the colour itself without one). sRGB, not
    /// premultiplied.
    pub color: Rgba,
    pub texture: Option<TextureKey>,
    /// Scissor rectangle `[x, y, w, h]` in pixels.
    pub clip: Option<[f32; 4]>,
    /// Rotation about the quad centre, degrees clockwise.
    pub angle: f32,
}

/// One frame of UI quads, in drawing order.
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    pub quads: Vec<Quad>,
    textures: Vec<String>,
    index: HashMap<String, u32>,
}

impl DrawList {
    /// The key of texture `path` (normalized: no leading backslash, lower case).
    pub fn texture(&mut self, path: &str) -> TextureKey {
        let normalized = normalize_texture_path(path);
        if let Some(&i) = self.index.get(&normalized) {
            return TextureKey(i);
        }
        let i = self.textures.len() as u32;
        self.textures.push(normalized.clone());
        self.index.insert(normalized, i);
        TextureKey(i)
    }

    /// The path of a key.
    pub fn texture_path(&self, key: TextureKey) -> &str {
        &self.textures[key.0 as usize]
    }

    /// Every texture used, indexed by key.
    pub fn textures(&self) -> &[String] {
        &self.textures
    }

    fn solid(&mut self, rect: [f32; 4], color: Rgba, clip: Option<[f32; 4]>) {
        if color[3] <= 0.0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }
        self.quads.push(Quad {
            rect,
            uv: FULL,
            color,
            texture: None,
            clip,
            angle: 0.0,
        });
    }

    fn image(
        &mut self,
        rect: [f32; 4],
        path: &str,
        color: Rgba,
        clip: Option<[f32; 4]>,
        angle: f32,
    ) {
        if path.trim().is_empty() || color[3] <= 0.0 {
            return;
        }
        let texture = Some(self.texture(path));
        self.quads.push(Quad {
            rect,
            uv: FULL,
            color,
            texture,
            clip,
            angle,
        });
    }
}

/// Texture path as the renderer's cache key: no leading backslash, lower case; procedural
/// textures (`#(...)`) unchanged.
pub fn normalize_texture_path(path: &str) -> String {
    let path = path.trim();
    if path.starts_with('#') {
        return path.to_owned();
    }
    path.trim_start_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn with_alpha(mut c: Rgba, alpha: f32) -> Rgba {
    c[3] *= alpha;
    c
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// Horizontal alignment of text in a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Text drawing parameters.
#[derive(Debug, Clone)]
pub struct TextStyle<'a> {
    pub font: &'a str,
    /// Line height in pixels.
    pub pixels: f32,
    pub color: Rgba,
    pub align: Align,
    /// Vertical centring in the rectangle (otherwise top-aligned).
    pub vcenter: bool,
    /// 0 none, 1 drop shadow, 2 outline.
    pub shadow: i32,
    pub shadow_color: Rgba,
    /// Wrap at the rectangle width.
    pub multiline: bool,
    pub line_spacing: f32,
}

/// Draws `text` in `rect` (pixels).
pub fn draw_text(
    list: &mut DrawList,
    fonts: &mut Fonts,
    text: &str,
    rect: [f32; 4],
    style: &TextStyle<'_>,
    clip: Option<[f32; 4]>,
) {
    if text.is_empty() || style.color[3] <= 0.0 || style.pixels < 1.0 {
        return;
    }
    let lines: Vec<String> = if style.multiline {
        fonts.wrap(style.font, style.pixels, text, rect[2])
    } else {
        vec![text.replace('\n', " ")]
    };
    let line_step = style.pixels * style.line_spacing.max(0.1);
    let total = line_step * lines.len() as f32;
    let mut y = if style.vcenter {
        rect[1] + (rect[3] - total) / 2.0
    } else {
        rect[1]
    };
    // Text is clipped to its control.
    let clip = Some(clip.map_or(rect, |c| intersect(c, rect)));
    for line in lines {
        let laid = fonts.layout_line(style.font, style.pixels, &line);
        let x = match style.align {
            Align::Left => rect[0],
            Align::Center => rect[0] + (rect[2] - laid.width) / 2.0,
            Align::Right => rect[0] + rect[2] - laid.width,
        };
        let (x, y0) = (x.round(), y.round());
        let emit = |dx: f32, dy: f32, color: Rgba, list: &mut DrawList| {
            for g in &laid.glyphs {
                let texture = Some(list.texture(&g.texture));
                list.quads.push(Quad {
                    rect: [x + g.dst[0] + dx, y0 + g.dst[1] + dy, g.dst[2], g.dst[3]],
                    uv: Uv::Texels(g.src),
                    color,
                    texture,
                    clip,
                    angle: 0.0,
                });
            }
        };
        let offset = (style.pixels / 24.0).max(1.0).round();
        match style.shadow {
            1 => emit(offset, offset, style.shadow_color, list),
            2 => {
                for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    emit(dx * offset, dy * offset, style.shadow_color, list);
                }
            }
            _ => {}
        }
        emit(0.0, 0.0, style.color, list);
        y += line_step;
    }
}

/// Strips structured-text markup to plain text (`<br/>` becomes a line break; other tags are
/// removed; `&amp;`, `&lt;`, `&gt;`, `&quot;` decoded). Returns the text and the alignment of
/// the first `<t align=...>` tag, if any.
pub fn plain_structured_text(text: &str) -> (String, Option<Align>) {
    let mut out = String::new();
    let mut align = None;
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        if tag.starts_with("br") {
            out.push('\n');
        }
        if align.is_none() && tag.starts_with('t') {
            if tag.contains("align='center'") || tag.contains("align=\"center\"") {
                align = Some(Align::Center);
            } else if tag.contains("align='right'") || tag.contains("align=\"right\"") {
                align = Some(Align::Right);
            }
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&");
    (decoded, align)
}

fn text_case(c: &Control, text: &str) -> String {
    match c.style & style::TYPE {
        style::UPPERCASE => text.to_uppercase(),
        style::LOWERCASE => text.to_lowercase(),
        _ => text.to_owned(),
    }
}

fn align_of(c: &Control) -> Align {
    match c.style & style::HPOS {
        style::RIGHT => Align::Right,
        style::CENTER => Align::Center,
        _ => Align::Left,
    }
}

/// Builds the quads of every open display, bottom display first.
pub fn build_draw_list(ui: &Ui, fonts: &mut Fonts) -> DrawList {
    let mut list = DrawList::default();
    let m = ui.metrics;
    for &display in ui.stack() {
        for id in ui.controls_in_order(display) {
            if !ui.is_visible(id) {
                continue;
            }
            let (Some(c), Some(pos)) = (ui.control(id), ui.absolute_position(id)) else {
                continue;
            };
            let alpha = ui.opacity(id);
            if alpha <= 0.0 {
                continue;
            }
            let rect = m.rect_to_px(pos);
            let clip = ui.clip_rect(id).map(|r| m.rect_to_px(r));
            draw_control(&mut list, fonts, ui, c, rect, clip, alpha);
        }
    }
    list
}

fn draw_control(
    list: &mut DrawList,
    fonts: &mut Fonts,
    ui: &Ui,
    c: &Control,
    rect: [f32; 4],
    clip: Option<[f32; 4]>,
    alpha: f32,
) {
    let m = ui.metrics;
    let text_color = with_alpha(
        if c.enabled {
            c.color_text
        } else {
            c.color_disabled
        },
        alpha,
    );
    let pixels = c.size_ex * m.viewport_h;
    let kind = c.style & style::TYPE;
    let base_style = TextStyle {
        font: &c.font,
        pixels,
        color: text_color,
        align: align_of(c),
        vcenter: true,
        shadow: c.shadow,
        shadow_color: with_alpha(c.color_shadow, alpha),
        multiline: kind == style::MULTI,
        line_spacing: c.line_spacing,
    };
    match c.kind {
        ControlType::ControlsGroup | ControlType::ControlsTable => {
            list.solid(rect, with_alpha(c.color_background, alpha), clip);
        }
        ControlType::ShortcutButton | ControlType::XButton => {
            if !c.texture_normal.is_empty() {
                list.image(
                    rect,
                    &c.texture_normal,
                    with_alpha(c.color_background, alpha),
                    clip,
                    0.0,
                );
            } else {
                list.solid(rect, with_alpha(c.color_background, alpha), clip);
            }
            if !c.picture.is_empty() && !c.picture.starts_with('#') {
                list.image(rect, &c.picture, text_color, clip, 0.0);
            }
            let inset = c.text_pos.unwrap_or([0.0; 4]);
            let text_rect = [
                rect[0] + inset[0] * m.viewport_w,
                rect[1] + inset[1] * m.viewport_h,
                (rect[2] - (inset[0] + inset[2]) * m.viewport_w).max(0.0),
                (rect[3] - (inset[1] + inset[3]) * m.viewport_h).max(0.0),
            ];
            let (plain, align) = plain_structured_text(&c.text);
            let style = TextStyle {
                align: align.unwrap_or(base_style.align),
                vcenter: false,
                ..base_style.clone()
            };
            draw_text(list, fonts, &text_case(c, &plain), text_rect, &style, clip);
        }
        ControlType::StructuredText => {
            list.solid(rect, with_alpha(c.color_background, alpha), clip);
            let (plain, align) = plain_structured_text(&c.text);
            let style = TextStyle {
                align: align.unwrap_or(Align::Left),
                vcenter: false,
                multiline: true,
                ..base_style
            };
            draw_text(list, fonts, &plain, rect, &style, clip);
        }
        ControlType::Progress => {
            list.solid(rect, with_alpha(c.color_background, alpha), clip);
            let span = (c.range[1] - c.range[0]).max(f32::EPSILON);
            let t = ((c.value - c.range[0]) / span).clamp(0.0, 1.0);
            list.solid([rect[0], rect[1], rect[2] * t, rect[3]], text_color, clip);
        }
        ControlType::ListBox
        | ControlType::Combo
        | ControlType::XListBox
        | ControlType::ListNBox => {
            list.solid(rect, with_alpha(c.color_background, alpha), clip);
            let row = pixels.max(1.0);
            let shown: Box<dyn Iterator<Item = &crate::model::ListItem>> =
                if c.kind == ControlType::Combo {
                    Box::new(c.items.get(c.cur_sel.max(0) as usize).into_iter())
                } else {
                    Box::new(c.items.iter())
                };
            for (i, item) in shown.enumerate() {
                let y = rect[1] + row * i as f32;
                if y > rect[1] + rect[3] {
                    break;
                }
                let color = item.color.map_or(text_color, |col| with_alpha(col, alpha));
                let style = TextStyle {
                    color,
                    align: Align::Left,
                    ..base_style.clone()
                };
                draw_text(
                    list,
                    fonts,
                    &item.text,
                    [rect[0], y, rect[2], row],
                    &style,
                    clip,
                );
            }
        }
        _ => {
            if kind == style::PICTURE || kind == style::TILE_PICTURE {
                list.solid(rect, with_alpha(c.color_background, alpha), clip);
                list.image(rect, &c.text, text_color, clip, c.angle);
                return;
            }
            if kind == style::FRAME || kind == style::GROUP_BOX {
                let t = 1.0;
                for r in [
                    [rect[0], rect[1], rect[2], t],
                    [rect[0], rect[1] + rect[3] - t, rect[2], t],
                    [rect[0], rect[1], t, rect[3]],
                    [rect[0] + rect[2] - t, rect[1], t, rect[3]],
                ] {
                    list.solid(r, text_color, clip);
                }
                return;
            }
            list.solid(rect, with_alpha(c.color_background, alpha), clip);
            if kind == style::LINE {
                list.solid(
                    [rect[0], rect[1], rect[2].max(1.0), rect[3].max(1.0)],
                    text_color,
                    clip,
                );
                return;
            }
            draw_text(list, fonts, &text_case(c, &c.text), rect, &base_style, clip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_text_is_flattened() {
        let (t, a) = plain_structured_text("<t align='center' size='2'>Hello</t><br/>A &amp; B");
        assert_eq!(t, "Hello\nA & B");
        assert_eq!(a, Some(Align::Center));
    }

    #[test]
    fn texture_paths_are_normalized() {
        let mut list = DrawList::default();
        let a = list.texture(r"\A3\Ui_f\data\x.paa");
        let b = list.texture(r"a3\ui_f\DATA\x.paa");
        assert_eq!(a, b);
        assert_eq!(list.texture_path(a), r"a3\ui_f\data\x.paa");
        let p = list.texture("#(argb,8,8,3)color(1,1,1,1)");
        assert_eq!(list.texture_path(p), "#(argb,8,8,3)color(1,1,1,1)");
    }
}
