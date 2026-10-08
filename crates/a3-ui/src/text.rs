//! Fonts (`CfgFontFamilies` + FXY) and text layout.
//!
//! A font family lists one FXY font per pixel size (`fonts[]`), each optionally with fallback
//! fonts for characters it lacks (CJK). Text of height `sizeEx` (UI units) uses the size
//! whose line height (the FXY page `height` metric) is closest to `sizeEx` in pixels, scaled to
//! fit exactly _(uncertain: the engine's choice and scaling, #155)_.
//!
//! In BIFo 0x102 fonts the glyph offsets place the image relative to the pen: `offset_x` from
//! the pen position, `offset_y` from the top of the line (the page `ascent` is the baseline);
//! see `docs/re/ui.md`.

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_fonts::{Font, page_texture_path};

/// Loads FXY files (the game reads them from the VFS).
pub trait FontLoader {
    /// The font at `fxy_path` (a path without or with the `.fxy` extension).
    fn load_font(&mut self, fxy_path: &str) -> Option<Font>;
}

/// One loaded FXY font with its derived metrics.
#[derive(Debug)]
pub struct LoadedFont {
    /// The path as listed in config (without `.fxy`).
    pub path: String,
    pub font: Font,
    /// Line height in pixels.
    pub line_height: f32,
    /// Baseline from the top of the line in pixels.
    pub ascent: f32,
    glyphs: HashMap<u16, usize>,
}

impl LoadedFont {
    fn new(path: &str, font: Font) -> Self {
        let page = font.pages.first();
        let max_h = font.glyphs.iter().map(|g| g.height).max().unwrap_or(0) as f32;
        let line_height = page
            .map(|p| p.height as f32)
            .filter(|&h| h > 0.0)
            .unwrap_or(max_h);
        let ascent = page
            .map(|p| p.ascent as f32)
            .filter(|&a| a > 0.0)
            .unwrap_or(line_height * 0.8);
        let glyphs = font
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| (g.code, i))
            .collect();
        Self {
            path: path.to_owned(),
            font,
            line_height: line_height.max(1.0),
            ascent,
            glyphs,
        }
    }

    fn glyph(&self, c: char) -> Option<&a3_fonts::Glyph> {
        let code = u16::try_from(u32::from(c)).ok()?;
        self.glyphs.get(&code).map(|&i| &self.font.glyphs[i])
    }
}

/// A font family from `CfgFontFamilies`.
#[derive(Debug, Clone, Default)]
pub struct FontFamily {
    pub name: String,
    /// Per size: the main font and its fallbacks.
    pub sizes: Vec<Vec<String>>,
    /// `spaceWidth`, relative to the font's space advance (default 1).
    pub space_width: f32,
    /// `spacing`, extra pen advance in multiples of the line height _(uncertain)_.
    pub spacing: f32,
}

/// One glyph image of laid-out text.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphQuad {
    /// Font page texture (`<font>-NN.paa`).
    pub texture: String,
    /// Source rectangle on the page in texels `[x, y, w, h]`.
    pub src: [f32; 4],
    /// Destination relative to the line's top-left corner, in pixels `[x, y, w, h]`.
    pub dst: [f32; 4],
}

/// A laid-out line of text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextLine {
    pub glyphs: Vec<GlyphQuad>,
    /// Pen advance, pixels.
    pub width: f32,
    /// Line height, pixels.
    pub height: f32,
}

/// The font families and the fonts loaded so far.
pub struct Fonts {
    families: HashMap<String, FontFamily>,
    cache: HashMap<String, Option<Arc<LoadedFont>>>,
    loader: Box<dyn FontLoader>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("families", &self.families.len())
            .field("loaded", &self.cache.len())
            .finish()
    }
}

/// The text of a config value that is a string or an array of strings (first element).
fn font_list(v: &a3_config::Value) -> Vec<String> {
    match v {
        a3_config::Value::String(s) => vec![s.clone()],
        a3_config::Value::Array(items) => items
            .iter()
            .filter_map(|i| match i {
                a3_config::Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Fonts {
    /// The families of `configFile >> "CfgFontFamilies"`, loading fonts through `loader`.
    pub fn from_config(config: &ConfigTree, loader: Box<dyn FontLoader>) -> Self {
        let mut families = HashMap::new();
        let cfg = config.root() >> "CfgFontFamilies";
        for class in cfg.entries().into_iter().filter(|c| c.is_class()) {
            let sizes: Vec<Vec<String>> = (&class >> "fonts")
                .array()
                .iter()
                .map(font_list)
                .filter(|l| !l.is_empty())
                .collect();
            let space = &class >> "spaceWidth";
            let spacing = &class >> "spacing";
            families.insert(
                class.name().to_ascii_lowercase(),
                FontFamily {
                    name: class.name().to_owned(),
                    sizes,
                    space_width: if space.is_null() { 1.0 } else { space.number() },
                    spacing: if spacing.is_null() {
                        0.0
                    } else {
                        spacing.number()
                    },
                },
            );
        }
        Self {
            families,
            cache: HashMap::new(),
            loader,
        }
    }

    /// The family `name` (case-insensitive).
    pub fn family(&self, name: &str) -> Option<&FontFamily> {
        self.families.get(&name.to_ascii_lowercase())
    }

    fn load(&mut self, path: &str) -> Option<Arc<LoadedFont>> {
        let key = path.to_ascii_lowercase();
        if let Some(f) = self.cache.get(&key) {
            return f.clone();
        }
        let loaded = self
            .loader
            .load_font(path)
            .map(|font| Arc::new(LoadedFont::new(path, font)));
        self.cache.insert(key, loaded.clone());
        loaded
    }

    /// The fonts (main first, then fallbacks) of `family` best matching `pixels` line height,
    /// with the scale to apply.
    pub fn select(&mut self, family: &str, pixels: f32) -> Option<(Vec<Arc<LoadedFont>>, f32)> {
        let fam = self
            .family(family)
            .or_else(|| self.family("RobotoCondensed"))?
            .clone();
        let mut best: Option<(Vec<Arc<LoadedFont>>, f32)> = None;
        let mut best_error = f32::MAX;
        for size in &fam.sizes {
            let Some(main) = size.first().and_then(|p| self.load(p)) else {
                continue;
            };
            let error = (main.line_height - pixels).abs();
            if error < best_error {
                best_error = error;
                let mut fonts = vec![main];
                fonts.extend(size.iter().skip(1).filter_map(|p| self.load(p)));
                best = Some((fonts, 0.0));
            }
        }
        let (fonts, _) = best?;
        let scale = pixels / fonts[0].line_height;
        Some((fonts, scale))
    }

    /// Lays out one line of `text` in `family` with a line height of `pixels`.
    pub fn layout_line(&mut self, family: &str, pixels: f32, text: &str) -> TextLine {
        let mut line = TextLine {
            height: pixels,
            ..TextLine::default()
        };
        let Some((fonts, scale)) = self.select(family, pixels) else {
            return line;
        };
        let space_width = self.family(family).map_or(1.0, |f| f.space_width);
        let main = &fonts[0];
        let mut pen = 0.0f32;
        for c in text.chars() {
            let Some((font, glyph)) = fonts
                .iter()
                .find_map(|f| f.glyph(c).map(|g| (f, g)))
                .or_else(|| main.glyph('?').map(|g| (main, g)))
            else {
                continue;
            };
            // Fallback fonts have their own metrics; align their baseline with the main font.
            let font_scale = scale * main.line_height / font.line_height;
            let baseline_shift = main.ascent * scale - font.ascent * font_scale;
            if glyph.width > 0 && glyph.height > 0 {
                line.glyphs.push(GlyphQuad {
                    texture: page_texture_path(&format!("{}.fxy", font.path), glyph.page),
                    src: [
                        glyph.x as f32,
                        glyph.y as f32,
                        glyph.width as f32,
                        glyph.height as f32,
                    ],
                    dst: [
                        pen + glyph.offset_x as f32 * font_scale,
                        baseline_shift + glyph.offset_y as f32 * font_scale,
                        glyph.width as f32 * font_scale,
                        glyph.height as f32 * font_scale,
                    ],
                });
            }
            let advance = glyph.advance as f32 * font_scale;
            pen += if c == ' ' {
                advance * space_width
            } else {
                advance
            };
        }
        line.width = pen;
        line
    }

    /// Width in pixels of `text` (`getTextWidth`-style measurement).
    pub fn measure(&mut self, family: &str, pixels: f32, text: &str) -> f32 {
        self.layout_line(family, pixels, text).width
    }

    /// Splits `text` into lines no wider than `width` pixels at spaces (multi-line statics).
    pub fn wrap(&mut self, family: &str, pixels: f32, text: &str, width: f32) -> Vec<String> {
        let mut out = Vec::new();
        for paragraph in text.split('\n') {
            let mut current = String::new();
            for word in paragraph.split(' ') {
                let candidate = if current.is_empty() {
                    word.to_owned()
                } else {
                    format!("{current} {word}")
                };
                if !current.is_empty() && self.measure(family, pixels, &candidate) > width {
                    out.push(std::mem::take(&mut current));
                    current = word.to_owned();
                } else {
                    current = candidate;
                }
            }
            out.push(current);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_fonts::{FxyVersion, Glyph, Page};

    struct TestFonts;

    fn glyph(code: char, w: u16, off_x: i32, off_y: i32, advance: u32) -> Glyph {
        Glyph {
            code: code as u16,
            page: 1,
            x: 0,
            y: 0,
            width: w,
            height: if w > 0 { 15 } else { 0 },
            offset_x: off_x,
            offset_y: off_y,
            advance,
        }
    }

    impl FontLoader for TestFonts {
        fn load_font(&mut self, path: &str) -> Option<Font> {
            let height = if path.ends_with("16") { 28 } else { 14 };
            Some(Font {
                version: FxyVersion::V102,
                pages: vec![Page {
                    index: 1,
                    height,
                    ascent: height * 22 / 28,
                }],
                glyphs: vec![
                    glyph('A', 12, 0, 7, 12),
                    glyph(' ', 0, -10, -10, 5),
                    glyph('?', 8, 0, 7, 9),
                ],
                kerning: Vec::new(),
            })
        }
    }

    fn fonts() -> Fonts {
        let config = a3_config::parse_text(
            r#"class CfgFontFamilies { class Test { fonts[] = {"f\small8", {"f\big16", "f\cjk16"}}; spaceWidth = 2; }; };"#,
        )
        .unwrap();
        Fonts::from_config(&ConfigTree::from_config(&config), Box::new(TestFonts))
    }

    #[test]
    fn picks_the_closest_line_height_and_scales() {
        let mut f = fonts();
        let (selected, scale) = f.select("test", 30.0).unwrap();
        assert_eq!(selected[0].path, r"f\big16");
        assert_eq!(selected.len(), 2);
        assert!((scale - 30.0 / 28.0).abs() < 1e-6);
        let (selected, _) = f.select("TEST", 12.0).unwrap();
        assert_eq!(selected[0].path, r"f\small8");
    }

    #[test]
    fn layout_places_glyphs_by_offsets_and_advances() {
        let mut f = fonts();
        let line = f.layout_line("Test", 28.0, "A A");
        assert_eq!(line.glyphs.len(), 2);
        assert_eq!(line.glyphs[0].dst, [0.0, 7.0, 12.0, 15.0]);
        // Space advance 5 doubled by spaceWidth.
        assert_eq!(line.glyphs[1].dst[0], 22.0);
        assert_eq!(line.width, 34.0);
        assert_eq!(line.glyphs[0].texture, r"f\big16-01.paa");
    }

    #[test]
    fn unknown_characters_draw_a_question_mark() {
        let mut f = fonts();
        let line = f.layout_line("Test", 28.0, "Ž");
        assert_eq!(line.glyphs.len(), 1);
        assert_eq!(line.width, 9.0);
    }

    #[test]
    fn wraps_at_spaces() {
        let mut f = fonts();
        let lines = f.wrap("Test", 28.0, "A A A", 30.0);
        assert_eq!(lines, ["A", "A", "A"]);
        assert_eq!(f.wrap("Test", 28.0, "A A", 100.0), ["A A"]);
    }
}
