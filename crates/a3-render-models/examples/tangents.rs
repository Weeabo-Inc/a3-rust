//! How do ODOL's stored tangents relate to the UV gradients? Prints the mean cosine between S
//! and dP/du and between T and dP/dv over all triangles of LOD 0.
use glam::Vec3;

fn main() {
    let root = std::env::var("A3_ROOT").unwrap();
    let vfs = a3_vfs::Vfs::new();
    vfs.mount_game(std::path::Path::new(&root), &[]);
    for path in std::env::args().skip(1) {
        let m = a3_p3d::Model::from_bytes(&vfs.open(&path).unwrap()).unwrap();
        let lod = &m.lods[0];
        let v = &lod.vertices;
        let (mut su, mut tv, mut sv, mut n) = (0.0f64, 0.0f64, 0.0f64, 0usize);
        let mut handed = 0i64;
        for tri in lod.triangles().chunks(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
            let (p0, p1, p2) = (v.positions[a], v.positions[b], v.positions[c]);
            let (t0, t1, t2) = (v.uv_sets[0][a], v.uv_sets[0][b], v.uv_sets[0][c]);
            let (e1, e2) = (p1 - p0, p2 - p0);
            let (d1, d2) = (t1 - t0, t2 - t0);
            let det = d1.x * d2.y - d2.x * d1.y;
            if det.abs() < 1e-8 {
                continue;
            }
            let dpdu: Vec3 = (e1 * d2.y - e2 * d1.y) / det;
            let dpdv: Vec3 = (e2 * d1.x - e1 * d2.x) / det;
            let [s, t] = v.tangents[a];
            if dpdu.length() < 1e-6 || dpdv.length() < 1e-6 || s.length() < 0.5 {
                continue;
            }
            su += f64::from(s.normalize().dot(dpdu.normalize()));
            tv += f64::from(t.normalize().dot(dpdv.normalize()));
            sv += f64::from(s.normalize().dot(dpdv.normalize()));
            let nrm = v.normals[a];
            handed += if nrm.cross(s).dot(t) > 0.0 { 1 } else { -1 };
            n += 1;
        }
        let n = n.max(1) as f64;
        println!(
            "{path}: S.dPdu {:.3}  T.dPdv {:.3}  S.dPdv {:.3}  sum sign(cross(N,S).T) {handed}",
            su / n,
            tv / n,
            sv / n
        );
    }
}
