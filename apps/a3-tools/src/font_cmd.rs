//! `a3-tools font ...`: inspect FXY bitmap fonts.

use std::fmt::Write as _;

use a3_fonts::Font;
use clap::Subcommand;

use crate::input::InputArgs;

#[derive(Subcommand)]
pub enum FontCommand {
    /// Print the layout version, pages and glyph count of an .fxy file.
    Info(InputArgs),
    /// List every glyph: character, page, rectangle, offsets and advance.
    Glyphs(InputArgs),
}

pub fn run(cmd: FontCommand) -> anyhow::Result<()> {
    match cmd {
        FontCommand::Info(input) => print!("{}", info(&Font::read(&input.read()?)?, &input.input)),
        FontCommand::Glyphs(input) => print!("{}", glyphs(&Font::read(&input.read()?)?)),
    }
    Ok(())
}

fn info(font: &Font, path: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "version  {}", font.version);
    let _ = writeln!(out, "glyphs   {}", font.glyphs.len());
    let _ = writeln!(out, "kerning  {} pairs", font.kerning.len());
    let _ = writeln!(out, "pages    {}", font.pages.len());
    for page in &font.pages {
        let count = font.glyphs.iter().filter(|g| g.page == page.index).count();
        let _ = writeln!(
            out,
            "  {:>3}  {count:>4} glyphs  height {} ascent {}  {}",
            page.index,
            page.height,
            page.ascent,
            a3_fonts::page_texture_path(path, page.index)
        );
    }
    out
}

fn glyphs(font: &Font) -> String {
    let mut out = String::new();
    for g in &font.glyphs {
        let shown = char::from_u32(u32::from(g.code))
            .filter(|c| !c.is_control())
            .unwrap_or('?');
        let _ = writeln!(
            out,
            "U+{:04X} {shown}  page {} at {},{} {}x{}  offset {},{}  advance {}",
            g.code, g.page, g.x, g.y, g.width, g.height, g.offset_x, g.offset_y, g.advance
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        // Unversioned layout: code - 0x20, page, x, y, w, h.
        let bytes: Vec<u8> = [0x21u16, 1, 4, 0, 7, 13]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        Font::read(&bytes).unwrap()
    }

    #[test]
    fn info_names_page_textures() {
        let text = info(&font(), r"fonts\lucida8.fxy");
        assert!(text.contains(r"fonts\lucida8-01.paa"), "{text}");
        assert!(text.contains("glyphs   1"), "{text}");
    }

    #[test]
    fn glyphs_lists_characters() {
        assert_eq!(
            glyphs(&font()),
            "U+0041 A  page 1 at 4,0 7x13  offset 0,0  advance 7\n"
        );
    }
}
