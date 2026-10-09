//! Draws synthetic UI draw lists through the feature offscreen and checks pixels. Skips when no
//! GPU adapter (not even a software one) is available.

mod common;

use a3_config::{ConfigTree, parse_text};
use a3_fonts::{Font, FxyVersion, Glyph, Page};
use a3_render::wgpu::TextureFormat;
use a3_render::{Camera, Gpu, Renderer};
use a3_render_ui::{MemoryAssets, UiFeature};
use a3_ui::draw::{self, Align, FULL, TextStyle};
use a3_ui::text::FontLoader;
use a3_ui::{DrawList, Quad, Rgba, Uv};
use common::pixel;

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;

/// The feature on a fresh renderer, ready to draw.
fn setup(gpu: &Gpu) -> (Renderer, UiFeature) {
    let mut renderer = Renderer::new(gpu, TextureFormat::Rgba8UnormSrgb);
    let ui = UiFeature::new(gpu, &renderer);
    renderer.add_feature(Box::new(ui.clone()));
    (renderer, ui)
}

/// One frame with `list` as the UI draw list.
fn render(gpu: &Gpu, renderer: &mut Renderer, ui: &UiFeature, list: DrawList) -> Vec<u8> {
    ui.lock().set_draw_list(list);
    renderer
        .render_to_image(gpu, SIZE, SIZE, &Camera::default(), &Default::default(), DT)
        .expect("render")
}

fn solid(rect: [f32; 4], color: Rgba) -> Quad {
    Quad {
        rect,
        uv: FULL,
        color,
        texture: None,
        clip: None,
        angle: 0.0,
    }
}

/// Pushes a white quad sampling `path` (registered in `list`).
fn push_textured(list: &mut DrawList, rect: [f32; 4], path: &str, uv: Uv) {
    let texture = Some(list.texture(path));
    list.quads.push(Quad {
        rect,
        uv,
        color: [1.0, 1.0, 1.0, 1.0],
        texture,
        clip: None,
        angle: 0.0,
    });
}

/// A 16x16 page: red in the top-left 4x4 texels, blue in the next 4x4, transparent elsewhere.
fn two_block_page() -> Vec<u8> {
    let mut image = vec![0u8; 16 * 16 * 4];
    for y in 0..4 {
        for x in 0..4 {
            let i = (y * 16 + x) * 4;
            image[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            let j = (y * 16 + x + 4) * 4;
            image[j..j + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    a3_paa::encode_rgba8(
        16,
        16,
        &image,
        &a3_paa::EncodeOptions::new(a3_paa::PixelFormat::Argb8888),
    )
    .unwrap()
    .to_bytes()
    .unwrap()
}

/// A 16x16 glyph page: white, full alpha, only in the top-left 4x4 texels.
fn glyph_page() -> Vec<u8> {
    let mut image = vec![0u8; 16 * 16 * 4];
    for y in 0..4 {
        for x in 0..4 {
            let i = (y * 16 + x) * 4;
            image[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    a3_paa::encode_rgba8(
        16,
        16,
        &image,
        &a3_paa::EncodeOptions::new(a3_paa::PixelFormat::Argb8888),
    )
    .unwrap()
    .to_bytes()
    .unwrap()
}

// A font family with one 16 px font; 'A' is a 4x4 glyph at the page origin.
struct TestFonts;

impl FontLoader for TestFonts {
    fn load_font(&mut self, _path: &str) -> Option<Font> {
        Some(Font {
            version: FxyVersion::V102,
            pages: vec![Page {
                index: 1,
                height: 16,
                ascent: 12,
            }],
            glyphs: vec![Glyph {
                code: 'A' as u16,
                page: 1,
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                offset_x: 0,
                offset_y: 0,
                advance: 4,
            }],
            kerning: Vec::new(),
        })
    }
}

fn test_fonts() -> a3_ui::Fonts {
    let config =
        parse_text(r#"class CfgFontFamilies { class Test { fonts[] = {"f\big16"}; }; };"#).unwrap();
    a3_ui::Fonts::from_config(&ConfigTree::from_config(&config), Box::new(TestFonts))
}

#[test]
fn an_untextured_quad_is_drawn_with_its_colour() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [1.0, 0.0, 0.0, 1.0],
    ));
    let image = render(&gpu, &mut renderer, &ui, list);
    // Opaque and untextured: exactly the authored sRGB colour, everywhere.
    assert_eq!(pixel(&image, SIZE, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, SIZE, 32, 32), [255, 0, 0, 255]);
    let stats = ui.lock().stats();
    assert_eq!((stats.quads, stats.vertices, stats.batches), (1, 6, 1));
    assert_eq!(stats.missing_textures, 0);
}

#[test]
fn later_quads_paint_over_earlier_ones() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [0.0, 0.0, 1.0, 1.0],
    ));
    list.quads
        .push(solid([16.0, 16.0, 16.0, 16.0], [1.0, 0.0, 0.0, 1.0]));
    let image = render(&gpu, &mut renderer, &ui, list);
    assert_eq!(pixel(&image, SIZE, 24, 24), [255, 0, 0, 255], "centre");
    assert_eq!(pixel(&image, SIZE, 4, 4), [0, 0, 255, 255], "corner");
    assert_eq!(ui.lock().stats().batches, 1, "same texture and clip join");
}

#[test]
fn clip_becomes_a_scissor_and_leaves_the_rest_alone() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [1.0, 1.0, 1.0, 1.0],
    ));
    let mut clipped = solid([0.0, 0.0, SIZE as f32, SIZE as f32], [0.0, 1.0, 0.0, 1.0]);
    clipped.clip = Some([0.0, 0.0, 16.0, SIZE as f32]);
    list.quads.push(clipped);
    // A third quad *without* the clip: the scissor must not leak into it.
    list.quads
        .push(solid([48.0, 0.0, 16.0, 16.0], [1.0, 0.0, 0.0, 1.0]));
    let image = render(&gpu, &mut renderer, &ui, list);
    assert_eq!(
        pixel(&image, SIZE, 8, 32),
        [0, 255, 0, 255],
        "inside the clip"
    );
    assert_eq!(pixel(&image, SIZE, 32, 32), [255, 255, 255, 255], "outside");
    assert_eq!(
        pixel(&image, SIZE, 56, 8),
        [255, 0, 0, 255],
        "after the clip"
    );
    assert_eq!(ui.lock().stats().batches, 3);
}

#[test]
fn alpha_blends_over_what_is_below() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [0.0, 0.0, 1.0, 1.0],
    ));
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [1.0, 0.0, 0.0, 0.5],
    ));
    let image = render(&gpu, &mut renderer, &ui, list);
    let [r, g, b, _] = pixel(&image, SIZE, 32, 32);
    assert!(
        r > 100 && b > 100 && g < 60,
        "half red over blue: {r} {g} {b}"
    );
}

#[test]
fn a_procedural_texture_is_generated_and_bound() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    push_textured(
        &mut list,
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        "#(argb,8,8,3)color(0,1,0,1,CO)",
        FULL,
    );
    let image = render(&gpu, &mut renderer, &ui, list);
    assert_eq!(pixel(&image, SIZE, 32, 32), [0, 255, 0, 255]);
    assert_eq!(ui.lock().stats().textures, 2, "white plus the procedural");
}

#[test]
fn texel_uvs_sample_the_right_block_of_a_paa_file() {
    let Some(gpu) = common::gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("page.paa"), two_block_page()).unwrap();
    let (mut renderer, ui) = setup(&gpu);
    ui.lock()
        .set_assets(Box::new(a3_render_ui::vfs_with_dir(dir.path())));

    let mut list = DrawList::default();
    // The whole viewport from the red 4x4 block...
    push_textured(
        &mut list,
        [0.0, 0.0, 32.0, SIZE as f32],
        "page.paa",
        Uv::Texels([0.0, 0.0, 4.0, 4.0]),
    );
    // ...and the blue one next to it.
    push_textured(
        &mut list,
        [32.0, 0.0, 32.0, SIZE as f32],
        "page.paa",
        Uv::Texels([4.0, 0.0, 4.0, 4.0]),
    );
    let image = render(&gpu, &mut renderer, &ui, list);
    assert_eq!(pixel(&image, SIZE, 16, 32), [255, 0, 0, 255], "left half");
    assert_eq!(pixel(&image, SIZE, 48, 32), [0, 0, 255, 255], "right half");
    assert_eq!(
        ui.lock().stats().uploads,
        1,
        "the page, the white is built in"
    );
}

#[test]
fn glyph_text_is_drawn_from_the_font_page() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut assets = MemoryAssets::new();
    assets.insert("f\\big16-01.paa", glyph_page());
    ui.lock().set_assets(Box::new(assets));

    let mut fonts = test_fonts();
    let mut list = DrawList::default();
    draw::draw_text(
        &mut list,
        &mut fonts,
        "A",
        [0.0, 0.0, 8.0, 8.0],
        &TextStyle {
            font: "Test",
            pixels: 16.0,
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
    assert_eq!(list.quads.len(), 1, "one glyph quad");
    assert_eq!(list.quads[0].texture, Some(a3_ui::TextureKey(0)));
    let image = render(&gpu, &mut renderer, &ui, list);
    // The glyph's white page texels take the text colour.
    assert_eq!(pixel(&image, SIZE, 2, 2), [255, 0, 0, 255], "the glyph");
    let [r, g, b, _] = pixel(&image, SIZE, 24, 24);
    assert!(!(r > 200 && g < 60 && b < 60), "no text here: {r} {g} {b}");
    let stats = ui.lock().stats();
    assert_eq!(stats.missing_textures, 0);
    assert_eq!(stats.textures, 2, "white plus the glyph page");
}

#[test]
fn a_missing_texture_drops_its_quad_and_is_counted() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut list = DrawList::default();
    push_textured(
        &mut list,
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        "nope.paa",
        FULL,
    );
    // Drawn under it, to prove the quad above was dropped, not drawn.
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [1.0, 0.0, 0.0, 1.0],
    ));
    let image = render(&gpu, &mut renderer, &ui, list);
    assert_eq!(pixel(&image, SIZE, 32, 32), [255, 0, 0, 255]);
    let stats = ui.lock().stats();
    assert_eq!(stats.missing_textures, 1);
    assert_eq!(stats.vertices, 6, "only the quad below");
}

#[test]
fn a_whole_frame_draws_panels_texture_and_text() {
    let Some(gpu) = common::gpu() else { return };
    let (mut renderer, ui) = setup(&gpu);
    let mut assets = MemoryAssets::new();
    assets.insert("f\\big16-01.paa", glyph_page());
    ui.lock().set_assets(Box::new(assets));
    let mut fonts = test_fonts();

    let mut list = DrawList::default();
    // A dark backdrop, a translucent panel over it, a procedural swatch and white text.
    list.quads.push(solid(
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        [0.05, 0.06, 0.08, 1.0],
    ));
    list.quads
        .push(solid([6.0, 6.0, 52.0, 40.0], [0.2, 0.4, 0.9, 0.5]));
    push_textured(
        &mut list,
        [44.0, 44.0, 16.0, 16.0],
        "#(argb,8,8,3)color(0,1,0,1,CO)",
        FULL,
    );
    draw::draw_text(
        &mut list,
        &mut fonts,
        "A",
        [8.0, 12.0, 24.0, 20.0],
        &TextStyle {
            font: "Test",
            pixels: 16.0,
            color: [1.0, 1.0, 1.0, 1.0],
            align: Align::Left,
            vcenter: false,
            shadow: 0,
            shadow_color: [0.0, 0.0, 0.0, 1.0],
            multiline: false,
            line_spacing: 1.0,
        },
        None,
    );

    let image = render(&gpu, &mut renderer, &ui, list);
    common::dump_png(&image, SIZE, SIZE);

    assert_eq!(pixel(&image, SIZE, 2, 2), [13, 15, 20, 255], "backdrop");
    let [r, g, b, _] = pixel(&image, SIZE, 30, 30);
    assert!(
        b > r && r > 0,
        "the panel lightens the backdrop: {r} {g} {b}"
    );
    assert_eq!(pixel(&image, SIZE, 52, 52), [0, 255, 0, 255], "the swatch");
    assert_eq!(
        pixel(&image, SIZE, 10, 14),
        [255, 255, 255, 255],
        "the glyph"
    );
    let stats = ui.lock().stats();
    assert_eq!(stats.missing_textures, 0);
    // Backdrop and panel share the white slot and join; the swatch and the text do not.
    assert_eq!((stats.quads, stats.batches), (4, 3));
}

#[test]
fn textures_are_decoded_once_and_kept_across_frames() {
    let Some(gpu) = common::gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("page.paa"), two_block_page()).unwrap();
    let (mut renderer, ui) = setup(&gpu);
    ui.lock()
        .set_assets(Box::new(a3_render_ui::vfs_with_dir(dir.path())));
    let mut list = DrawList::default();
    push_textured(
        &mut list,
        [0.0, 0.0, SIZE as f32, SIZE as f32],
        "page.paa",
        FULL,
    );
    render(&gpu, &mut renderer, &ui, list.clone());
    assert_eq!(ui.lock().stats().uploads, 1, "the page on frame one");
    render(&gpu, &mut renderer, &ui, list);
    let stats = ui.lock().stats();
    assert_eq!(stats.uploads, 0, "frame two uploads nothing");
    assert_eq!(stats.textures, 2);
    assert_eq!(ui.lock().texture_count(), 2);
}
