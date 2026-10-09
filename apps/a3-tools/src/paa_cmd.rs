//! `a3-tools paa ...`: inspect and convert PAA/PAC textures and texHeaders.bin.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use a3_paa::{
    EncodeOptions, PaaHeader, PixelFormat, Procedural, TexHeaders, TextureKind, decode_rgba8,
    encode_rgba8,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct PaaArgs {
    /// Game install folder, for inputs given as VFS paths.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: PaaCommand,
}

#[derive(Subcommand)]
enum PaaCommand {
    /// Print a texture's format, TAGGs and mipmap table.
    Info {
        /// A .paa/.pac file, or a VFS path (looked up in the game at --game-dir).
        input: String,
    },
    /// Decode one mipmap to an RGBA PNG.
    Topng {
        /// A .paa/.pac file, a VFS path, or a procedural texture string such as
        /// `#(ai,64,64,1)fresnel(1.3,7)`.
        input: String,
        /// Output PNG file.
        output: PathBuf,
        /// Mipmap index (0 = largest).
        #[arg(long, default_value_t = 0)]
        mip: usize,
        /// Undo the channel swizzle recorded in the texture (e.g. for `_nohq`, `_sky`).
        #[arg(long)]
        unswizzle: bool,
    },
    /// Encode a PNG as a PAA.
    Frompng {
        /// Input PNG file.
        input: PathBuf,
        /// Output PAA file.
        output: PathBuf,
        /// Pixel format: dxt1, dxt3, dxt5, argb8888, argb4444, argb1555, ai88.
        #[arg(long, default_value = "dxt5")]
        format: String,
        /// Write only the full-size level.
        #[arg(long)]
        no_mipmaps: bool,
    },
    /// List the entries of a texHeaders.bin.
    Texheaders {
        /// A texHeaders.bin file, or a VFS path.
        input: String,
    },
}

/// Runs a `paa` subcommand.
pub fn run(args: PaaArgs) -> Result<()> {
    let game_dir = args.game_dir.as_deref();
    match args.command {
        PaaCommand::Info { input } => info(&input, game_dir),
        PaaCommand::Topng {
            input,
            output,
            mip,
            unswizzle,
        } => to_png(&input, &output, mip, unswizzle, game_dir),
        PaaCommand::Frompng {
            input,
            output,
            format,
            no_mipmaps,
        } => from_png(&input, &output, parse_format(&format)?, !no_mipmaps),
        PaaCommand::Texheaders { input } => texheaders(&input, game_dir),
    }
}

fn load(input: &str, game_dir: Option<&Path>) -> Result<Vec<u8>> {
    crate::vfs_cmd::read_file_or_vfs(input, game_dir)
}

/// Prints the header, TAGGs and mipmap table of a texture.
pub fn info(input: &str, game_dir: Option<&Path>) -> Result<()> {
    let data = load(input, game_dir)?;
    let header = PaaHeader::read(&data).context("cannot parse PAA")?;
    let meta = &header.meta;
    println!("format     {:?}", meta.format);
    println!("kind       {:?}", TextureKind::from_path(input));
    if let Some(c) = meta.average_color {
        println!(
            "average    #{:02x}{:02x}{:02x}{:02x} (RGBA)",
            c.r, c.g, c.b, c.a
        );
    }
    if let Some(c) = meta.max_color {
        println!(
            "max        #{:02x}{:02x}{:02x}{:02x} (RGBA)",
            c.r, c.g, c.b, c.a
        );
    }
    if let Some(flags) = meta.flags {
        println!(
            "flags      {} (interpolated alpha: {}, binary alpha: {})",
            flags.0,
            flags.is_interpolated(),
            flags.is_binary()
        );
    }
    if let Some(swizzle) = meta.swizzle {
        println!(
            "swizzle    A<-{:?} R<-{:?} G<-{:?} B<-{:?}",
            swizzle.a, swizzle.r, swizzle.g, swizzle.b
        );
    }
    if let Some(text) = &meta.procedural {
        println!("procedural {text}");
    }
    for tagg in &meta.other_taggs {
        println!(
            "tagg       {} ({} bytes)",
            String::from_utf8_lossy(&tagg.name),
            tagg.data.len()
        );
    }
    if !meta.palette.is_empty() {
        println!("palette    {} colours", meta.palette.len());
    }
    println!(
        "{:>4}  {:>11}  {:>8}  {:>10}  {:>10}",
        "mip", "size", "storage", "offset", "stored"
    );
    for (i, mip) in header.mips.iter().enumerate() {
        println!(
            "{i:>4}  {:>11}  {:>8}  {:>10}  {:>10}",
            format!("{}x{}", mip.width, mip.height),
            format!("{:?}", mip.compression),
            mip.offset,
            mip.stored_len
        );
    }
    Ok(())
}

/// Writes mipmap `mip` of a texture as an RGBA PNG.
pub fn to_png(
    input: &str,
    output: &Path,
    mip: usize,
    unswizzle: bool,
    game_dir: Option<&Path>,
) -> Result<()> {
    let (meta, level) = if Procedural::is_procedural(input) {
        let mut texture = Procedural::parse(input)?.generate()?;
        if mip >= texture.mips.len() {
            bail!("the texture has {} mipmaps", texture.mips.len());
        }
        let level = texture.mips.swap_remove(mip);
        (texture, level)
    } else {
        let data = load(input, game_dir)?;
        let header = PaaHeader::read(&data).context("cannot parse PAA")?;
        let level = header.read_mip(&data, mip)?;
        (header.meta, level)
    };
    let mut rgba = decode_rgba8(meta.format, &level)?;
    if let (true, Some(swizzle)) = (unswizzle, meta.swizzle) {
        swizzle.restore(&mut rgba);
    }
    write_png(
        output,
        u32::from(level.width),
        u32::from(level.height),
        &rgba,
    )?;
    eprintln!(
        "wrote {} ({}x{} {:?})",
        output.display(),
        level.width,
        level.height,
        meta.format
    );
    Ok(())
}

pub(crate) fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let file = File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(())
}

/// Reads a PNG as RGBA8.
pub(crate) fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut buf)?;
    buf.truncate(frame.buffer_size());
    let pixels = (frame.width * frame.height) as usize;
    let rgba = match frame.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => bail!("unsupported PNG colour type {other:?}"),
    };
    debug_assert_eq!(rgba.len(), pixels * 4);
    Ok((frame.width, frame.height, rgba))
}

/// Parses a `--format` value.
pub fn parse_format(name: &str) -> Result<PixelFormat> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "dxt1" => PixelFormat::Dxt1,
        "dxt3" => PixelFormat::Dxt3,
        "dxt5" => PixelFormat::Dxt5,
        "argb8888" => PixelFormat::Argb8888,
        "argb4444" => PixelFormat::Argb4444,
        "argb1555" => PixelFormat::Argb1555,
        "ai88" => PixelFormat::Ai88,
        other => {
            bail!("unknown format {other:?} (dxt1, dxt3, dxt5, argb8888, argb4444, argb1555, ai88)")
        }
    })
}

/// Converts a PNG into a PAA.
pub fn from_png(input: &Path, output: &Path, format: PixelFormat, mipmaps: bool) -> Result<()> {
    let (width, height, rgba) = read_png(input)?;
    let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
        bail!("{width}x{height} is too large for a PAA");
    };
    let options = EncodeOptions {
        mipmaps,
        ..EncodeOptions::new(format)
    };
    let texture = encode_rgba8(w, h, &rgba, &options)?;
    let bytes = texture.to_bytes()?;
    std::fs::write(output, &bytes).with_context(|| format!("cannot write {}", output.display()))?;
    eprintln!(
        "wrote {} ({width}x{height} {format:?}, {} mipmaps, {} bytes)",
        output.display(),
        texture.mips.len(),
        bytes.len()
    );
    Ok(())
}

/// Prints the entries of a texHeaders.bin.
pub fn texheaders(input: &str, game_dir: Option<&Path>) -> Result<()> {
    let data = load(input, game_dir)?;
    let headers = TexHeaders::read(&data).context("cannot parse texHeaders.bin")?;
    println!("{} textures", headers.textures.len());
    for t in &headers.textures {
        let (w, h) = t.mips.first().map_or((0, 0), |m| (m.width, m.height));
        let format = t
            .format
            .map_or_else(|| format!("#{}", t.format_index), |f| format!("{f:?}"));
        let kind = t.texture_type().map_or_else(
            || format!("#{}", t.texture_type_index),
            |k| format!("{k:?}"),
        );
        println!(
            "{format:>9} {:>11} {:>2} mips  {kind:>14}  {:>9} bytes  {}",
            format!("{w}x{h}"),
            t.mips.len(),
            t.file_size,
            t.path
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_to_paa_and_back_preserves_argb8888_pixels() {
        let tmp = tempfile::tempdir().unwrap();
        let rgba: Vec<u8> = (0..8 * 4 * 4).map(|i| (i * 7) as u8).collect();
        let src = tmp.path().join("in.png");
        write_png(&src, 8, 4, &rgba).unwrap();

        let paa = tmp.path().join("out.paa");
        from_png(&src, &paa, parse_format("ARGB8888").unwrap(), true).unwrap();
        let back = tmp.path().join("back.png");
        to_png(paa.to_str().unwrap(), &back, 0, false, None).unwrap();

        assert_eq!(read_png(&back).unwrap(), (8, 4, rgba));
        info(paa.to_str().unwrap(), None).unwrap();
    }

    #[test]
    fn a_vfs_path_without_a_game_dir_is_an_error() {
        assert!(load(r"a3\data_f\nothere.paa", None).is_err());
        assert!(parse_format("dxt9").is_err());
    }
}
