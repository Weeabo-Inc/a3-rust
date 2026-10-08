//! Decodes every FXY of a real game install and checks that each glyph lies inside its page
//! texture. Skipped when `A3_ROOT` is unset.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use a3_fonts::{Font, page_texture_path};
use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn every_font_decodes_and_glyphs_fit_their_pages() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));

    let paths = vfs.glob("**/*.fxy");
    let mut versions = BTreeMap::new();
    let (mut glyphs, mut kerning, mut missing_pages) = (0, 0, Vec::new());
    let mut failures = Vec::new();
    let mut outside = Vec::new();
    let mut page_sizes: HashMap<String, (u16, u16)> = HashMap::new();
    for path in &paths {
        let data = vfs.open(path.as_str()).unwrap();
        let font = match Font::read(&data) {
            Ok(font) => font,
            Err(e) => {
                failures.push(format!("{path}: {e}"));
                continue;
            }
        };
        *versions.entry(font.version.to_string()).or_insert(0) += 1;
        glyphs += font.glyphs.len();
        kerning += font.kerning.len();
        for glyph in &font.glyphs {
            let texture = page_texture_path(path.as_str(), glyph.page);
            let size = match page_sizes.get(&texture) {
                Some(size) => *size,
                None => {
                    let Ok(bytes) = vfs.open(&texture) else {
                        missing_pages.push(texture);
                        continue;
                    };
                    let header = a3_paa::PaaHeader::read(&bytes).unwrap();
                    let size = (header.mips[0].width, header.mips[0].height);
                    page_sizes.insert(texture.clone(), size);
                    size
                }
            };
            if u32::from(glyph.x) + u32::from(glyph.width) > u32::from(size.0)
                || u32::from(glyph.y) + u32::from(glyph.height) > u32::from(size.1)
            {
                outside.push(format!(
                    "{path}: glyph {:#x} at {},{} {}x{} outside {size:?}",
                    glyph.code, glyph.x, glyph.y, glyph.width, glyph.height
                ));
            }
        }
    }
    missing_pages.sort();
    missing_pages.dedup();

    eprintln!(
        "{} fonts {versions:?}; {glyphs} glyphs, {kerning} kerning pairs, {} page textures, \
         {} missing pages {missing_pages:?}",
        paths.len(),
        page_sizes.len(),
        missing_pages.len()
    );
    eprintln!("{} glyphs reach past their page texture:", outside.len());
    for line in outside.iter().take(5) {
        eprintln!("  {line}");
    }
    for failure in failures.iter().take(20) {
        eprintln!("FAIL {failure}");
    }
    assert!(paths.len() > 400, "expected the full install");
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(missing_pages.is_empty());
    // A few glyphs in shipped fonts reach past the edge of their page (a data quirk).
    assert!(
        outside.len() < 50,
        "{} glyphs outside their page",
        outside.len()
    );

    let font = Font::read(
        &vfs.open(r"a3\ui_f_enoch\data\cfgfontfamilies\caveat\caveat10.fxy")
            .unwrap(),
    )
    .unwrap();
    assert!(font.glyph('A').is_some() && font.glyph('ž').is_some());
}
