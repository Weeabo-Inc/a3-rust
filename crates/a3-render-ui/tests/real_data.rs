//! Draws real game UI textures and real FXY text offscreen, and decodes the UI texture corpus.
//! Skipped when `A3_ROOT` is unset.

mod common;

use std::path::Path;
use std::sync::OnceLock;

use a3_config::{ConfigTree, parse_text};
use a3_fonts::{Font, page_texture_path};
use a3_render::wgpu::TextureFormat;
use a3_render::{Camera, Gpu, Renderer};
use a3_render_ui::{UiFeature, decode_ui_texture};
use a3_ui::draw::{self, Align, TextStyle};
use a3_ui::text::FontLoader;
use a3_ui::{DrawList, Quad};
use a3_vfs::{Vfs, optional_mod_dirs};
use common::pixel;

const W: u32 = 256;
const H: u32 = 96;
const DT: f32 = 1.0 / 60.0;

/// The game VFS, mounted once for all tests in this binary (mounting reads 500 PBO indexes).
fn game_vfs() -> Option<Vfs> {
    static VFS: OnceLock<Option<Vfs>> = OnceLock::new();
    VFS.get_or_init(|| {
        let Some(root) = std::env::var_os("A3_ROOT") else {
            eprintln!("skipping: A3_ROOT not set");
            return None;
        };
        let root = Path::new(&root);
        let vfs = Vfs::new();
        let report = vfs.mount_game(root, &optional_mod_dirs(root));
        eprintln!("mounted {} PBOs, {} files", report.pbos, report.files);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        Some(vfs)
    })
    .clone()
}

fn setup(gpu: &Gpu) -> (Renderer, UiFeature) {
    let mut renderer = Renderer::new(gpu, TextureFormat::Rgba8UnormSrgb);
    let ui = UiFeature::new(gpu, &renderer);
    renderer.add_feature(Box::new(ui.clone()));
    (renderer, ui)
}

fn render(gpu: &Gpu, renderer: &mut Renderer, ui: &UiFeature, list: DrawList) -> Vec<u8> {
    ui.lock().set_draw_list(list);
    renderer
        .render_to_image(gpu, W, H, &Camera::default(), &Default::default(), DT)
        .expect("render")
}

/// Every `.paa` under `a3\ui_f` (the UI textures the feature is built for).
fn ui_textures(vfs: &Vfs) -> Vec<String> {
    let mut paths: Vec<String> = vfs
        .glob("a3\\ui_f\\**\\*.paa")
        .into_iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    paths.sort();
    paths
}

/// Fonts straight from the VFS: `fonts[]` lists FXY paths without the extension.
struct VfsFonts {
    vfs: Vfs,
}

impl FontLoader for VfsFonts {
    fn load_font(&mut self, path: &str) -> Option<Font> {
        let data = self.vfs.open(path).ok().or_else(|| {
            let with_ext = format!("{path}.fxy");
            self.vfs.open(&with_ext).ok()
        })?;
        Font::read(&data).ok()
    }
}

/// The first real FXY whose first page texture exists, and that page's bytes.
fn a_real_font(vfs: &Vfs) -> Option<(String, Font, String, Vec<u8>)> {
    let mut paths: Vec<String> = vfs
        .glob("**/*.fxy")
        .into_iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    // Prefer the font the UI defaults to, so the path is stable across installs.
    paths.sort_by_key(|p| !p.to_ascii_lowercase().contains("robotocondensed"));
    for path in paths {
        let Ok(bytes) = vfs.open(&path) else { continue };
        let Ok(font) = Font::read(&bytes) else {
            continue;
        };
        let Some(page) = font.pages.first() else {
            continue;
        };
        let texture = page_texture_path(&path, page.index);
        if let Ok(page_bytes) = vfs.open(&texture) {
            return Some((path, font, texture, page_bytes.to_vec()));
        }
    }
    None
}

#[test]
fn real_ui_textures_decode_with_and_without_block_compression() {
    let Some(vfs) = game_vfs() else { return };
    let paths = ui_textures(&vfs);
    assert!(paths.len() > 500, "{} UI textures", paths.len());

    let (mut bc, mut rgba) = (0usize, 0usize);
    let mut failures = Vec::new();
    for path in &paths {
        let bytes = vfs.open(path).expect("globbed path opens");
        match decode_ui_texture(path, Some(&bytes), true) {
            Ok(data) => {
                if matches!(
                    data.format,
                    a3_render::TextureFormat::Bc1
                        | a3_render::TextureFormat::Bc2
                        | a3_render::TextureFormat::Bc3
                ) {
                    bc += 1;
                }
            }
            Err(e) => failures.push(format!("bc {path}: {e}")),
        }
        match decode_ui_texture(path, Some(&bytes), false) {
            Ok(data) => {
                assert_eq!(
                    data.format,
                    a3_render::TextureFormat::Rgba8,
                    "{path}: no BC support must decode to RGBA8"
                );
                rgba += 1;
            }
            Err(e) => failures.push(format!("rgba {path}: {e}")),
        }
    }
    eprintln!(
        "{} UI textures: {bc} kept block-compressed, {rgba} decoded to RGBA8, {} failures",
        paths.len(),
        failures.len()
    );
    for line in failures.iter().take(10) {
        eprintln!("FAIL {line}");
    }
    assert!(failures.is_empty());
    assert!(bc * 2 > paths.len(), "most UI textures are DXT: {bc}");
}

#[test]
fn real_font_pages_carry_glyph_coverage_in_the_alpha_channel() {
    let Some(vfs) = game_vfs() else { return };
    let Some((path, font, texture, bytes)) = a_real_font(&vfs) else {
        panic!("no FXY with a page texture");
    };
    eprintln!("font {path}: {} glyphs, page {texture}", font.glyphs.len());

    // The shader tints the page RGB with the text colour and uses the page alpha as coverage,
    // so a real page must be white with the glyph in alpha.
    let header = a3_paa::PaaHeader::read(&bytes).unwrap();
    let fmt = header.meta.format;
    let tex = a3_paa::Texture::read(&bytes).unwrap();
    let rgba = a3_paa::decode_rgba8(tex.format, &tex.mips[0]).unwrap();
    let (w, h) = (tex.mips[0].width as usize, tex.mips[0].height as usize);
    let mut dark_ink = 0usize;
    let mut ink = 0usize;
    let mut rgb_min = 255u8;
    for px in rgba.chunks_exact(4) {
        if px[3] > 200 {
            ink += 1;
            rgb_min = rgb_min.min(px[0]).min(px[1]).min(px[2]);
            if px[0] < 200 || px[1] < 200 || px[2] < 200 {
                dark_ink += 1;
            }
        }
    }
    eprintln!(
        "{w}x{h} {fmt:?} ({} bytes): {ink} texels with alpha > 200, {dark_ink} of them not \
         near-white, min channel {rgb_min}",
        tex.mips[0].data.len()
    );
    assert!(ink > 0, "the page has ink");
    assert!(
        dark_ink * 100 < ink,
        "{dark_ink} of {ink} ink texels are dark: the page is not white-with-alpha"
    );
}

#[test]
fn real_font_text_renders_in_the_text_colour() {
    let Some(vfs) = game_vfs() else { return };
    let Some(gpu) = common::gpu() else { return };
    let Some((path, font, texture, _)) = a_real_font(&vfs) else {
        panic!("no FXY with a page texture");
    };
    eprintln!(
        "drawing {path} ({} glyphs) from {texture}",
        font.glyphs.len()
    );

    // Real configs list the font without its extension; glyph page paths derive from it.
    let listed = path.strip_suffix(".fxy").unwrap_or(&path);
    let config = parse_text(&format!(
        "class CfgFontFamilies {{ class Real {{ fonts[] = {{\"{listed}\"}}; }}; }};"
    ))
    .unwrap();
    let mut fonts = a3_ui::Fonts::from_config(
        &ConfigTree::from_config(&config),
        Box::new(VfsFonts { vfs: vfs.clone() }),
    );

    let mut list = DrawList::default();
    // Scaled up: the font ships one size, the layout scales it to `pixels`.
    let pixels = 48.0;
    draw::draw_text(
        &mut list,
        &mut fonts,
        "Arma",
        [8.0, 8.0, 240.0, 80.0],
        &TextStyle {
            font: "Real",
            pixels,
            color: [1.0, 0.0, 0.0, 1.0],
            align: Align::Left,
            vcenter: false,
            shadow: 0,
            shadow_color: [0.0, 0.0, 0.0, 1.0],
            multiline: false,
            line_spacing: 1.0,
        },
        None,
    );
    eprintln!(
        "{path} at {pixels}px: {} quads, textures {:?}",
        list.quads.len(),
        list.textures()
    );
    assert!(!list.quads.is_empty(), "the text laid out");
    assert!(
        list.quads.iter().all(|q: &Quad| q.texture.is_some()),
        "every glyph quad is textured"
    );

    let (mut renderer, ui) = setup(&gpu);
    ui.lock().set_assets(Box::new(vfs));
    ui.lock().preload(&gpu, &list);
    let image = render(&gpu, &mut renderer, &ui, list);
    common::dump_png(&image, W, H);

    let stats = ui.lock().stats();
    eprintln!("stats {stats:?}");
    assert_eq!(stats.missing_textures, 0, "the page texture resolved");
    assert_eq!(stats.batches, 1, "one page, one batch");

    // Text colour over the page's white glyph cores: red, no green or blue, opaque.
    let mut core = 0usize;
    let mut red_dominant = 0usize;
    let mut max_red = 0u8;
    for y in 0..H {
        for x in 0..W {
            let [r, g, b, a] = pixel(&image, W, x, y);
            max_red = max_red.max(r);
            if r > 200 && g < 60 && b < 60 && a == 255 {
                core += 1;
            }
            if r > 100 && r > g.saturating_mul(2) && r > b.saturating_mul(2) {
                red_dominant += 1;
            }
        }
    }
    eprintln!(
        "{core} pixels are the text colour, {red_dominant} are red-dominant \
         (max red {max_red})"
    );
    assert!(core > 50, "the glyph cores painted in the text colour");
    assert!(
        red_dominant > 300,
        "the glyphs and their antialiased edges are red"
    );
}

#[test]
fn a_real_ui_icon_draws_over_the_background() {
    let Some(vfs) = game_vfs() else { return };
    let Some(gpu) = common::gpu() else { return };
    // A shipped main-menu icon; any real UI texture would do.
    let path = "a3\\ui_f\\data\\gui\\rsc\\rscdisplaymain\\spotlight_2_ca.paa";
    if vfs.open(path).is_err() {
        eprintln!("skipping: {path} not in this install");
        return;
    }

    let (mut renderer, ui) = setup(&gpu);
    ui.lock().set_assets(Box::new(vfs));

    // The background alone, to compare the icon against.
    let background = render(&gpu, &mut renderer, &ui, DrawList::default());
    let mut list = DrawList::default();
    let texture = Some(list.texture(path));
    list.quads.push(Quad {
        rect: [0.0, 0.0, W as f32, H as f32],
        uv: a3_ui::draw::FULL,
        color: [1.0, 1.0, 1.0, 1.0],
        texture,
        clip: None,
        angle: 0.0,
    });
    let image = render(&gpu, &mut renderer, &ui, list);

    let stats = ui.lock().stats();
    eprintln!("icon {path}: stats {stats:?}");
    assert_eq!(stats.missing_textures, 0, "the icon texture resolved");
    let changed = (0..(W * H) as usize)
        .filter(|&i| image[i * 4..i * 4 + 4] != background[i * 4..i * 4 + 4])
        .count();
    eprintln!("{changed} of {} pixels changed", W * H);
    assert!(
        changed > 100,
        "the icon covers part of the viewport ({changed} pixels)"
    );
}
