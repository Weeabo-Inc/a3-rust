//! Encoding RGBA8 images into textures and decoding them back.

use a3_paa::{AlphaFlags, Color, EncodeOptions, PixelFormat, Texture, decode_rgba8, encode_rgba8};

fn solid(width: usize, height: usize, rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat(width * height)
}

#[test]
fn an_opaque_dxt1_image_mips_until_the_short_side_is_4_and_gets_its_average_colour() {
    let image = solid(16, 8, [255, 0, 0, 255]);
    let texture = encode_rgba8(16, 8, &image, &EncodeOptions::new(PixelFormat::Dxt1)).unwrap();

    let sizes: Vec<_> = texture.mips.iter().map(|m| (m.width, m.height)).collect();
    assert_eq!(sizes, [(16, 8), (8, 4)]);
    assert_eq!(texture.average_color, Some(Color::rgba(255, 0, 0, 255)));
    assert_eq!(texture.max_color, Some(Color::WHITE));
    assert_eq!(texture.flags, None);

    let reread = Texture::read(&texture.to_bytes().unwrap()).unwrap();
    let pixels = decode_rgba8(reread.format, &reread.mips[0]).unwrap();
    assert_eq!(pixels, image);
}

#[test]
fn an_argb8888_image_round_trips_exactly_and_mips_down_to_1x1() {
    let image: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 3) as u8).collect();
    let texture = encode_rgba8(4, 4, &image, &EncodeOptions::new(PixelFormat::Argb8888)).unwrap();

    assert_eq!(texture.mips.len(), 3);
    let reread = Texture::read(&texture.to_bytes().unwrap()).unwrap();
    assert_eq!(decode_rgba8(reread.format, &reread.mips[0]).unwrap(), image);
    assert_eq!((reread.mips[2].width, reread.mips[2].height), (1, 1));
}

#[test]
fn alpha_decides_the_flag_tagg() {
    let mut binary = solid(8, 8, [0, 0, 255, 255]);
    binary[3] = 0;
    let options = EncodeOptions::new(PixelFormat::Dxt5);
    let texture = encode_rgba8(8, 8, &binary, &options).unwrap();
    assert_eq!(texture.flags, Some(AlphaFlags(AlphaFlags::BINARY)));

    let soft = solid(8, 8, [0, 0, 255, 128]);
    let texture = encode_rgba8(8, 8, &soft, &options).unwrap();
    assert_eq!(texture.flags, Some(AlphaFlags(AlphaFlags::INTERPOLATED)));
    let pixels = decode_rgba8(texture.format, &texture.mips[0]).unwrap();
    assert_eq!(&pixels[..4], &[0, 0, 255, 128]);
}

#[test]
fn rejects_an_image_buffer_of_the_wrong_size() {
    let options = EncodeOptions::new(PixelFormat::Dxt5);
    assert!(encode_rgba8(8, 8, &[0; 10], &options).is_err());
}
