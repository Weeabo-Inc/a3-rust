//! Reader for P3D models in both encodings: MLOD (editable) and ODOL (binarised, shipped).
//!
//! [`Model::from_bytes`] detects the encoding and decodes the file into one shape: model-wide
//! [`ModelInfo`], the [`Skeleton`], binarised Model [`Animation`]s and a [`Lod`] per resolution.
//!
//! Every `.p3d` in the Arma 3 2.22 install is ODOL version 73; that is the ODOL version this
//! crate reads. The layout is documented in `docs/re/p3d-odol.md`.
//!
//! Render data: [`Lod::vertices`] holds one entry per render vertex, [`Lod::sections`] split the
//! faces by texture and material, and [`Lod::section_triangles`] gives a triangle index buffer.

mod error;
mod mlod;
mod model;
mod odol;
mod reader;
mod resolution;
mod rvmat;

pub use error::{Error, Result};
pub use model::*;
pub use resolution::{LodKind, LodResolution};
pub use rvmat::{RvMat, RvMatStage};

impl Model {
    /// Decodes a P3D file (MLOD or ODOL) from its bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Model> {
        let mut signature = [0; 4];
        let len = data.len().min(4);
        signature[..len].copy_from_slice(&data[..len]);
        match &signature {
            b"ODOL" => odol::read(data),
            b"MLOD" => mlod::read(data),
            _ => Err(Error::UnknownSignature(signature)),
        }
    }
}
