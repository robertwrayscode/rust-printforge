//! Robust mesh booleans (via boolmesh, a Manifold port) that keep per-triangle colours.

use crate::mesh::{Mesh, NO_COLOR};
use boolmesh::prelude::*;
use glam::DVec3;

/// Colour value used for faces coming from holes; recoloured afterwards.
const HOLE_TAG: f64 = 1000.0;

fn to_manifold(m: &Mesh, tag_override: Option<f64>) -> Result<Manifold<f64>, String> {
    let pos: Vec<f64> = m.verts.iter().flat_map(|v| [v.x, v.y, v.z]).collect();
    let idx: Vec<usize> = m.tris.iter().flat_map(|t| t.iter().map(|&i| i as usize)).collect();
    let vals: Vec<f64> = m
        .colors
        .iter()
        .flat_map(|&c| {
            let v = tag_override.unwrap_or(c as f64);
            [v, v, v]
        })
        .collect();
    Manifold::new_with_values(&pos, &idx, &vals)
}

fn from_manifold(r: &Manifold<f64>) -> Mesh {
    let mut m = Mesh::new();
    m.verts = r.ps.iter().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
    let n = r.hs.len() / 3;
    for f in 0..n {
        let t = [r.hs[3 * f].tail as u32, r.hs[3 * f + 1].tail as u32, r.hs[3 * f + 2].tail as u32];
        m.tris.push(t);
        let v = r.vals.get(3 * f).copied().unwrap_or(NO_COLOR as f64);
        m.colors.push(if v >= HOLE_TAG - 0.5 { 254 } else { v.round().clamp(0.0, 255.0) as u8 });
    }
    m.compact();
    m
}

#[derive(Debug)]
pub struct CsgResult {
    pub mesh: Mesh,
    pub warnings: Vec<String>,
}

/// Union of `solids` minus union of `holes`. Faces cut by holes take the colour of
/// the neighbouring solid surface. Falls back to plain merging if an input isn't
/// a closed solid (and reports it).
pub fn combine(solids: &[Mesh], holes: &[Mesh]) -> CsgResult {
    let mut warnings = Vec::new();
    let mut acc: Option<Manifold<f64>> = None;
    let mut loose = Mesh::new();
    for (i, s) in solids.iter().enumerate() {
        if s.tris.is_empty() {
            continue;
        }
        match to_manifold(s, None) {
            Ok(mf) => {
                acc = Some(match acc {
                    None => mf,
                    Some(a) => match compute_boolean(&a, &mf, OpType::Add) {
                        Ok(r) => r,
                        Err(e) => {
                            warnings.push(format!("union failed for part {}: {e}", i + 1));
                            loose.append(s);
                            a
                        }
                    },
                })
            }
            Err(e) => {
                warnings.push(format!("part {} is not a closed solid ({e}); merged without boolean", i + 1));
                loose.append(s);
            }
        }
    }
    let Some(mut acc) = acc else {
        return CsgResult { mesh: loose, warnings };
    };
    for (i, h) in holes.iter().enumerate() {
        if h.tris.is_empty() {
            continue;
        }
        match to_manifold(h, Some(HOLE_TAG)) {
            Ok(hm) => match compute_boolean(&acc, &hm, OpType::Subtract) {
                Ok(r) => acc = r,
                Err(e) => warnings.push(format!("hole {} failed: {e}", i + 1)),
            },
            Err(e) => warnings.push(format!("hole {} is not a closed solid ({e}); ignored", i + 1)),
        }
    }
    let mut out = from_manifold(&acc);
    recolor_hole_faces(&mut out);
    out.append(&loose);
    CsgResult { mesh: out, warnings }
}

/// Faces that came from hole cutters (tag 254) adopt the colour of adjacent solid faces.
fn recolor_hole_faces(m: &mut Mesh) {
    if !m.colors.contains(&254) {
        return;
    }
    let nb = m.tri_neighbors();
    for _ in 0..64 {
        let mut changed = false;
        let mut pending = false;
        for t in 0..m.tris.len() {
            if m.colors[t] != 254 {
                continue;
            }
            pending = true;
            for &n in &nb[t] {
                if n != u32::MAX && m.colors[n as usize] != 254 {
                    m.colors[t] = m.colors[n as usize];
                    changed = true;
                    break;
                }
            }
        }
        if !pending || !changed {
            break;
        }
    }
    for c in &mut m.colors {
        if *c == 254 {
            *c = NO_COLOR;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    #[test]
    fn box_minus_cylinder_keeps_colors() {
        let mut a = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        a.set_color(2);
        let mut h = Shape::Cylinder { sides: 32, bevel: 0.0 }.generate(DVec3::new(8.0, 8.0, 30.0));
        h.translate(DVec3::new(0.0, 0.0, -5.0));
        let r = combine(&[a], &[h]);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let v = r.mesh.volume();
        let expect = 8000.0 - std::f64::consts::PI * 16.0 * 20.0;
        assert!((v - expect).abs() < 15.0, "vol {v} vs {expect}");
        assert!(r.mesh.colors.iter().all(|&c| c == 2));
    }

    #[test]
    fn two_color_union() {
        let mut a = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        a.set_color(0);
        let mut b = Shape::Sphere { steps: 16 }.generate(DVec3::splat(15.0));
        b.translate(DVec3::new(10.0, 0.0, 10.0));
        b.set_color(3);
        let r = combine(&[a, b], &[]);
        assert!(r.warnings.is_empty());
        let used = r.mesh.used_colors();
        assert_eq!(used, vec![0, 3]);
    }
}
