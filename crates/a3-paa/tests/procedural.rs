//! Procedural texture strings: parsing as the engine accepts them, and generated pixels.
//!
//! Expected pixel values come from the formulas in `docs/re/paa.md`, evaluated by hand or (for
//! Perlin noise) with Ken Perlin's published reference implementation.

use a3_paa::{
    ColorFormat, PixelFormat, Procedural, ProceduralFunction, Texture, TextureType, decode_rgba8,
};

fn generate(text: &str) -> Texture {
    Procedural::parse(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .generate()
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// The first mipmap as RGBA8.
fn pixels(texture: &Texture) -> Vec<[u8; 4]> {
    decode_rgba8(texture.format, &texture.mips[0])
        .unwrap()
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

/// Intensity and alpha of every pixel of an AI88 texture's first mipmap.
fn intensity_alpha(texture: &Texture) -> Vec<(u8, u8)> {
    assert_eq!(texture.format, PixelFormat::Ai88);
    texture.mips[0]
        .data
        .chunks_exact(2)
        .map(|p| (p[0], p[1]))
        .collect()
}

fn sizes(texture: &Texture) -> Vec<(u16, u16)> {
    texture.mips.iter().map(|m| (m.width, m.height)).collect()
}

#[test]
fn parses_the_header_and_a_color_function() {
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
    assert_eq!(p.to_string(), "#(argb,8,8,3)color(1,0,0.5,1)");
}

#[test]
fn names_and_formats_are_case_insensitive() {
    let p = Procedural::parse("#(ARGB,8,8,3)Color(0.5,0.5,1,1,NOHQ)").unwrap();
    assert_eq!(
        p.function,
        ProceduralFunction::Color {
            rgba: [0.5, 0.5, 1.0, 1.0],
            tag: Some("NOHQ".into())
        }
    );
    let p = Procedural::parse("#(ai,512,512,9)perlinnoise(256,256,0.8,1)").unwrap();
    assert_eq!(
        p.function,
        ProceduralFunction::PerlinNoise {
            x_scale: 256.0,
            y_scale: 256.0,
            min: 0.8,
            max: 1.0
        }
    );
    assert!(Procedural::parse("#(ai,64,64,1)fresnelGlass(2)").is_ok());
}

#[test]
fn missing_fresnel_arguments_take_the_engine_defaults() {
    let p = Procedural::parse("#(ai,64,64,1)fresnel()").unwrap();
    assert_eq!(
        p.function,
        ProceduralFunction::Fresnel {
            n: 0.96977,
            k: 0.0118
        }
    );
    let p = Procedural::parse("#(ai,64,64,1)fresnelglass()").unwrap();
    assert_eq!(p.function, ProceduralFunction::FresnelGlass { n: 1.7 });
}

#[test]
fn rejects_what_the_engine_rejects() {
    for text in [
        r"a3\data_f\default.paa",
        "#(argb,8,8)color(1,0,0,1)",
        "#(xyz,8,8,3)color(1,0,0,1)",
        "#(argb,6,8,1)color(1,0,0,1)",  // not a power of two
        "#(argb,4,4,4)color(1,0,0,1)",  // more mipmaps than the size allows
        "#(argb,8,8,3)color(.5,0,0,1)", // numbers must start with a digit
        "#(argb,8,8,3)color(1,0,-1,1)",
        "#(argb,8,8,3)color(1,0,red,1)",
        "#(argb,8,8,3)color(1,0,0)",
        "#(ai,64,64,1)fresnel(1.3)", // k is required once n is given
        "#(ai,2048,2048,1)perlinNoise(%1,%2,%3,%4)",
        "#(ai,8,8,1)sparkle(1)",
    ] {
        assert!(Procedural::parse(text).is_err(), "{text}");
    }
}

#[test]
fn color_textures_are_a_single_pixel_whatever_the_declared_size() {
    let t = generate("#(argb,8,8,3)color(1,0,0.5,1)");
    assert_eq!(t.format, PixelFormat::Argb8888);
    assert_eq!(sizes(&t), [(1, 1)]);
    assert_eq!(pixels(&t), [[255, 0, 128, 255]]);
}

#[test]
fn rgb_is_the_same_as_argb_and_keeps_alpha() {
    let t = generate("#(rgb,1,1,1)color(0,1,0,0)");
    assert_eq!(pixels(&t), [[0, 255, 0, 0]]);
}

#[test]
fn ai_color_takes_the_luma_of_the_colour() {
    // 0.2*76.245 + 0.4*149.685 + 0.6*29.07 = 92.565; alpha 0.5*255 = 127.5 rounds to even.
    let t = generate("#(ai,1,1,1)color(0.2,0.4,0.6,0.5)");
    assert_eq!(intensity_alpha(&t), [(93, 128)]);
    let t = generate("#(i,1,1,1)color(0.2,0.4,0.6,0.5)");
    assert_eq!(intensity_alpha(&t), [(93, 128)]);
}

#[test]
fn color_texture_type_comes_from_the_tag_or_the_nearest_known_colour() {
    let ty = |text: &str| Procedural::parse(text).unwrap().texture_type();
    assert_eq!(
        ty("#(argb,8,8,3)color(0.5,0.5,1,1,nohq)"),
        TextureType::Normal
    );
    assert_eq!(ty("#(argb,8,8,3)color(0,0,0,0,MC)"), TextureType::Macro);
    assert_eq!(ty("#(argb,8,8,3)color(1,1,1,1,co)"), TextureType::Diffuse);
    assert_eq!(ty("#(argb,8,8,3)color(0.5,0.5,1,1)"), TextureType::Normal);
    assert_eq!(ty("#(argb,8,8,3)color(0.3,0.2,0.1,1)"), TextureType::Detail);
    assert_eq!(ty("#(argb,8,8,3)color(0,1,0,1)"), TextureType::Diffuse);
    assert_eq!(ty("#(ai,64,64,1)fresnel(1,1)"), TextureType::Irradiance);
    assert_eq!(ty("#(ai,8,8,1)perlinNoise(1,1,0,1)"), TextureType::Detail);
}

#[test]
fn fresnel_is_a_one_row_lookup_of_conductor_reflectance_in_alpha() {
    let t = generate("#(ai,4,4,1)fresnel(1.3,7)");
    assert_eq!(sizes(&t), [(4, 1)]);
    let px = intensity_alpha(&t);
    // Grazing incidence (cos = 0) reflects everything.
    assert_eq!(px[0], (0, 255));
    // Normal incidence: ((n-1)^2 + k^2) / ((n+1)^2 + k^2) = 49.09 / 54.29 -> 230.6.
    assert_eq!(px[3], (0, 231));
    // In between, the exact conductor formula (evaluated in f64): 220.8 and 229.0.
    assert_eq!((px[1], px[2]), ((0, 221), (0, 229)));

    let argb = generate("#(argb,4,4,1)fresnel(1.3,7)");
    assert_eq!(pixels(&argb)[3], [0, 0, 0, 231]);
}

#[test]
fn fresnel_glass_is_dielectric_reflectance() {
    let t = generate("#(ai,4,4,1)fresnelGlass(1.5)");
    assert_eq!(sizes(&t), [(4, 1)]);
    // Normal incidence: ratio (1 - 1/1.5)/(1 + 1/1.5) squared twice, S / (S/2 + 1) * 255 -> 19.6.
    assert_eq!(intensity_alpha(&t)[3], (0, 20));
}

#[test]
fn irradiance_ramps_intensity_across_and_raises_alpha_to_a_power_down() {
    let t = generate("#(ai,4,4,1)irradiance(2)");
    let px = intensity_alpha(&t);
    // Column 0 is all zero.
    assert_eq!(px[0], (0, 0));
    assert_eq!(px[3 * 4], (0, 0));
    // x = 1/3 -> 85; y = 2/3 -> (2/3)^2 * 255 = 113.3.
    assert_eq!(px[2 * 4 + 1], (85, 113));
    assert_eq!(px[3 * 4 + 3], (255, 255));
}

#[test]
fn water_irradiance_puts_air_to_water_fresnel_in_alpha() {
    let t = generate("#(ai,4,4,1)waterIrradiance(1)");
    let px = intensity_alpha(&t);
    // Row 3 has intensity 255 (y^1); grazing (x = 0) reflects fully, normal incidence ~2%.
    assert_eq!(px[3 * 4], (255, 255));
    assert_eq!(px[3 * 4 + 3], (255, 5));
    assert_eq!(px[0].0, 0);
}

#[test]
fn perlin_noise_matches_the_reference_improved_noise() {
    let t = generate("#(ai,2,2,1)perlinNoise(1,1,0,1)");
    let px = intensity_alpha(&t);
    assert_eq!(px, [(136, 136), (99, 99), (118, 118), (80, 80)]);
    // Samples on lattice points are 0, mapped to the middle of [min, max].
    let t = generate("#(argb,2,2,1)perlinNoise(4,4,0,1)");
    assert_eq!(pixels(&t), [[128; 4]; 4]);
}

#[test]
fn point_is_a_white_disc_fading_out_from_the_centre() {
    let t = generate("#(ai,4,4,1)point()");
    let px = intensity_alpha(&t);
    assert_eq!(px[0], (255, 0));
    // (1/3, 1/3) -> u = v = -1/3, 1 - sqrt(2)/3 = 0.5286.
    assert_eq!(px[4 + 1], (255, 135));
}

#[test]
fn tree_crown_attenuates_by_density_to_the_power_of_the_coordinate() {
    let t = generate("#(ai,2,2,1)treeCrown(0.25)");
    // Alpha follows the row, intensity the column: 0.25^0 = 1, 0.25^1 = 63.75.
    assert_eq!(
        intensity_alpha(&t),
        [(255, 255), (64, 255), (255, 64), (64, 64)]
    );

    let t = generate("#(ai,2,4,1)treeCrownAmb(0.25)");
    let px = intensity_alpha(&t);
    // Outside the unit circle: 1. (0, 1): r^2 = 1/9, 0.25^(8/9) = 0.2916.
    assert_eq!(px[0], (255, 255));
    assert_eq!(px[2], (74, 74));
}

#[test]
fn dither_builds_an_ordered_dither_matrix() {
    let t = generate("#(ai,2,2,1)dither(0,64)");
    assert_eq!(sizes(&t), [(2, 2)]);
    assert_eq!(intensity_alpha(&t), [(8, 8), (40, 40), (56, 56), (24, 24)]);
    assert!(
        Procedural::parse("#(argb,2,2,1)dither(0,64)")
            .unwrap()
            .generate()
            .is_err()
    );
}

#[test]
fn levels_halve_until_a_side_drops_below_two() {
    let t = generate("#(ai,8,2,3)perlinNoise(1,1,0,1)");
    assert_eq!(sizes(&t), [(8, 2), (4, 1)]);
}

#[test]
fn runtime_sources_parse_but_cannot_be_generated_offline() {
    let p = Procedural::parse("#(argb,512,512,1)r2t(rendertarget0,1.0)").unwrap();
    assert!(matches!(p.function, ProceduralFunction::Runtime { .. }));
    assert!(p.generate().is_err());
}
