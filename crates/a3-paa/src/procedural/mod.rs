//! Procedural textures: `#(argb,8,8,3)color(1,0,0,1)` and friends.
//!
//! Wherever a texture path is accepted (rvmat stages, model faces, `hiddenSelectionsTextures`,
//! UI `text` entries), a string starting with `#` describes a texture the engine generates:
//!
//! ```text
//! #(format,width,height,mipmaps)function(arguments)
//! ```
//!
//! Parsing follows the engine's rules (`docs/re/paa.md`): `format` is `ai`, `a`, `i` (all AI88)
//! or `argb`, `rgb` (both ARGB8888); width and height are powers of two; the larger side must
//! allow the mipmap count; every numeric argument starts with a digit. Function names and
//! formats are case-insensitive. [`Procedural::generate`] reproduces the engine's generators;
//! the runtime sources (render-to-texture, text, UI, extensions) only parse.

mod dither;
mod generate;
mod noise;

use std::fmt;

use crate::{Error, Result, TextureType};

/// The colour format in a procedural texture string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorFormat {
    /// `argb`: ARGB8888.
    Argb,
    /// `rgb`: ARGB8888 as well; alpha is not forced opaque.
    Rgb,
    /// `ai`: AI88 (intensity and alpha).
    Ai,
    /// `a`: AI88, same as `ai`.
    A,
    /// `i`: AI88, same as `ai`.
    I,
}

impl ColorFormat {
    fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "argb" => Self::Argb,
            "rgb" => Self::Rgb,
            "ai" => Self::Ai,
            "a" => Self::A,
            "i" => Self::I,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Argb => "argb",
            Self::Rgb => "rgb",
            Self::Ai => "ai",
            Self::A => "a",
            Self::I => "i",
        }
    }

    /// `true` for the AI88 formats, `false` for ARGB8888.
    pub fn is_ai88(self) -> bool {
        matches!(self, Self::Ai | Self::A | Self::I)
    }
}

/// A runtime-only texture source: rendered by the engine while the game runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeSource {
    /// `r2t(name,aspect)`: a render-to-texture target.
    RenderToTexture,
    /// `text(...)`: rendered text.
    Text,
    /// `ui(...)`: a UI display rendered to a texture.
    Ui,
    /// `uiex(...)`: extended UI on a texture.
    UiEx,
    /// `extension(...)`: a texture provided by an engine extension.
    Extension,
}

impl RuntimeSource {
    fn name(self) -> &'static str {
        match self {
            Self::RenderToTexture => "r2t",
            Self::Text => "text",
            Self::Ui => "ui",
            Self::UiEx => "uiex",
            Self::Extension => "extension",
        }
    }
}

/// The generator part of a procedural texture string.
#[derive(Debug, Clone, PartialEq)]
pub enum ProceduralFunction {
    /// `color(r,g,b,a[,tag])`: a solid colour (always generated as one 1x1 pixel). The
    /// optional tag is a file-name suffix (`co`, `nohq`, `mc`, ...) giving the texture type.
    Color {
        /// Red, green, blue, alpha, nominally 0..1.
        rgba: [f32; 4],
        /// Texture suffix, without `_`.
        tag: Option<String>,
    },
    /// `irradiance(power)`: specular lookup; intensity ramps across, alpha = `(y)^power`.
    Irradiance {
        /// Specular power.
        power: f32,
    },
    /// `dither(a,b)`: ordered dither matrix (AI88 only).
    Dither {
        /// First level weight (rounded to an integer).
        a: i32,
        /// Second level weight (rounded to an integer).
        b: i32,
    },
    /// `perlinNoise(xScale,yScale,min,max)`: Ken Perlin's improved noise mapped to
    /// `min..max`.
    PerlinNoise {
        /// Noise periods across the width.
        x_scale: f32,
        /// Noise periods across the height.
        y_scale: f32,
        /// Output for noise -1.
        min: f32,
        /// Output for noise +1.
        max: f32,
    },
    /// `waterIrradiance(power)`: air-to-water Fresnel in alpha, `(y)^power` in intensity.
    WaterIrradiance {
        /// Specular power.
        power: f32,
    },
    /// `fresnelGlass(n)`: dielectric Fresnel reflectance lookup (default n = 1.7).
    FresnelGlass {
        /// Refractive index.
        n: f32,
    },
    /// `treeCrown(density)`: `density^x` in intensity, `density^y` in alpha.
    TreeCrown {
        /// Density.
        density: f32,
    },
    /// `treeCrownAmb(density)`: ambient tree crown attenuation over a half disc.
    TreeCrownAmb {
        /// Density.
        density: f32,
    },
    /// `point()`: white disc with alpha falling off linearly from the centre.
    Point,
    /// `fresnel(n,k)`: conductor Fresnel reflectance lookup (defaults n = 0.96977,
    /// k = 0.0118).
    Fresnel {
        /// Refractive index.
        n: f32,
        /// Extinction coefficient.
        k: f32,
    },
    /// A source the engine renders at run time; the arguments are kept verbatim.
    Runtime {
        /// Which source.
        source: RuntimeSource,
        /// The raw argument text.
        args: String,
    },
}

/// A parsed procedural texture string.
#[derive(Debug, Clone, PartialEq)]
pub struct Procedural {
    /// The colour format.
    pub format: ColorFormat,
    /// Declared width in pixels (a power of two).
    pub width: u16,
    /// Declared height in pixels (a power of two).
    pub height: u16,
    /// Declared number of mipmap levels.
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

/// Splits a leading decimal number (`strtod` syntax without sign, hex or inf/nan) off `text`.
/// The engine requires a digit first.
fn leading_number(text: &str) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut end = digits(0);
    if bytes.get(end) == Some(&b'.') {
        end += 1 + digits(end + 1);
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exp = end + 1;
        if matches!(bytes.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        let n = digits(exp);
        if n > 0 {
            end = exp + n;
        }
    }
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// Reads comma-separated numbers the way the engine's argument parsers do: each must start
/// with a digit and be followed by `,` or the end. Returns the numbers and whatever follows
/// the last consumed comma (empty at the end).
fn numbers(args: &str, count: usize) -> Option<(Vec<f32>, &str)> {
    let mut rest = args;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let (value, after) = leading_number(rest)?;
        out.push(value as f32);
        rest = match after.strip_prefix(',') {
            Some(next) => next,
            None if after.is_empty() => after,
            None => return None,
        };
        if rest.is_empty() && i + 1 < count {
            return None;
        }
    }
    Some((out, rest))
}

/// A leading unsigned integer (`strtol` after an `isdigit` check).
fn int(s: &str) -> Option<(u32, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

/// Exactly `count` numbers and nothing else.
fn exact_numbers(args: &str, count: usize) -> Option<Vec<f32>> {
    let mut rest = args;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let (value, after) = leading_number(rest)?;
        out.push(value as f32);
        rest = if i + 1 < count {
            after.strip_prefix(',')?
        } else if after.is_empty() {
            after
        } else {
            return None;
        };
    }
    Some(out)
}

impl Procedural {
    /// `true` when `text` names a procedural texture rather than a file.
    pub fn is_procedural(text: &str) -> bool {
        text.starts_with('#')
    }

    /// Parses a procedural texture string as the engine does.
    pub fn parse(text: &str) -> Result<Self> {
        let Some(body) = text.strip_prefix("#(") else {
            return error(text, "does not start with `#(`");
        };
        let Some((format, rest)) = body.split_once(',') else {
            return error(text, "expected `#(format,width,height,mipmaps)`");
        };
        let Some(format) = ColorFormat::parse(format) else {
            return error(text, format!("unknown format {format:?}"));
        };
        let header = (|| {
            let (width, rest) = int(rest)?;
            let (height, rest) = int(rest.strip_prefix(',')?)?;
            let (mips, rest) = int(rest.strip_prefix(',')?)?;
            Some((width, height, mips, rest.strip_prefix(')')?))
        })();
        let Some((width, height, mips, rest)) = header else {
            return error(text, "expected `#(format,width,height,mipmaps)`");
        };
        if mips < 1 || width < 1 || height < 1 {
            return error(text, "sizes and mipmap count must be at least 1");
        }
        if !width.is_power_of_two() || !height.is_power_of_two() || width.max(height) > 32768 {
            return error(text, "width and height must be powers of two");
        }
        if mips > 16 || width.max(height) < 1 << (mips - 1) {
            return error(text, "more mipmaps than the size allows");
        }

        let Some((name, rest)) = rest.split_once('(') else {
            return error(text, "missing function arguments");
        };
        // Arguments run to the first `)`, or to the last one when they contain quotes.
        let close = if rest.contains('"') {
            rest.rfind(')')
        } else {
            rest.find(')')
        };
        let Some(close) = close else {
            return error(text, "unclosed function arguments");
        };
        let args = &rest[..close];
        let function = parse_function(name, args).map_or_else(
            || error(text, format!("bad function or arguments {name}({args})")),
            Ok,
        )?;

        Ok(Self {
            format,
            width: width as u16,
            height: height as u16,
            mip_count: mips as u8,
            function,
        })
    }

    /// The texture type the engine assigns to the generated texture.
    pub fn texture_type(&self) -> TextureType {
        match &self.function {
            ProceduralFunction::Color { rgba, tag } => color_texture_type(*rgba, tag.as_deref()),
            ProceduralFunction::Irradiance { .. }
            | ProceduralFunction::WaterIrradiance { .. }
            | ProceduralFunction::FresnelGlass { .. }
            | ProceduralFunction::Fresnel { .. } => TextureType::Irradiance,
            ProceduralFunction::TreeCrown { .. } | ProceduralFunction::TreeCrownAmb { .. } => {
                TextureType::TreeCrown
            }
            ProceduralFunction::PerlinNoise { .. } | ProceduralFunction::Point => {
                TextureType::Detail
            }
            ProceduralFunction::Dither { .. } => TextureType::Dither,
            ProceduralFunction::Runtime { .. } => TextureType::Diffuse,
        }
    }
}

fn parse_function(name: &str, args: &str) -> Option<ProceduralFunction> {
    use ProceduralFunction as F;
    let one = |args| exact_numbers(args, 1).map(|v| v[0]);
    let runtime = |source| {
        Some(F::Runtime {
            source,
            args: args.to_owned(),
        })
    };
    Some(match name.to_ascii_lowercase().as_str() {
        "color" => {
            let (v, rest) = numbers(args, 4)?;
            F::Color {
                rgba: [v[0], v[1], v[2], v[3]],
                tag: (!rest.is_empty()).then(|| rest.to_owned()),
            }
        }
        "irradiance" => F::Irradiance { power: one(args)? },
        "dither" => {
            let v = exact_numbers(args, 2)?;
            F::Dither {
                a: v[0].round_ties_even() as i32,
                b: v[1].round_ties_even() as i32,
            }
        }
        "perlinnoise" => {
            let v = exact_numbers(args, 4)?;
            F::PerlinNoise {
                x_scale: v[0],
                y_scale: v[1],
                min: v[2],
                max: v[3],
            }
        }
        "waterirradiance" => F::WaterIrradiance { power: one(args)? },
        "fresnelglass" => {
            let n = if args.is_empty() { 1.7 } else { one(args)? };
            // The engine warns and clamps a non-positive index.
            F::FresnelGlass {
                n: if n <= 0.0 { 0.001 } else { n },
            }
        }
        "treecrown" => F::TreeCrown {
            density: one(args)?,
        },
        "treecrownamb" => F::TreeCrownAmb {
            density: one(args)?,
        },
        "point" => F::Point,
        "fresnel" => {
            let (n, k) = if args.is_empty() {
                (0.96977, 0.0118)
            } else {
                let v = exact_numbers(args, 2)?;
                (v[0], v[1])
            };
            F::Fresnel {
                n: if n <= 0.0 { 0.001 } else { n },
                k: if k <= 0.0 { 0.001 } else { k },
            }
        }
        "r2t" => return runtime(RuntimeSource::RenderToTexture),
        "text" => return runtime(RuntimeSource::Text),
        "ui" => return runtime(RuntimeSource::Ui),
        "uiex" => return runtime(RuntimeSource::UiEx),
        "extension" => return runtime(RuntimeSource::Extension),
        _ => return None,
    })
}

/// Colours the engine recognises when `color()` has no tag, with the texture type each
/// implies: the nearest one (squared RGBA distance below 0.5) wins, otherwise diffuse.
const KNOWN_COLORS: [([f32; 4], TextureType); 7] = [
    ([0.5, 0.5, 0.5, 1.0], TextureType::Detail),
    ([0.5, 0.5, 1.0, 1.0], TextureType::Normal),
    ([1.0, 1.0, 1.0, 1.0], TextureType::DiffuseLinear),
    ([0.0, 0.0, 0.0, 0.0], TextureType::Macro),
    ([1.0, 1.0, 1.0, 0.0], TextureType::Macro),
    ([1.0, 0.0, 0.0, 1.0], TextureType::Specular),
    ([1.0, 0.0, 1.0, 1.0], TextureType::Specular),
];

fn color_texture_type(rgba: [f32; 4], tag: Option<&str>) -> TextureType {
    if let Some(tag) = tag {
        return TextureType::from_path(&format!("_{tag}."));
    }
    let mut best = (f32::MAX, TextureType::Diffuse);
    for (known, ty) in KNOWN_COLORS {
        let d = rgba
            .iter()
            .zip(known)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>();
        if d < best.0 {
            best = (d, ty);
        }
    }
    if best.0 < 0.5 {
        best.1
    } else {
        TextureType::Diffuse
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
        use ProceduralFunction as F;
        match &self.function {
            F::Color { rgba, tag } => {
                let [r, g, b, a] = rgba;
                write!(f, "color({r},{g},{b},{a}")?;
                if let Some(tag) = tag {
                    write!(f, ",{tag}")?;
                }
                write!(f, ")")
            }
            F::Irradiance { power } => write!(f, "irradiance({power})"),
            F::Dither { a, b } => write!(f, "dither({a},{b})"),
            F::PerlinNoise {
                x_scale,
                y_scale,
                min,
                max,
            } => write!(f, "perlinNoise({x_scale},{y_scale},{min},{max})"),
            F::WaterIrradiance { power } => write!(f, "waterIrradiance({power})"),
            F::FresnelGlass { n } => write!(f, "fresnelGlass({n})"),
            F::TreeCrown { density } => write!(f, "treeCrown({density})"),
            F::TreeCrownAmb { density } => write!(f, "treeCrownAmb({density})"),
            F::Point => write!(f, "point()"),
            F::Fresnel { n, k } => write!(f, "fresnel({n},{k})"),
            F::Runtime { source, args } => write!(f, "{}({args})", source.name()),
        }
    }
}
