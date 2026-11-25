//! Parametric functional parts: containers and lids, brackets, gears, knobs, cable clips,
//! threaded bolts and nuts, print-in-place hinges and snap-fit hooks.
//! Every generator returns a closed mesh sitting on z = 0, centred on X/Y, in mm.

use crate::csg::combine;
use crate::mesh::Mesh;
use crate::shapes::{extrude, orient_outward, revolve, rounded_box};
use glam::{DVec2, DVec3};
use std::f64::consts::{PI, TAU};

fn moved(mut m: Mesh, d: DVec3) -> Mesh {
    m.translate(d);
    m
}

/// Box from min corner to max corner.
fn cube(min: DVec3, max: DVec3) -> Mesh {
    moved(rounded_box(max - min, 0.0, 1), DVec3::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0, min.z))
}

/// Cylinder along Z: centre (x,y), z from z0 to z1.
fn cyl_z(x: f64, y: f64, r: f64, z0: f64, z1: f64, sides: u32) -> Mesh {
    let mut m = revolve(&[(0.0, 0.0), (r, 0.0), (r, z1 - z0), (0.0, z1 - z0)], sides);
    m.weld(1e-9);
    orient_outward(&mut m);
    moved(m, DVec3::new(x, y, z0))
}

/// Cylinder along X: axis at (y, z), x from x0 to x1.
fn cyl_x(y: f64, z: f64, r: f64, x0: f64, x1: f64, sides: u32) -> Mesh {
    let mut m = cyl_z(0.0, 0.0, r, x0, x1, sides);
    for v in &mut m.verts {
        *v = DVec3::new(v.z, v.y + y, -v.x + z);
    }
    orient_outward(&mut m);
    m
}

/// Cylinder along Y: axis at (x, z), y from y0 to y1.
fn cyl_y(x: f64, z: f64, r: f64, y0: f64, y1: f64, sides: u32) -> Mesh {
    let mut m = cyl_z(0.0, 0.0, r, y0, y1, sides);
    for v in &mut m.verts {
        *v = DVec3::new(v.x + x, v.z, -v.y + z);
    }
    orient_outward(&mut m);
    m
}

fn circle(r: f64, n: usize) -> Vec<DVec2> {
    (0..n).map(|i| DVec2::from_angle(TAU * i as f64 / n as f64) * r).collect()
}

fn hexagon(across_flats: f64) -> Vec<DVec2> {
    let r = across_flats / 2.0 / (PI / 6.0).cos();
    (0..6).map(|i| DVec2::from_angle(TAU * i as f64 / 6.0) * r).collect()
}

fn solid(m: Mesh) -> Mesh {
    let mut m = m;
    m.weld(1e-9);
    orient_outward(&mut m);
    m
}

fn union(parts: Vec<Mesh>) -> Mesh {
    combine(&parts, &[]).mesh
}

fn cut(a: Mesh, holes: Vec<Mesh>) -> Mesh {
    combine(&[a], &holes).mesh
}

// ---------------------------------------------------------------- containers

/// Prism with rounded vertical corners (top and bottom stay flat).
fn rrect(w: f64, d: f64, h: f64, r: f64) -> Mesh {
    let r = r.clamp(0.0, w.min(d) / 2.0 - 0.01);
    let mut pts = Vec::new();
    if r <= 0.01 {
        pts = vec![DVec2::new(-w / 2.0, -d / 2.0), DVec2::new(w / 2.0, -d / 2.0), DVec2::new(w / 2.0, d / 2.0), DVec2::new(-w / 2.0, d / 2.0)];
    } else {
        let corners = [(w / 2.0 - r, -d / 2.0 + r, -PI / 2.0), (w / 2.0 - r, d / 2.0 - r, 0.0), (-w / 2.0 + r, d / 2.0 - r, PI / 2.0), (-w / 2.0 + r, -d / 2.0 + r, PI)];
        for (cx, cy, a0) in corners {
            for k in 0..=8 {
                let a = a0 + PI / 2.0 * k as f64 / 8.0;
                pts.push(DVec2::new(cx + a.cos() * r, cy + a.sin() * r));
            }
        }
    }
    solid(extrude(&[vec![pts]], 0.0, h))
}

/// Open-top container with walls `wall` and floor `floor`.
pub fn container(size: DVec3, wall: f64, floor: f64, radius: f64) -> Mesh {
    let w = wall.clamp(0.4, size.x.min(size.y) / 2.5);
    let outer = rrect(size.x, size.y, size.z, radius);
    let inner = moved(rrect(size.x - 2.0 * w, size.y - 2.0 * w, size.z, (radius - w).max(0.0)), DVec3::new(0.0, 0.0, floor.clamp(0.4, size.z - 0.4)));
    cut(outer, vec![inner])
}

/// Matching lid: a plate (printed face down) with a lip that fits inside the container.
/// `size` is the container's outer W × D; H = plate + lip.
pub fn lid(size: DVec3, wall: f64, lip: f64, clearance: f64, radius: f64) -> Mesh {
    let lip = lip.clamp(0.0, size.z - 0.6);
    let plate_t = size.z - lip;
    let plate = rrect(size.x, size.y, plate_t, radius);
    if lip <= 0.01 {
        return plate;
    }
    let ow = size.x - 2.0 * (wall + clearance);
    let od = size.y - 2.0 * (wall + clearance);
    let ring = cut(
        moved(rrect(ow, od, lip + 0.01, (radius - wall).max(0.0)), DVec3::new(0.0, 0.0, plate_t - 0.01)),
        vec![moved(rrect(ow - 2.0 * wall, od - 2.0 * wall, lip + 2.0, (radius - 2.0 * wall).max(0.0)), DVec3::new(0.0, 0.0, plate_t - 1.0))],
    );
    union(vec![plate, ring])
}

// ---------------------------------------------------------------- bracket

/// L-bracket: base W × D, upright W × H, thickness t, two screw holes in each leg.
pub fn bracket(size: DVec3, t: f64, hole_d: f64) -> Mesh {
    let t = t.clamp(1.0, size.y.min(size.z) * 0.8);
    let (hw, hd) = (size.x / 2.0, size.y / 2.0);
    let base = cube(DVec3::new(-hw, -hd, 0.0), DVec3::new(hw, hd, t));
    let up = cube(DVec3::new(-hw, hd - t, 0.0), DVec3::new(hw, hd, size.z));
    // triangular gusset in the middle
    let g = (size.y.min(size.z) - t) * 0.6;
    let gw = t.max(2.0);
    let mut gus = extrude(&[vec![vec![DVec2::new(0.0, 0.0), DVec2::new(g, 0.0), DVec2::new(0.0, g)]]], -gw / 2.0, gw / 2.0);
    for v in &mut gus.verts {
        *v = DVec3::new(v.z, hd - t - v.x + 0.01, t + v.y - 0.01);
    }
    let gus = solid(gus);
    let body = union(vec![base, up, gus]);
    let r = (hole_d / 2.0).max(0.5);
    let xs = [-hw * 0.6, hw * 0.6];
    let mut holes = vec![];
    for x in xs {
        holes.push(cyl_z(x, -hd + (size.y - t) * 0.45, r, -1.0, t + 1.0, 32));
        holes.push(cyl_y(x, t + (size.z - t) * 0.55, r, hd - t - 1.0, hd + 1.0, 32));
    }
    cut(body, holes)
}

// ---------------------------------------------------------------- gear

fn inv(a: f64) -> f64 {
    a.tan() - a
}

/// Involute spur gear outline, outer diameter 1 (scaled later), with a round bore.
pub fn gear_outline(teeth: u32, pressure_deg: f64) -> Vec<DVec2> {
    let n = teeth.clamp(6, 200) as f64;
    let m = 1.0 / (n + 2.0); // module for outer diameter 1
    let rp = n * m / 2.0;
    let ra = rp + m;
    let rf = (rp - 1.25 * m).max(0.05);
    let alpha = pressure_deg.to_radians();
    let rb = rp * alpha.cos();
    let psi = |r: f64| -> f64 {
        let r = r.max(rb);
        let ar = (rb / r).acos();
        PI / (2.0 * n) + inv(alpha) - inv(ar)
    };
    let steps = 8;
    let mut pts = Vec::new();
    for i in 0..teeth.clamp(6, 200) {
        let c = TAU * i as f64 / n;
        let r_start = rf.max(rb * 0.999);
        let flank: Vec<f64> = (0..=steps).map(|k| r_start + (ra - r_start) * k as f64 / steps as f64).collect();
        // root on the left side of this tooth
        pts.push(DVec2::from_angle(c - psi(rb) - 0.5 * (TAU / n - 2.0 * psi(rb)) * 0.5) * rf);
        pts.push(DVec2::from_angle(c - psi(rb)) * rf);
        for &r in &flank {
            pts.push(DVec2::from_angle(c - psi(r)) * r);
        }
        for &r in flank.iter().rev() {
            pts.push(DVec2::from_angle(c + psi(r)) * r);
        }
        pts.push(DVec2::from_angle(c + psi(rb)) * rf);
        pts.push(DVec2::from_angle(c + psi(rb) + 0.5 * (TAU / n - 2.0 * psi(rb)) * 0.5) * rf);
    }
    pts.dedup_by(|a, b| a.distance(*b) < 1e-7);
    pts
}

pub fn gear(size: DVec3, teeth: u32, bore: f64) -> Mesh {
    let d = size.x.min(size.y);
    let outline: Vec<DVec2> = gear_outline(teeth, 20.0).into_iter().map(|p| p * d).collect();
    let mut groups = vec![outline];
    if bore > 0.0 && bore < d * 0.6 {
        groups.push(circle(bore / 2.0, 48));
    }
    let mut m = extrude(&[groups], 0.0, size.z);
    m.weld(1e-9);
    orient_outward(&mut m);
    crate::scene::fit_to_size(&m, size)
}

// ---------------------------------------------------------------- knob

pub fn knob(size: DVec3, grips: u32, bore: f64) -> Mesh {
    let r = size.x.min(size.y) / 2.0;
    let n = grips.clamp(3, 60) as f64;
    let outline: Vec<DVec2> = (0..360)
        .map(|i| {
            let a = TAU * i as f64 / 360.0;
            let k = ((1.0 + (n * a).cos()) / 2.0).powi(3);
            DVec2::from_angle(a) * r * (1.0 - 0.09 * k)
        })
        .collect();
    let body = solid(extrude(&[vec![outline]], 0.0, size.z * 0.8));
    // domed cap
    let cap_prof: Vec<(f64, f64)> = std::iter::once((0.0, 0.0))
        .chain((0..=12).map(|i| {
            let a = 0.5 * PI * i as f64 / 12.0;
            (r * 0.9 * a.cos(), size.z * 0.2 * a.sin() + 0.0)
        }))
        .collect();
    let mut cap = revolve(&cap_prof, 64);
    cap.weld(1e-9);
    orient_outward(&mut cap);
    let cap = moved(cap, DVec3::new(0.0, 0.0, size.z * 0.8 - 0.01));
    let k = union(vec![body, cap]);
    if bore <= 0.0 {
        return k;
    }
    // D-shaped shaft hole from the bottom, 70 % deep
    let br = bore / 2.0;
    let d_shape: Vec<DVec2> = circle(br, 48).into_iter().map(|p| DVec2::new(p.x.min(br * 0.62), p.y)).collect();
    let hole = solid(extrude(&[vec![d_shape]], -1.0, size.z * 0.7));
    cut(k, vec![hole])
}

// ---------------------------------------------------------------- cable clip

/// Natural size of a cable clip for a cable diameter.
pub fn cable_clip_size(cable_d: f64) -> DVec3 {
    let wall = 1.8;
    DVec3::new(10.0, cable_d + 2.0 * wall + 14.0, cable_d + 2.0 * wall + 2.0)
}

pub fn cable_clip(size: DVec3, cable_d: f64) -> Mesh {
    let nat = cable_clip_size(cable_d);
    let wall = 1.8;
    let r_in = cable_d / 2.0;
    let r_out = r_in + wall;
    let tab_t = 2.0;
    let cz = tab_t + r_in; // cable centre height
    // ring with a 70° opening at the top (profile in Y/Z, extruded along X)
    let open = 35f64.to_radians();
    let n = 48;
    let mut ring = Vec::new();
    for i in 0..=n {
        let a = PI / 2.0 + open + (TAU - 2.0 * open) * i as f64 / n as f64;
        ring.push(DVec2::new(a.cos() * r_out, cz + a.sin() * r_out));
    }
    for i in (0..=n).rev() {
        let a = PI / 2.0 + open + (TAU - 2.0 * open) * i as f64 / n as f64;
        ring.push(DVec2::new(a.cos() * r_in, cz + a.sin() * r_in));
    }
    let tab = vec![
        DVec2::new(-nat.y / 2.0, 0.0),
        DVec2::new(nat.y / 2.0, 0.0),
        DVec2::new(nat.y / 2.0, tab_t),
        DVec2::new(-nat.y / 2.0, tab_t),
    ];
    let to3 = |m: Mesh| {
        let mut m = m;
        for v in &mut m.verts {
            *v = DVec3::new(v.z, v.x, v.y);
        }
        solid(m)
    };
    let hw = nat.x / 2.0;
    let ring_m = to3(extrude(&[vec![ring]], -hw, hw));
    let tab_m = to3(extrude(&[vec![tab]], -hw, hw));
    let body = union(vec![ring_m, tab_m]);
    let hole = cyl_z(0.0, -nat.y / 2.0 + 3.5, 1.7, -1.0, tab_t + 1.0, 24);
    let hole2 = cyl_z(0.0, nat.y / 2.0 - 3.5, 1.7, -1.0, tab_t + 1.0, 24);
    let mut m = cut(body, vec![hole, hole2]);
    let b = m.bounds();
    m.translate(DVec3::new(0.0, 0.0, -b.min.z));
    crate::scene::fit_to_size(&m, size)
}

// ---------------------------------------------------------------- threads

/// External ISO-like thread on a rod: major diameter d, pitch p, length len (z from 0).
/// `internal` widens it slightly for use as a nut cutter.
fn threaded_rod(d: f64, p: f64, len: f64, chamfer_bottom: bool, chamfer_top: bool) -> Mesh {
    let r_maj = d / 2.0;
    let depth = 0.54 * p;
    let n = 64usize;
    let dz = p / 12.0;
    let rows = ((len / dz).ceil() as usize).max(2);
    let mut m = Mesh::new();
    let mut grid = vec![vec![0u32; n]; rows + 1];
    for j in 0..=rows {
        let z = len * j as f64 / rows as f64;
        for i in 0..n {
            let a = TAU * i as f64 / n as f64;
            let ph = (z / p - a / TAU).rem_euclid(1.0);
            // trapezoid-ish profile: crest flat 1/8, root flat 1/4
            let prof = if ph < 0.125 {
                1.0
            } else if ph < 0.4375 {
                1.0 - (ph - 0.125) / 0.3125
            } else if ph < 0.6875 {
                0.0
            } else {
                (ph - 0.6875) / 0.3125
            };
            let mut r = r_maj - depth + depth * prof;
            let lim = r_maj - depth;
            if chamfer_bottom {
                r = r.min(lim + z * 0.9);
            }
            if chamfer_top {
                r = r.min(lim + (len - z) * 0.9);
            }
            grid[j][i] = m.add_vert(DVec3::new(a.cos() * r, a.sin() * r, z));
        }
    }
    for j in 0..rows {
        for i in 0..n {
            let i2 = (i + 1) % n;
            m.add_quad(grid[j][i], grid[j][i2], grid[j + 1][i2], grid[j + 1][i]);
        }
    }
    let b = m.add_vert(DVec3::ZERO);
    let t = m.add_vert(DVec3::new(0.0, 0.0, len));
    for i in 0..n {
        let i2 = (i + 1) % n;
        m.add_tri(b, grid[0][i2], grid[0][i]);
        m.add_tri(t, grid[rows][i], grid[rows][i2]);
    }
    solid(m)
}

/// Metric-style coarse pitch for a diameter.
pub fn coarse_pitch(d: f64) -> f64 {
    let table = [(3.0, 0.5), (4.0, 0.7), (5.0, 0.8), (6.0, 1.0), (8.0, 1.25), (10.0, 1.5), (12.0, 1.75), (16.0, 2.0), (20.0, 2.5)];
    table.iter().rev().find(|(dd, _)| d >= *dd - 0.01).map(|x| x.1).unwrap_or(0.5)
}

pub fn bolt_size(d: f64, len: f64) -> DVec3 {
    let af = 1.7 * d;
    let corner = af / (PI / 6.0).cos();
    DVec3::new(corner, af, len + 0.7 * d)
}

/// Hex-head bolt, printed head down.
pub fn bolt(d: f64, len: f64, pitch: f64) -> Mesh {
    let hh = 0.7 * d;
    let head = solid(extrude(&[vec![hexagon(1.7 * d)]], 0.0, hh));
    let rod = moved(threaded_rod(d, pitch, len, false, true), DVec3::new(0.0, 0.0, hh - 0.01));
    union(vec![head, rod])
}

pub fn nut_size(d: f64) -> DVec3 {
    let af = 1.7 * d;
    DVec3::new(af / (PI / 6.0).cos(), af, 0.8 * d)
}

/// Hex nut with an internal thread (clearance added for printing).
pub fn nut(d: f64, pitch: f64, clearance: f64) -> Mesh {
    let h = 0.8 * d;
    let body = solid(extrude(&[vec![hexagon(1.7 * d)]], 0.0, h));
    let cutter = moved(threaded_rod(d + 2.0 * clearance, pitch, h + 2.0 * pitch, false, false), DVec3::new(0.0, 0.0, -pitch));
    // countersinks at both faces: no knife-edge slivers, and the bolt starts easily
    let r_out = d / 2.0 + clearance + 0.4;
    let r_in = d / 2.0 + clearance - 0.54 * pitch - 0.2;
    let depth = (r_out - r_in).max(0.3);
    let mut sink = revolve(&[(0.0, -0.5), (r_out + 0.5, -0.5), (r_in, depth), (0.0, depth)], 64);
    sink.weld(1e-9);
    orient_outward(&mut sink);
    let mut sink_top = sink.clone();
    for v in &mut sink_top.verts {
        v.z = h - v.z;
    }
    orient_outward(&mut sink_top);
    cut(body, vec![cutter, sink, sink_top])
}

// ---------------------------------------------------------------- hinge

/// Print-in-place hinge: two leaves joined by alternating knuckles around a captive pin.
/// size: W = hinge length (along the pin), D = open width (both leaves), H = knuckle diameter.
pub fn hinge(size: DVec3, knuckles: u32, clearance: f64) -> Mesh {
    let k = (knuckles.max(3) | 1) as usize; // odd
    let r = size.z / 2.0;
    let c = clearance.clamp(0.15, 1.0);
    let pin_r = (r * 0.45).max(0.8);
    let w = size.x;
    let seg = (w - (k as f64 - 1.0) * c) / k as f64;
    let leaf_t = (size.z * 0.4).clamp(1.2, 3.0);
    let half_d = size.y / 2.0;
    let seg_x = |i: usize| -> (f64, f64) {
        let x0 = -w / 2.0 + i as f64 * (seg + c);
        (x0, x0 + seg)
    };
    let axis_z = r;
    let mut a_parts = vec![cube(DVec3::new(-w / 2.0, -half_d, 0.0), DVec3::new(w / 2.0, -r - c, leaf_t))];
    let mut b_parts = vec![cube(DVec3::new(-w / 2.0, r + c, 0.0), DVec3::new(w / 2.0, half_d, leaf_t))];
    for i in 0..k {
        let (x0, x1) = seg_x(i);
        let knuckle = cyl_x(0.0, axis_z, r, x0, x1, 48);
        // bridge from leaf to knuckle
        if i % 2 == 0 {
            a_parts.push(knuckle);
            a_parts.push(cube(DVec3::new(x0, -r - c - 0.01, 0.0), DVec3::new(x1, 0.0, leaf_t.min(axis_z))));
        } else {
            b_parts.push(knuckle);
            b_parts.push(cube(DVec3::new(x0, 0.0, 0.0), DVec3::new(x1, r + c + 0.01, leaf_t.min(axis_z))));
        }
    }
    a_parts.push(cyl_x(0.0, axis_z, pin_r, -w / 2.0 + 0.01, w / 2.0 - 0.01, 32));
    let a = union(a_parts);
    let b_body = union(b_parts);
    let b = cut(b_body, vec![cyl_x(0.0, axis_z, pin_r + c, -w / 2.0 - 1.0, w / 2.0 + 1.0, 32)]);
    // screw holes
    let hr = 1.7;
    let holes_a = vec![cyl_z(-w * 0.3, -half_d + (half_d - r) * 0.5, hr, -1.0, leaf_t + 1.0, 24), cyl_z(w * 0.3, -half_d + (half_d - r) * 0.5, hr, -1.0, leaf_t + 1.0, 24)];
    let holes_b = vec![cyl_z(-w * 0.3, half_d - (half_d - r) * 0.5, hr, -1.0, leaf_t + 1.0, 24), cyl_z(w * 0.3, half_d - (half_d - r) * 0.5, hr, -1.0, leaf_t + 1.0, 24)];
    let mut out = cut(a, holes_a);
    out.append(&cut(b, holes_b));
    out
}

// ---------------------------------------------------------------- snap-fit

/// Cantilever snap-fit hook (profile in X/Z, extruded along Y = width).
pub fn snap_hook(size: DVec3, arm_t: f64, catch: f64) -> Mesh {
    let (w, h) = (size.x, size.z);
    let t = arm_t.clamp(0.6, w * 0.5);
    let base_h = (h * 0.2).max(1.5);
    let catch = catch.clamp(0.3, w - t);
    let hook_h = (catch * 2.5).min(h * 0.3);
    let p = vec![
        DVec2::new(-w / 2.0, 0.0),
        DVec2::new(w / 2.0, 0.0),
        DVec2::new(w / 2.0, base_h),
        DVec2::new(-w / 2.0 + t, base_h),
        DVec2::new(-w / 2.0 + t, h - hook_h),
        DVec2::new(-w / 2.0 + t + catch, h - hook_h),
        DVec2::new(-w / 2.0 + t, h),
        DVec2::new(-w / 2.0, h),
    ];
    let mut m = extrude(&[vec![p]], -size.y / 2.0, size.y / 2.0);
    for v in &mut m.verts {
        *v = DVec3::new(v.x, v.z, v.y);
    }
    solid(m)
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
        e.iter().all(|(&(a, b), &n)| n == 1 && e.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn parts_are_closed_solids() {
        let cases: Vec<(&str, Mesh)> = vec![
            ("container", container(DVec3::new(60.0, 40.0, 30.0), 2.0, 2.0, 4.0)),
            ("lid", lid(DVec3::new(60.0, 40.0, 6.0), 2.0, 4.0, 0.25, 4.0)),
            ("bracket", bracket(DVec3::new(30.0, 25.0, 25.0), 3.0, 4.0)),
            ("gear", gear(DVec3::new(40.0, 40.0, 6.0), 20, 5.0)),
            ("knob", knob(DVec3::new(30.0, 30.0, 18.0), 12, 6.0)),
            ("clip", cable_clip(cable_clip_size(6.0), 6.0)),
            ("bolt", bolt(8.0, 20.0, 1.25)),
            ("nut", nut(8.0, 1.25, 0.25)),
            ("hinge", hinge(DVec3::new(40.0, 30.0, 6.0), 5, 0.3)),
            ("snap", snap_hook(DVec3::new(6.0, 8.0, 15.0), 1.4, 1.0)),
        ];
        for (name, m) in cases {
            assert!(!m.tris.is_empty(), "{name} empty");
            assert!(closed(&m), "{name} not watertight");
            assert!(m.volume() > 0.0, "{name} volume {}", m.volume());
        }
    }

    #[test]
    fn hinge_has_two_separate_halves() {
        let h = hinge(DVec3::new(40.0, 30.0, 6.0), 5, 0.3);
        let (_, n) = h.components();
        assert_eq!(n, 2, "print-in-place hinge must be two separate pieces");
    }

    #[test]
    fn nut_fits_bolt() {
        // nut's hole must be bigger than the bolt's major diameter
        let n = nut(8.0, 1.25, 0.25);
        let b = n.bounds();
        assert!(b.size().z > 6.0);
        let inner_min = n
            .verts
            .iter()
            .filter(|v| v.z > 1.0 && v.z < 5.0)
            .map(|v| (v.x * v.x + v.y * v.y).sqrt())
            .fold(f64::INFINITY, f64::min);
        assert!(inner_min > 8.0 / 2.0 - 0.54 * 1.25 + 0.2, "inner radius {inner_min}");
    }
}
