//! Texture suffix interpretation.

use a3_paa::{TextureKind, TextureType};

#[test]
fn the_suffix_before_the_extension_decides_the_kind() {
    let cases = [
        (
            r"a3\characters_f\blufor\data\armor1_co.paa",
            TextureKind::Color,
        ),
        (
            r"A3\UI_F\Data\IGUI\Cfg\Actions\autohover_CA.paa",
            TextureKind::ColorAlpha,
        ),
        (
            r"a3\characters_f\blufor\data\armor1_nohq.paa",
            TextureKind::NormalHq,
        ),
        (r"x\y_no.paa", TextureKind::Normal),
        (r"x\y_nopx.paa", TextureKind::NormalParallax),
        (r"x\y_smdi.paa", TextureKind::SpecularMetalDetail),
        (r"x\y_as.paa", TextureKind::AmbientShadow),
        (r"x\y_mc.paa", TextureKind::Macro),
        (r"x\y_dt.paa", TextureKind::Detail),
        (r"a3\map_altis\data\sky_clear_sky.paa", TextureKind::Sky),
        (r"x\layers\00_00\s_002_016_lco.paa", TextureKind::LayerColor),
        (
            r"x\layers\00_00\m_002_016_lca.paa",
            TextureKind::LayerColorAlpha,
        ),
        (r"x\y_mask.paa", TextureKind::Mask),
        (r"x\detailmaps\drevoleta_detail.paa", TextureKind::Detail),
        (r"x\missile_aa_01_ns.paa", TextureKind::Normal),
        (r"x\titan\data\missilem_at_ti_co.paa", TextureKind::Color),
        (
            r"a3\characters_f\civil\data\hat_TI_ca.paa",
            TextureKind::Thermal,
        ),
    ];
    for (path, kind) in cases {
        assert_eq!(TextureKind::from_path(path), kind, "{path}");
    }
}

#[test]
fn names_without_a_known_suffix_are_unknown() {
    assert_eq!(
        TextureKind::from_path(r"a3\data_f\default.pac"),
        TextureKind::Unknown
    );
    assert_eq!(
        TextureKind::from_path("noise_wat.paa"),
        TextureKind::Unknown
    );
    assert_eq!(
        TextureKind::from_path(r"ammo\data\ammorail_01_ti.paa"),
        TextureKind::Unknown
    );
}

#[test]
fn normal_maps_and_colour_maps_are_classified() {
    assert!(TextureKind::NormalHq.is_normal_map());
    assert!(!TextureKind::Color.is_normal_map());
    assert!(TextureKind::Color.is_color());
    assert!(!TextureKind::SpecularMetalDetail.is_color());
    assert_eq!(TextureKind::NormalHq.suffix(), Some("nohq"));
}

#[test]
fn each_kind_has_the_texture_type_binarize_records_in_texheaders() {
    assert_eq!(TextureKind::Color.texture_type(), TextureType::Diffuse);
    assert_eq!(TextureKind::Sky.texture_type(), TextureType::DiffuseLinear);
    assert_eq!(TextureKind::NormalHq.texture_type(), TextureType::Normal);
    assert_eq!(
        TextureKind::SpecularMetalDetail.texture_type(),
        TextureType::Specular
    );
    assert_eq!(TextureKind::Mask.texture_type(), TextureType::Mask);
    assert_eq!(
        TextureType::from_index(11),
        Some(TextureType::DetailSpecular)
    );
    assert_eq!(TextureType::from_index(99), None);
    assert_eq!(TextureType::Macro.index(), 7);
}

#[test]
fn texture_type_from_a_path_follows_the_engine_rule() {
    let cases = [
        (
            r"a3\map_altis\data\sky_clear_lco.paa",
            TextureType::DiffuseLinear,
        ),
        (
            r"a3\map_altis\data\layers\00_00\s_002_016_lco.paa",
            TextureType::Diffuse,
        ),
        (
            r"a3\map_altis\data\layers\00_00\m_002_016_lca.paa",
            TextureType::Diffuse,
        ),
        (r"x\hat_ti_ca.paa", TextureType::Thermal),
        (r"x\missilem_at_ti_co.paa", TextureType::Diffuse),
        (r"x\Armor1_NOHQ.paa", TextureType::Normal),
        (r"x\y_novhq.paa", TextureType::Normal),
        (r"x\y_normalmap.paa", TextureType::Normal),
        (r"x\drevoleta_detail.paa", TextureType::Detail),
        (r"x\l_middle_mco.paa", TextureType::Detail),
        (r"x\y_sm.paa", TextureType::Specular),
        (r"x\y_dtsmdi.paa", TextureType::DetailSpecular),
        (r"x\y_sky.paa", TextureType::DiffuseLinear),
        (r"x\y_mask.paa", TextureType::Mask),
        (r"x\nounderscore.paa", TextureType::Diffuse),
        // The engine matches `_as.`: without an extension nothing matches.
        (r"x\y_as", TextureType::Diffuse),
    ];
    for (path, ty) in cases {
        assert_eq!(TextureType::from_path(path), ty, "{path}");
    }
}
