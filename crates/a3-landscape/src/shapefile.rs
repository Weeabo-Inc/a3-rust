//! ESRI shapefile (`.shp`) and dBase III table (`.dbf`) readers, as used by terrain road
//! networks. Only 2D geometry is kept; Z and M values are skipped.

use glam::DVec2;

/// Errors from reading a shapefile or dBase table.
#[derive(Debug, thiserror::Error)]
pub enum ShapefileError {
    /// The data ended inside a header or record.
    #[error("unexpected end of {file} at offset {offset}")]
    Truncated {
        /// `"shp"` or `"dbf"`.
        file: &'static str,
        /// Where the missing data starts.
        offset: usize,
    },
    /// The data is not a shapefile / dBase table, or holds an unsupported shape type.
    #[error("invalid {file} at offset {offset}: {detail}")]
    Invalid {
        /// `"shp"` or `"dbf"`.
        file: &'static str,
        /// Where the problem is.
        offset: usize,
        /// What is wrong.
        detail: String,
    },
}

type Result<T> = std::result::Result<T, ShapefileError>;

/// One geometry record of a shapefile.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// No geometry (type 0).
    Null,
    /// A point (types 1, 11, 21).
    Point(DVec2),
    /// Points (types 8, 18, 28).
    MultiPoint(Vec<DVec2>),
    /// Polylines, one point list per part (types 3, 13, 23).
    PolyLine(Vec<Vec<DVec2>>),
    /// Polygon rings (types 5, 15, 25).
    Polygon(Vec<Vec<DVec2>>),
}

/// A parsed `.shp` file.
#[derive(Debug, Clone, PartialEq)]
pub struct Shapefile {
    /// The file's shape type (3 = polyline).
    pub shape_type: u32,
    /// `[x_min, y_min, x_max, y_max]`.
    pub bbox: [f64; 4],
    /// The records in file order (record `i` matches dBase row `i`).
    pub shapes: Vec<Shape>,
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    file: &'static str,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.data.len() - self.pos < n {
            return Err(ShapefileError::Truncated {
                file: self.file,
                offset: self.pos,
            });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("length"))
    }
    fn u32_be(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    fn u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn f64_le(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    fn point(&mut self) -> Result<DVec2> {
        Ok(DVec2::new(self.f64_le()?, self.f64_le()?))
    }
    fn invalid(&self, detail: String) -> ShapefileError {
        ShapefileError::Invalid {
            file: self.file,
            offset: self.pos,
            detail,
        }
    }
}

impl Shapefile {
    /// Parses a `.shp` file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cursor {
            data,
            pos: 0,
            file: "shp",
        };
        let code = c.u32_be()?;
        if code != 9994 {
            return Err(c.invalid(format!("file code {code}, expected 9994")));
        }
        c.take(20)?;
        let words = c.u32_be()? as usize;
        let _version = c.u32_le()?;
        let shape_type = c.u32_le()?;
        let bbox = [c.f64_le()?, c.f64_le()?, c.f64_le()?, c.f64_le()?];
        c.take(32)?; // z and m ranges
        let end = (words * 2).min(data.len());
        let mut shapes = Vec::new();
        while c.pos + 8 <= end {
            let _number = c.u32_be()?;
            let len = c.u32_be()? as usize * 2;
            let start = c.pos;
            let body = c.take(len)?;
            let mut r = Cursor {
                data: body,
                pos: 0,
                file: "shp",
            };
            shapes.push(read_shape(&mut r).map_err(|e| shift(e, start))?);
        }
        Ok(Self {
            shape_type,
            bbox,
            shapes,
        })
    }
}

fn shift(e: ShapefileError, base: usize) -> ShapefileError {
    match e {
        ShapefileError::Truncated { file, offset } => ShapefileError::Truncated {
            file,
            offset: offset + base,
        },
        ShapefileError::Invalid {
            file,
            offset,
            detail,
        } => ShapefileError::Invalid {
            file,
            offset: offset + base,
            detail,
        },
    }
}

fn read_shape(r: &mut Cursor<'_>) -> Result<Shape> {
    let kind = r.u32_le()?;
    Ok(match kind {
        0 => Shape::Null,
        1 | 11 | 21 => Shape::Point(r.point()?),
        8 | 18 | 28 => {
            r.take(32)?;
            let n = count(r, 16)?;
            Shape::MultiPoint((0..n).map(|_| r.point()).collect::<Result<_>>()?)
        }
        3 | 13 | 23 | 5 | 15 | 25 => {
            r.take(32)?;
            let parts = count(r, 4)?;
            let points = count(r, 16)?;
            let mut starts = Vec::with_capacity(parts);
            for _ in 0..parts {
                starts.push(r.u32_le()? as usize);
            }
            let all: Vec<DVec2> = (0..points).map(|_| r.point()).collect::<Result<_>>()?;
            let mut lines = Vec::with_capacity(parts);
            for (i, &s) in starts.iter().enumerate() {
                let e = starts.get(i + 1).copied().unwrap_or(points);
                if s > e || e > points {
                    return Err(r.invalid(format!("part {i} spans {s}..{e} of {points} points")));
                }
                lines.push(all[s..e].to_vec());
            }
            if matches!(kind, 3 | 13 | 23) {
                Shape::PolyLine(lines)
            } else {
                Shape::Polygon(lines)
            }
        }
        other => return Err(r.invalid(format!("unsupported shape type {other}"))),
    })
}

fn count(r: &mut Cursor<'_>, item: usize) -> Result<usize> {
    let n = r.u32_le()? as usize;
    if n.saturating_mul(item) > r.data.len() - r.pos {
        return Err(r.invalid(format!("count {n} exceeds the record")));
    }
    Ok(n)
}

/// One column of a dBase table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbfField {
    /// Column name (upper case in shipped files, e.g. `ID`).
    pub name: String,
    /// dBase type letter: `C` text, `N` number, `F` float, `L` logical, `D` date.
    pub kind: char,
    /// Width in bytes.
    pub length: u8,
    /// Decimal places.
    pub decimals: u8,
}

/// A parsed dBase III table: the attributes of the shapefile records.
#[derive(Debug, Clone, PartialEq)]
pub struct Dbf {
    /// The columns.
    pub fields: Vec<DbfField>,
    /// One row per record, one trimmed string per column.
    pub records: Vec<Vec<String>>,
    /// Whether each record is marked deleted.
    pub deleted: Vec<bool>,
}

impl Dbf {
    /// Parses a `.dbf` file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cursor {
            data,
            pos: 0,
            file: "dbf",
        };
        let header: [u8; 32] = c.arr()?;
        let n = u32::from_le_bytes(header[4..8].try_into().expect("4")) as usize;
        let header_len = usize::from(u16::from_le_bytes([header[8], header[9]]));
        let record_len = usize::from(u16::from_le_bytes([header[10], header[11]]));
        let mut fields = Vec::new();
        while c.pos < header_len.saturating_sub(1) {
            if data.get(c.pos) == Some(&0x0d) {
                break;
            }
            let d: [u8; 32] = c.arr()?;
            let name_end = d[..11].iter().position(|&b| b == 0).unwrap_or(11);
            fields.push(DbfField {
                name: String::from_utf8_lossy(&d[..name_end]).into_owned(),
                kind: char::from(d[11]),
                length: d[16],
                decimals: d[17],
            });
        }
        let width: usize = 1 + fields.iter().map(|f| usize::from(f.length)).sum::<usize>();
        if width > record_len {
            return Err(c.invalid(format!(
                "fields need {width} bytes, records have {record_len}"
            )));
        }
        c.pos = header_len;
        let mut records = Vec::with_capacity(n);
        let mut deleted = Vec::with_capacity(n);
        for _ in 0..n {
            let rec = c.take(record_len)?;
            deleted.push(rec[0] == b'*');
            let mut off = 1;
            let row = fields
                .iter()
                .map(|f| {
                    let bytes = &rec[off..off + usize::from(f.length)];
                    off += usize::from(f.length);
                    bytes
                        .iter()
                        .map(|&b| char::from(b))
                        .collect::<String>()
                        .trim_matches([' ', '\0'])
                        .to_owned()
                })
                .collect();
            records.push(row);
        }
        Ok(Self {
            fields,
            records,
            deleted,
        })
    }

    /// The index of the column named `name` (case-insensitive).
    pub fn column(&self, name: &str) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// The value of column `name` in record `row`.
    pub fn get(&self, row: usize, name: &str) -> Option<&str> {
        let col = self.column(name)?;
        self.records.get(row)?.get(col).map(String::as_str)
    }
}
