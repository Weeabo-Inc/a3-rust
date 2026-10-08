//! Procedural texture strings.

use a3_paa::{ColorFormat, PixelFormat, Procedural, ProceduralFunction, decode_rgba8};

#[test]
fn parses_a_color_texture() {
    let p = Procedural::parse("#(argb,8,8,3)color(1,0,0.5,1)").unwrap();
    assert_eq!(p.format, ColorFormat::Argb);
    assert_eq!((p.width, p.height, p.mip_count), (8, 8, 3));
    assert_eq!(
        p.function,
        ProceduralFunction::Color {
            rgba: [1.0, 0.0, 0.5, 1.0],
            tag: None
        }
    );
}

#[test]
fn parses_the_optional_texture_type_tag_and_any_case() {
    let p = Procedural::parse("#(ARGB,8,8,3)Color(0.5,0.5,1,1,\"NOHQ\")").unwrap();
    assert_eq!(
        p.function,
        ProceduralFunction::Color {
            rgba: [0.5, 0.5, 1.0, 1.0],
            tag: Some("NOHQ".into())
        }
    );
    let p = Procedural::parse("#(argb,8,8,3)color(0,0,0,1,CO)").unwrap();
    assert!(matches!(p.function, ProceduralFunction::Color { tag: Some(t), .. } if t == "CO"));
}

#[test]
fn keeps_other_functions_with_their_arguments() {
    let p = Procedural::parse("#(ai,64,64,1)fresnel(1.3,7)").unwrap();
    assert_eq!(p.format, ColorFormat::Ai);
    assert_eq!(
        p.function,
        ProceduralFunction::Other {
            name: "fresnel".into(),
            args: vec!["1.3".into(), "7".into()]
        }
    );
    assert_eq!(p.to_string(), "#(ai,64,64,1)fresnel(1.3,7)");
}

#[test]
fn rejects_strings_that_are_not_procedural_textures() {
    for text in [
        r"a3\data_f\default.paa",
        "#(argb,8,8)color(1,0,0,1)",
        "#(argb,8,8,3)color(1,0,0,1",
        "#(xyz,8,8,3)color(1,0,0,1)",
        "#(argb,8,8,3)color(1,0,red,1)",
    ] {
        assert!(Procedural::parse(text).is_err(), "{text}");
    }
}

#[test]
fn a_color_texture_generates_a_solid_mip_chain() {
    let texture = Procedural::parse("#(argb,8,8,3)color(1,0,0.5,1)")
        .unwrap()
        .generate()
        .unwrap();
    assert_eq!(texture.format, PixelFormat::Argb8888);
    let sizes: Vec<_> = texture.mips.iter().map(|m| (m.width, m.height)).collect();
    assert_eq!(sizes, [(8, 8), (4, 4), (2, 2)]);
    let pixels = decode_rgba8(texture.format, &texture.mips[2]).unwrap();
    assert_eq!(pixels, [255, 0, 128, 255].repeat(4));
}

#[test]
fn rgb_textures_are_opaque_and_ai_textures_are_grey() {
    let rgb = Procedural::parse("#(rgb,1,1,1)color(0,1,0,0)")
        .unwrap()
        .generate()
        .unwrap();
    assert_eq!(
        decode_rgba8(rgb.format, &rgb.mips[0]).unwrap(),
        [0, 255, 0, 255]
    );

    let ai = Procedural::parse("#(ai,2,2,1)color(0.2,0.2,0.2,0.4)")
        .unwrap()
        .generate()
        .unwrap();
    assert_eq!(ai.format, PixelFormat::Ai88);
    assert_eq!(
        &decode_rgba8(ai.format, &ai.mips[0]).unwrap()[..4],
        [51, 51, 51, 102]
    );
}

#[test]
fn other_functions_cannot_be_generated_yet() {
    let p = Procedural::parse("#(ai,64,64,1)fresnel(1.3,7)").unwrap();
    assert!(p.generate().is_err());
}
