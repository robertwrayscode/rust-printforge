//! Sculpting: push, pull, smooth and drag areas of a surface with a soft brush.
//!
//! Works on an object's mesh in its own frame. The mesh is refined (more, smaller
//! triangles) under the brush as needed so strokes stay smooth, and it stays closed:
//! edges are split the same way on both sides and vertices only move.

use crate::mesh::Mesh;
use glam::DVec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Pull,
    Push,
    Smooth,
    Flatten,
    Drag,
}

impl Mode {
    pub const ALL: [(Mode, &'static str, &'static str); 5] = [
        (Mode::Pull, "Pull out", "Raise the surface under the brush, like adding clay."),
        (Mode::Push, "Push in", "Press the surface in, like pressing a thumb into clay."),
        (Mode::Smooth, "Smooth", "Even out bumps and sharp ridges."),
        (Mode::Flatten, "Flatten", "Level the area to a flat patch."),
        (Mode::Drag, "Drag", "Grab a spot and pull it anywhere; the area around follows."),
    ];
}

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub mode: Mode,
    /// mm
    pub radius: f64,
    /// 0..1
    pub strength: f64,
    /// also work on the mirror image across the part's left/right middle
    pub mirror: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { mode: Mode::Pull, radius: 8.0, strength: 0.5, mirror: false }
    }
}

/// Soft falloff: 1 at the centre, 0 at the edge, smooth at both ends.
pub fn falloff(t: f64) -> f64 {
    if t >= 1.0 {
        0.0
    } else {
        let u = 1.0 - t * t;
        u * u
    }
}

/// Triangle budget: refining stops here.
pub const MAX_TRIS: usize = 600_000;

/// Prepare a mesh for sculpting: shared vertices everywhere.
pub fn prepare(m: &Mesh) -> Mesh {
    let mut m = m.clone();
    m.weld(1e-6);
    m.compact();
    m
}

/// Make the triangles under the brush small enough for it.
pub fn refine(m: &Mesh, center: DVec3, radius: f64) -> Mesh {
    let max_edge = (radius / 6.0).max(0.2);
    let reach = radius * 1.25;
    let r2 = reach * reach;
    m.subdivide_where(max_edge, MAX_TRIS, &|a, b| {
        // the edge passes near the brush
        let ab = b - a;
        let t = ((center - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        (a + ab * t).distance_squared(center) < r2
    })
}

fn vertex_normals(m: &Mesh) -> Vec<DVec3> {
    let mut n = vec![DVec3::ZERO; m.verts.len()];
    for t in &m.tris {
        let [a, b, c] = [m.verts[t[0] as usize], m.verts[t[1] as usize], m.verts[t[2] as usize]];
        let f = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += f;
        }
    }
    n.iter().map(|v| v.normalize_or_zero()).collect()
}

fn neighbours(m: &Mesh) -> Vec<Vec<u32>> {
    let mut nb: Vec<Vec<u32>> = vec![vec![]; m.verts.len()];
    for t in &m.tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !nb[a as usize].contains(&b) {
                nb[a as usize].push(b);
            }
            if !nb[b as usize].contains(&a) {
                nb[b as usize].push(a);
            }
        }
    }
    nb
}

/// Average surface direction under the brush.
pub fn area_normal(m: &Mesh, center: DVec3, radius: f64) -> DVec3 {
    let r2 = radius * radius;
    let mut n = DVec3::ZERO;
    for t in &m.tris {
        let [a, b, c] = [m.verts[t[0] as usize], m.verts[t[1] as usize], m.verts[t[2] as usize]];
        let mid = (a + b + c) / 3.0;
        let d2 = mid.distance_squared(center);
        if d2 < r2 {
            n += (b - a).cross(c - a) * falloff((d2 / r2).sqrt());
        }
    }
    n.normalize_or_zero()
}

/// The point mirrored across the part's left/right middle (x = 0 in its own frame).
pub fn mirrored(p: DVec3) -> DVec3 {
    DVec3::new(-p.x, p.y, p.z)
}

/// One dab of the brush at `center` (on the surface). Returns the refined, sculpted mesh.
pub fn dab(m: &Mesh, center: DVec3, brush: &Brush) -> Mesh {
    let mut out = refine(m, center, brush.radius);
    if brush.mirror && center.x.abs() > brush.radius * 0.05 {
        out = refine(&out, mirrored(center), brush.radius);
    }
    let centers: Vec<DVec3> = if brush.mirror && center.x.abs() > brush.radius * 0.05 { vec![center, mirrored(center)] } else { vec![center] };
    for c in centers {
        dab_one(&mut out, c, brush);
    }
    out
}

fn dab_one(m: &mut Mesh, center: DVec3, brush: &Brush) {
    let r = brush.radius;
    let r2 = r * r;
    let s = brush.strength.clamp(0.02, 1.0);
    match brush.mode {
        Mode::Pull | Mode::Push => {
            let n = area_normal(m, center, r);
            if n == DVec3::ZERO {
                return;
            }
            let sign = if brush.mode == Mode::Pull { 1.0 } else { -1.0 };
            let step = sign * s * r * 0.06;
            for v in m.verts.iter_mut() {
                let d2 = v.distance_squared(center);
                if d2 < r2 {
                    *v += n * (step * falloff((d2 / r2).sqrt()));
                }
            }
        }
        Mode::Flatten => {
            let n = area_normal(m, center, r);
            if n == DVec3::ZERO {
                return;
            }
            // plane through the weighted average height of the area
            let (mut sum, mut wsum) = (0.0, 0.0);
            for v in &m.verts {
                let d2 = v.distance_squared(center);
                if d2 < r2 {
                    let w = falloff((d2 / r2).sqrt());
                    sum += (*v - center).dot(n) * w;
                    wsum += w;
                }
            }
            if wsum <= 0.0 {
                return;
            }
            let h = sum / wsum;
            for v in m.verts.iter_mut() {
                let d2 = v.distance_squared(center);
                if d2 < r2 {
                    let off = (*v - center).dot(n) - h;
                    *v -= n * (off * falloff((d2 / r2).sqrt()) * s * 0.6);
                }
            }
        }
        Mode::Smooth => {
            let nb = neighbours(m);
            let old = m.verts.clone();
            let normals = vertex_normals(m);
            for (i, v) in m.verts.iter_mut().enumerate() {
                let d2 = old[i].distance_squared(center);
                if d2 >= r2 || nb[i].is_empty() {
                    continue;
                }
                let avg = nb[i].iter().map(|j| old[*j as usize]).sum::<DVec3>() / nb[i].len() as f64;
                // move along the normal only, so smoothing doesn't shrink or slide the surface
                let n = normals[i];
                let delta = (avg - old[i]).dot(n);
                *v = old[i] + n * (delta * falloff((d2 / r2).sqrt()) * s);
            }
        }
        Mode::Drag => {}
    }
}

/// Drag: move the area around `grab` (taken from `start`, the mesh when the drag began)
/// by `delta`, softly.
pub fn drag(start: &Mesh, grab: DVec3, delta: DVec3, brush: &Brush) -> Mesh {
    let r = brush.radius;
    let r2 = r * r;
    let mut m = start.clone();
    let mut grabs = vec![(grab, delta)];
    if brush.mirror && grab.x.abs() > r * 0.05 {
        grabs.push((mirrored(grab), DVec3::new(-delta.x, delta.y, delta.z)));
    }
    for (g, d) in grabs {
        for (v, o) in m.verts.iter_mut().zip(&start.verts) {
            let d2 = o.distance_squared(g);
            if d2 < r2 {
                *v += d * falloff((d2 / r2).sqrt());
            }
        }
    }
    m
}

/// Refine around a drag point before it starts (so the pulled area has enough detail
/// for the distance it might travel).
pub fn prepare_drag(m: &Mesh, grab: DVec3, brush: &Brush) -> Mesh {
    let mut out = refine(m, grab, brush.radius);
    if brush.mirror && grab.x.abs() > brush.radius * 0.05 {
        out = refine(&out, mirrored(grab), brush.radius);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(m: &Mesh) -> bool {
        let mut e = std::collections::HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                *e.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        e.iter().all(|(k, n)| *n == 1 && e.get(&(k.1, k.0)) == Some(&1))
    }

    fn cube() -> Mesh {
        let s = crate::shapes::Shape::Box { radius: 0.0, steps: 1 };
        prepare(&s.generate(DVec3::splat(20.0)))
    }

    #[test]
    fn pull_raises_and_stays_closed() {
        let m = cube();
        assert!(closed(&m));
        let top = DVec3::new(0.0, 0.0, 20.0);
        let b = Brush { mode: Mode::Pull, radius: 6.0, strength: 1.0, mirror: false };
        let mut out = m.clone();
        for _ in 0..10 {
            out = dab(&out, top, &b);
        }
        assert!(closed(&out));
        let z = out.bounds().max.z;
        assert!(z > 22.0, "top now {z}");
        // the corners didn't move
        assert!(out.verts.iter().any(|v| (v.z - 20.0).abs() < 1e-9 && v.x.abs() > 9.9));
        let vol = out.volume();
        assert!(vol > 8000.0, "{vol}");
    }

    #[test]
    fn push_and_mirror() {
        let m = cube();
        let side = DVec3::new(5.0, -10.0, 10.0);
        let b = Brush { mode: Mode::Push, radius: 4.0, strength: 1.0, mirror: true };
        let mut out = m.clone();
        for _ in 0..8 {
            out = dab(&out, side, &b);
        }
        assert!(closed(&out));
        let dent = |x: f64| out.verts.iter().filter(|v| (v.x - x).abs() < 0.6 && (v.z - 10.0).abs() < 0.6).map(|v| v.y).fold(f64::MIN, f64::max);
        // both the spot and its mirror image are pushed in (front face is y = -10)
        let probe = |x: f64| out.verts.iter().filter(|v| (v.x - x).abs() < 0.6 && (v.z - 10.0).abs() < 0.6 && v.y < 0.0).map(|v| v.y).fold(f64::MAX, f64::min);
        assert!(probe(5.0) > -9.5 && probe(-5.0) > -9.5, "{} {}", probe(5.0), probe(-5.0));
        let _ = dent;
    }

    #[test]
    fn drag_moves_softly() {
        let m = prepare_drag(&cube(), DVec3::new(0.0, 0.0, 20.0), &Brush { mode: Mode::Drag, radius: 8.0, strength: 1.0, mirror: false });
        let out = drag(&m, DVec3::new(0.0, 0.0, 20.0), DVec3::new(0.0, 0.0, 10.0), &Brush { mode: Mode::Drag, radius: 8.0, strength: 1.0, mirror: false });
        assert!(closed(&out));
        assert!((out.bounds().max.z - 30.0).abs() < 1e-6);
    }

    #[test]
    fn smooth_reduces_a_spike() {
        let m = cube();
        let top = DVec3::new(0.0, 0.0, 20.0);
        let mut out = dab(&m, top, &Brush { mode: Mode::Pull, radius: 3.0, strength: 1.0, mirror: false });
        for _ in 0..5 {
            out = dab(&out, top, &Brush { mode: Mode::Pull, radius: 3.0, strength: 1.0, mirror: false });
        }
        let before = out.bounds().max.z;
        for _ in 0..10 {
            out = dab(&out, top, &Brush { mode: Mode::Smooth, radius: 6.0, strength: 1.0, mirror: false });
        }
        assert!(closed(&out));
        assert!(out.bounds().max.z < before - 0.2, "{} -> {}", before, out.bounds().max.z);
    }
}
