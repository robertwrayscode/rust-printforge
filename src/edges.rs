//! Round or bevel (chamfer) the sharp outside edges of any solid.
//!
//! Every sharp convex edge gets a "cutter": the sliver between the edge and the rounded
//! (or bevelled) surface, extruded along the edge. Corners where three roughly square edges
//! meet get a matching corner cutter, so a box ends up with properly rounded corners.
//! All cutters are subtracted in one CSG step, so the result stays a clean solid.

use crate::mesh::Mesh;
use crate::shapes::{extrude, orient_outward};
use glam::{DVec2, DVec3};
use std::collections::HashMap;

pub struct EdgeOptions {
    pub radius: f64,
    pub chamfer: bool,
    /// leave the edges that sit on the print plate sharp (rounded bottoms print badly)
    pub skip_bottom: bool,
    /// edges sharper than this (degrees between faces) are treated
    pub min_angle: f64,
}

struct Edge {
    a: DVec3,
    b: DVec3,
    n1: DVec3,
    n2: DVec3,
    d1: DVec3,
    d2: DVec3,
}

fn sharp_convex_edges(m: &Mesh, o: &EdgeOptions) -> Vec<Edge> {
    let nb = m.tri_neighbors();
    let normals: Vec<DVec3> = (0..m.tris.len()).map(|t| m.tri_normal_area(t).0).collect();
    let cent = |t: usize| {
        let p = m.tri_points(t);
        (p[0] + p[1] + p[2]) / 3.0
    };
    let zmin = m.bounds().min.z;
    let cos_lim = o.min_angle.to_radians().cos();
    let mut out = vec![];
    for (i, tri) in m.tris.iter().enumerate() {
        for k in 0..3 {
            let j = nb[i][k];
            if j == u32::MAX || (j as usize) <= i {
                continue;
            }
            let j = j as usize;
            let (n1, n2) = (normals[i], normals[j]);
            if n1.dot(n2) > cos_lim {
                continue;
            }
            let (a, b) = (m.verts[tri[k] as usize], m.verts[tri[(k + 1) % 3] as usize]);
            if a.distance(b) < 1e-6 {
                continue;
            }
            // convex: the other face lies below this face's plane
            if (cent(j) - a).dot(n1) > -1e-7 {
                continue;
            }
            if o.skip_bottom && a.z < zmin + 0.01 && b.z < zmin + 0.01 {
                continue;
            }
            let e = (b - a).normalize();
            let mid = (a + b) / 2.0;
            let mut d1 = n1.cross(e).normalize();
            if (cent(i) - mid).dot(d1) < 0.0 {
                d1 = -d1;
            }
            let mut d2 = n2.cross(e).normalize();
            if (cent(j) - mid).dot(d2) < 0.0 {
                d2 = -d2;
            }
            out.push(Edge { a, b, n1, n2, d1, d2 });
        }
    }
    out
}

/// Extrude a 2D polygon given in the (u, v) basis at `origin`, from s0 to s1 along w.
fn prism(poly: &[DVec2], origin: DVec3, u: DVec3, v: DVec3, w: DVec3, s0: f64, s1: f64) -> Mesh {
    let mut m = extrude(&[vec![poly.to_vec()]], s0, s1);
    for p in &mut m.verts {
        *p = origin + u * p.x + v * p.y + w * p.z;
    }
    m.weld(1e-9);
    orient_outward(&mut m);
    m
}

fn edge_cutter(e: &Edge, o: &EdgeOptions) -> Option<Mesh> {
    let phi = e.d1.dot(e.d2).clamp(-1.0, 1.0).acos(); // inside angle between the faces
    if phi < 10f64.to_radians() || phi > 170f64.to_radians() {
        return None;
    }
    let r = o.radius;
    let t = if o.chamfer { r } else { r / (phi / 2.0).tan() };
    let eps = 0.02;
    let u = e.d1;
    let v = (e.d2 - u * u.dot(e.d2)).normalize();
    let w = u.cross(v);
    let to2 = |p: DVec3| DVec2::new(p.dot(u), p.dot(v));
    let t1 = e.d1 * t;
    let t2 = e.d2 * t;
    let out = -(e.d1 + e.d2).normalize() * (t + 1.0) + (e.n1 + e.n2) * eps;
    let mut poly = vec![to2(t1 + e.n1 * eps)];
    if o.chamfer {
        poly.push(to2(t1));
        poly.push(to2(t2));
    } else {
        let bis = (e.d1 + e.d2).normalize();
        let c = bis * (r / (phi / 2.0).sin());
        let (a1, a2) = (t1 - c, t2 - c);
        let steps = 10;
        // sweep from a1 to a2 the short way round (the side facing the edge)
        let axis = a1.cross(a2).normalize();
        let ang = a1.angle_between(a2);
        for s in 0..=steps {
            let q = glam::DQuat::from_axis_angle(axis, ang * s as f64 / steps as f64);
            poly.push(to2(c + q * a1));
        }
    }
    poly.push(to2(t2 + e.n2 * eps));
    poly.push(to2(out));
    // make sure the polygon winds counter-clockwise
    let area: f64 = (0..poly.len()).map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()])).sum();
    if area < 0.0 {
        poly.reverse();
    }
    let (s0, s1) = ((e.a).dot(w), (e.b).dot(w));
    let (lo, hi) = (s0.min(s1) - 0.01, s0.max(s1) + 0.01);
    // origin: the edge line itself (any point on it, projected to w = 0)
    let origin = e.a - w * e.a.dot(w);
    Some(prism(&poly, origin, u, v, w, lo, hi))
}

/// Corner piece for three roughly square rounded edges meeting at a point.
fn corner_cutter(vtx: DVec3, dirs: [DVec3; 3], r: f64) -> Mesh {
    // box spanned by the three edge directions, minus a ball tucked into the corner
    let (a, b, c) = (dirs[0] * r, dirs[1] * r, dirs[2] * r);
    let out = -(dirs[0] + dirs[1] + dirs[2]).normalize() * 0.05;
    let pts = [vtx + out, vtx + a, vtx + b, vtx + c, vtx + a + b, vtx + a + c, vtx + b + c, vtx + a + b + c];
    let mut bx = Mesh::new();
    let ids: Vec<u32> = pts.iter().map(|p| bx.add_vert(*p)).collect();
    // faces of the (slightly skewed) parallelepiped
    let f = [[0, 2, 4, 1], [0, 1, 5, 3], [0, 3, 6, 2], [7, 5, 1, 4], [7, 4, 2, 6], [7, 6, 3, 5]];
    for q in f {
        bx.add_quad(ids[q[0]], ids[q[1]], ids[q[2]], ids[q[3]]);
    }
    orient_outward(&mut bx);
    let mut ball = crate::shapes::Shape::Sphere { steps: 24 }.generate(DVec3::splat(2.0 * r));
    let centre = vtx + a + b + c;
    for p in &mut ball.verts {
        *p = *p + centre - DVec3::new(0.0, 0.0, r);
    }
    let res = crate::csg::combine(&[bx], &[ball]);
    res.mesh
}

pub fn round_edges(m: &Mesh, o: &EdgeOptions) -> Result<Mesh, String> {
    if o.radius <= 0.0 {
        return Err("Pick a radius above 0.".into());
    }
    let mut src = m.clone();
    src.weld(1e-7);
    let edges = sharp_convex_edges(&src, o);
    if edges.is_empty() {
        return Err("No sharp outside edges found to round.".into());
    }
    if edges.len() > 3000 {
        return Err(format!("This part has {} sharp edges — too many to round in one go. Try it on a simpler part.", edges.len()));
    }
    let mut cutters: Vec<Mesh> = edges.iter().filter_map(|e| edge_cutter(e, o)).collect();
    if !o.chamfer {
        // vertices shared by exactly three square-ish rounded edges
        let key = |p: DVec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
        let mut at: HashMap<(i64, i64, i64), Vec<DVec3>> = HashMap::new();
        for e in &edges {
            at.entry(key(e.a)).or_default().push((e.b - e.a).normalize());
            at.entry(key(e.b)).or_default().push((e.a - e.b).normalize());
        }
        for e in &edges {
            for (p, _) in [(e.a, 0), (e.b, 1)] {
                if let Some(d) = at.get(&key(p)) {
                    if d.len() == 3 && d[0].dot(d[1]).abs() < 0.2 && d[0].dot(d[2]).abs() < 0.2 && d[1].dot(d[2]).abs() < 0.2 {
                        cutters.push(corner_cutter(p, [d[0], d[1], d[2]], o.radius));
                        at.remove(&key(p));
                    }
                }
            }
        }
    }
    let res = crate::csg::combine(&[src.clone()], &cutters);
    if res.mesh.tris.is_empty() {
        return Err("Rounding failed — try a smaller radius.".into());
    }
    let vol0 = src.volume().abs();
    let vol1 = res.mesh.volume().abs();
    if vol1 < vol0 * 0.3 {
        return Err("That radius is too big for this part (it would remove most of it). Try a smaller one.".into());
    }
    Ok(res.mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    fn closed(m: &Mesh) -> bool {
        let mut e = std::collections::HashSet::new();
        for t in &m.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        e.iter().all(|&(a, b)| e.contains(&(b, a)))
    }

    #[test]
    fn rounded_and_chamfered_box() {
        let mut b = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        b.fill_unset_color(2);
        let o = EdgeOptions { radius: 3.0, chamfer: false, skip_bottom: false, min_angle: 30.0 };
        let r = round_edges(&b, &o).unwrap();
        assert!(closed(&r));
        // exact rounded cube volume: 20³ − (12 edges · (4−π)/4 · r² · (20−2r)) − 8 corners · (r³ − π r³/6)
        let rr = 3.0f64;
        let edge = 12.0 * (1.0 - std::f64::consts::PI / 4.0) * rr * rr * (20.0 - 2.0 * rr);
        let corner = 8.0 * (rr * rr * rr - std::f64::consts::PI * rr * rr * rr / 6.0);
        let expect = 8000.0 - edge - corner;
        let v = r.volume();
        assert!((v - expect).abs() / expect < 0.01, "{v} vs {expect}");
        assert!(r.colors.iter().all(|&c| c == 2), "colour kept");
        // size unchanged
        let bb = r.bounds();
        assert!((bb.max - bb.min - DVec3::splat(20.0)).length() < 1e-3);
        let c = round_edges(&b, &EdgeOptions { radius: 2.0, chamfer: true, skip_bottom: true, min_angle: 30.0 }).unwrap();
        assert!(closed(&c));
        // 8 chamfered edges (bottom 4 kept): each removes 2²/2 · ~20
        assert!(c.volume() < 8000.0 - 8.0 * 2.0 * 16.0 && c.volume() > 8000.0 - 8.0 * 2.0 * 21.0, "{}", c.volume());
    }

    #[test]
    fn cylinder_rim_rounds() {
        let cy = Shape::Cylinder { sides: 48, bevel: 0.0 }.generate(DVec3::new(30.0, 30.0, 10.0));
        let r = round_edges(&cy, &EdgeOptions { radius: 2.0, chamfer: false, skip_bottom: true, min_angle: 30.0 }).unwrap();
        assert!(closed(&r));
        assert!(r.volume() < cy.volume() && r.volume() > cy.volume() * 0.9);
    }
}

#[cfg(test)]
mod more {
    use super::*;
    use crate::shapes::Shape;
    #[test]
    fn container_and_text_dont_break() {
        let t0 = std::time::Instant::now();
        let c = Shape::Container { wall: 2.0, floor: 2.0, radius: 0.0 }.generate(DVec3::new(60.0, 40.0, 30.0));
        let r = round_edges(&c, &EdgeOptions { radius: 0.8, chamfer: false, skip_bottom: true, min_angle: 30.0 }).unwrap();
        assert!(r.volume() < c.volume() && r.volume() > c.volume() * 0.9);
        let t = Shape::Text { text: "RF".into(), font: crate::shapes::FontChoice::Sans, spacing: 0.0 }.generate(DVec3::new(30.0, 15.0, 3.0));
        let r2 = round_edges(&t, &EdgeOptions { radius: 0.5, chamfer: true, skip_bottom: true, min_angle: 30.0 }).unwrap();
        assert!(r2.volume() < t.volume());
        eprintln!("took {:?}", t0.elapsed());
    }
}
