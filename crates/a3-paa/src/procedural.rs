//! Procedural textures: `#(argb,8,8,3)color(1,0,0,1)` and friends.
//!
//! Wherever a texture path is accepted (rvmat stages, model faces, `hiddenSelectionsTextures`,
//! UI `text` entries), a string starting with `#` describes a texture the engine generates:
//!
//! ```text
//! #(format,width,height,mipmaps)function(arguments)
//! ```
//!
//! `format` is one of `rgb`, `argb`, `ai`, `a`, `i`; `mipmaps` is the number of levels. No
//! spaces are allowed and decimals need a leading zero. Identical strings share one generated
//! texture. The functions found in shipped data are listed in `docs/re/paa.md`; only `color`
//! is generated here, the others parse to [`ProceduralFunction::Other`].

use std::fmt;

use crate::{AlphaFlags, Color, Compression, Error, Mip, PixelFormat, Result, Texture};

/// The colour format in a procedural texture string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorFormat {
    /// `rgb`: colour, alpha forced opaque.
    Rgb,
    /// `argb`: colour and alpha.
    Argb,
    /// `ai`: intensity (from the red argument) and alpha.
    Ai,
    /// `a`: alpha only (colour white).
    A,
    /// `i`: intensity only (from the red argument), opaque.
    I,
}

impl ColorFormat {
    fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "rgb" => Self::Rgb,
            "argb" => Self::Argb,
            "ai" => Self::Ai,
            "a" => Self::A,
            "i" => Self::I,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Argb => "argb",
            Self::Ai => "ai",
            Self::A => "a",
            Self::I => "i",
        }
    }
}

/// The generator part of a procedural texture string.
#[derive(Debug, Clone, PartialEq)]
pub enum ProceduralFunction {
    /// `color(r,g,b,a)` or `color(r,g,b,a,TAG)`: a solid colour. Channels are 0..1; the
    /// optional tag (`CO`, `NOHQ`, `SMDI`, ...) tells the engine which texture type to treat
    /// the result as.
    Color {
        /// Red, green, blue, alpha in 0..1.
        rgba: [f32; 4],
        /// Texture type tag, without quotes.
        tag: Option<String>,
    },
    /// Any other generator (`fresnel`, `irradiance`, `perlinNoise`, ...), with its raw
    /// arguments.
    Other {
        /// Function name as written.
        name: String,
        /// Arguments as written, without surrounding quotes.
        args: Vec<String>,
    },
}

/// A parsed procedural texture string.
#[derive(Debug, Clone, PartialEq)]
pub struct Procedural {
    /// The colour format.
    pub format: ColorFormat,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Number of mipmap levels to generate.
    pub mip_count: u8,
    /// The generator.
    pub function: ProceduralFunction,
}

fn error<T>(text: &str, reason: impl Into<String>) -> Result<T> {
    Err(Error::Procedural {
        text: text.to_owned(),
        reason: reason.into(),
    })
}

impl Procedural {
    /// `true` when `text` names a procedural texture rather than a file.
    pub fn is_procedural(text: &str) -> bool {
        text.trim_start().starts_with('#')
    }

    /// Parses a procedural texture string.
    pub fn parse(text: &str) -> Result<Self> {
        let body = text.trim();
        let Some(body) = body.strip_prefix("#(") else {
            return error(text, "does not start with `#(`");
        };
        let Some((head, rest)) = body.split_once(')') else {
            return error(text, "unclosed `(` after `#`");
        };
        let head: Vec<&str> = head.split(',').map(str::trim).collect();
        let [format, width, height, mips] = head[..] else {
            return error(text, "expected `#(format,width,height,mipmaps)`");
        };
        let format = ColorFormat::parse(format)
            .map_or_else(|| error(text, format!("unknown format {format:?}")), Ok)?;
        let number = |s: &str, what: &str| -> Result<u16> {
            s.parse()
                .map_or_else(|_| error(text, format!("bad {what} {s:?}")), Ok)
        };
        let (width, height) = (number(width, "width")?, number(height, "height")?);
        let mip_count = number(mips, "mipmap count")?;
        if width == 0 || height == 0 || mip_count > 16 {
            return error(text, "zero size or more than 16 mipmaps");
        }

        let Some((name, args)) = rest.split_once('(') else {
            return error(text, "missing function arguments");
        };
        let Some(args) = args.trim_end().strip_suffix(')') else {
            return error(text, "unclosed function arguments");
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return error(text, format!("bad function name {name:?}"));
        }
        let args: Vec<String> = if args.trim().is_empty() {
            Vec::new()
        } else {
            args.split(',')
                .map(|a| a.trim().trim_matches('"').to_owned())
                .collect()
        };

        let function = if name.eq_ignore_ascii_case("color") {
            if !(4..=5).contains(&args.len()) {
                return error(text, "color takes r,g,b,a and an optional tag");
            }
            let mut rgba = [0f32; 4];
            for (v, arg) in rgba.iter_mut().zip(&args) {
                *v = arg
                    .parse()
                    .map_or_else(|_| error(text, format!("bad colour value {arg:?}")), Ok)?;
            }
            ProceduralFunction::Color {
                rgba,
                tag: args.get(4).cloned(),
            }
        } else {
            ProceduralFunction::Other {
                name: name.to_owned(),
                args,
            }
        };
        Ok(Self {
            format,
            width,
            height,
            mip_count: mip_count as u8,
            function,
        })
    }

    /// Generates the texture: ARGB8888 for `rgb`/`argb`, AI88 for `ai`/`a`/`i`, with
    /// `mip_count` levels (at least one; halving down to 1x1 at most).
    ///
    /// Only `color` is implemented; other functions return [`Error::Procedural`].
    pub fn generate(&self) -> Result<Texture> {
        let ProceduralFunction::Color { rgba, .. } = &self.function else {
            return error(&self.to_string(), "only color() can be generated");
        };
        let [r, g, b, a] = rgba.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let pixel = match self.format {
            ColorFormat::Rgb => Color::rgba(r, g, b, 255),
            ColorFormat::Argb => Color::rgba(r, g, b, a),
            ColorFormat::Ai => Color::rgba(r, r, r, a),
            ColorFormat::A => Color::rgba(255, 255, 255, a),
            ColorFormat::I => Color::rgba(r, r, r, 255),
        };
        let format = match self.format {
            ColorFormat::Rgb | ColorFormat::Argb => PixelFormat::Argb8888,
            ColorFormat::Ai | ColorFormat::A | ColorFormat::I => PixelFormat::Ai88,
        };
        let texel: Vec<u8> = match format {
            PixelFormat::Argb8888 => pixel.to_bgra().to_vec(),
            _ => vec![pixel.r, pixel.a],
        };
        let (mut w, mut h) = (self.width, self.height);
        let mut mips = Vec::new();
        for _ in 0..self.mip_count.max(1) {
            mips.push(Mip {
                width: w,
                height: h,
                data: texel.repeat(usize::from(w) * usize::from(h)),
                compression: Compression::None,
            });
            if w == 1 && h == 1 {
                break;
            }
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
        let flags = match pixel.a {
            255 => None,
            0 => Some(AlphaFlags(AlphaFlags::BINARY)),
            _ => Some(AlphaFlags(AlphaFlags::INTERPOLATED)),
        };
        Ok(Texture {
            format,
            average_color: Some(pixel),
            max_color: Some(Color::WHITE),
            flags,
            procedural: Some(self.to_string()),
            mips,
            ..Texture::default()
        })
    }
}

impl fmt::Display for Procedural {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#({},{},{},{})",
            self.format.name(),
            self.width,
            self.height,
            self.mip_count
        )?;
        match &self.function {
            ProceduralFunction::Color { rgba, tag } => {
                let [r, g, b, a] = rgba;
                write!(f, "color({r},{g},{b},{a}")?;
                if let Some(tag) = tag {
                    write!(f, ",{tag}")?;
                }
                write!(f, ")")
            }
            ProceduralFunction::Other { name, args } => write!(f, "{name}({})", args.join(",")),
        }
    }
}
