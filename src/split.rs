//! Split a part that's too big for the plate (or that prints better in pieces) along a
//! plane, with holes for separate alignment pins so the pieces glue together straight.

use crate::mesh::{Bvh, Mesh};
use glam::DVec3;

pub struct SplitOptions {
    /// 0 = X, 1 = Y, 2 = Z
    pub axis: usize,
    /// cut position along the axis (world mm)
    pub at: f64,
    pub pins: bool,
    pub pin_d: f64,
    pub pin_len: f64,
    /// extra room around the pins (each side)
    pub gap: f64,
}

pub struct SplitResult {
    pub low: Mesh,
    pub high: Mesh,
    pub pin_spots: Vec<DVec3>,
}

fn axis_vec(a: usize) -> DVec3 {
    [DVec3::X, DVec3::Y, DVec3::Z][a.min(2)]
}

fn inside(m: &Mesh, bvh: &Bvh, p: DVec3, dir: DVec3) -> bool {
    let mut o = p;
    let mut n = 0;
    for _ in 0..64 {
        match bvh.raycast(m, o, dir, None) {
            Some((t, _)) => {
                n += 1;
                o = o + dir * (t + 1e-5);
            }
            None => break,
        }
    }
    n % 2 == 1
}

fn dist_to_surface(m: &Mesh, bvh: &Bvh, p: DVec3, max: f64) -> f64 {
    let b = crate::mesh::Aabb { min: p - DVec3::splat(max), max: p + DVec3::splat(max) };
    let mut best = max;
    let mut found = vec![];
    bvh.query_box(&b, &mut found);
    for t in found {
        let [a, bb, c] = m.tri_points(t as usize);
        let q = crate::mesh::closest_on_tri(p, a, bb, c);
        best = best.min(q.distance(p));
    }
    best
}

/// Where alignment pins fit on the cut: up to `want` spots, well inside the material.
pub fn pin_spots(m: &Mesh, axis: usize, at: f64, pin_d: f64, want: usize) -> Vec<DVec3> {
    let n = axis_vec(axis);
    let (u, v) = match axis {
        0 => (DVec3::Y, DVec3::Z),
        1 => (DVec3::X, DVec3::Z),
        _ => (DVec3::X, DVec3::Y),
    };
    let bvh = Bvh::build(m);
    let b = m.bounds();
    let (u0, u1) = (b.min.dot(u), b.max.dot(u));
    let (v0, v1) = (b.min.dot(v), b.max.dot(v));
    let margin = pin_d / 2.0 + 2.0;
    let step = ((u1 - u0).max(v1 - v0) / 60.0).max(1.0);
    let mut good = vec![];
    let mut a = u0 + step / 2.0;
    while a < u1 {
        let mut c = v0 + step / 2.0;
        while c < v1 {
            let p = n * at + u * a + v * c;
            // the pin goes half into each piece: check a few points along it
            let ok = [-0.4, 0.0, 0.4].iter().all(|&f| {
                let q = p + n * f * pin_d * 1.5;
                inside(m, &bvh, q, (u * 0.37 + v * 0.61 + n * 0.11).normalize()) && dist_to_surface(m, &bvh, q, margin) >= margin - 1e-9
            });
            if ok {
                good.push(p);
            }
            c += step;
        }
        a += step;
    }
    if good.is_empty() || want == 0 {
        return vec![];
    }
    if want == 1 || good.len() == 1 {
        let cen = good.iter().copied().sum::<DVec3>() / good.len() as f64;
        return vec![*good.iter().min_by(|a, b| a.distance(cen).total_cmp(&b.distance(cen))).unwrap()];
    }
    // two pins as far apart as possible (stops the pieces twisting)
    let mut best = (0, 0, -1.0);
    for i in 0..good.len() {
        for j in i + 1..good.len() {
            let d = good[i].distance(good[j]);
            if d > best.2 {
                best = (i, j, d);
            }
        }
    }
    vec![good[best.0], good[best.1]]
}

pub fn split(m: &Mesh, o: &SplitOptions) -> Result<SplitResult, String> {
    let n = axis_vec(o.axis);
    let b = m.bounds();
    let (lo, hi) = (b.min.dot(n), b.max.dot(n));
    if o.at <= lo + 0.5 || o.at >= hi - 0.5 {
        return Err("The cut has to go through the part — move the cut position inside it.".into());
    }
    let big = (b.max - b.min).max_element() + 20.0;
    let c = b.center();
    let half = |upper: bool| {
        let mut min = c - DVec3::splat(big);
        let mut max = c + DVec3::splat(big);
        if upper {
            min[o.axis] = o.at;
        } else {
            max[o.axis] = o.at;
        }
        let mut cube = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(max - min);
        let off = DVec3::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0, min.z);
        for p in &mut cube.verts {
            *p += off;
        }
        cube
    };
    let spots = if o.pins { pin_spots(m, o.axis, o.at, o.pin_d, 2) } else { vec![] };
    let mut holes = vec![];
    for s in &spots {
        let r = o.pin_d / 2.0 + o.gap;
        let depth = o.pin_len / 2.0 + 0.5;
        let mut cyl = crate::shapes::Shape::Cylinder { sides: 32, bevel: 0.0 }.generate(DVec3::new(2.0 * r, 2.0 * r, 2.0 * depth));
        // cylinder stands along Z from 0..2·depth; turn it to the cut axis and centre it on the spot
        for p in &mut cyl.verts {
            let q = *p - DVec3::new(0.0, 0.0, depth);
            *p = match o.axis {
                0 => DVec3::new(q.z, q.x, q.y),
                1 => DVec3::new(q.x, q.z, q.y),
                _ => q,
            } + *s;
        }
        holes.push(cyl);
    }
    let low = crate::csg::combine(&[m.clone()], &[&holes[..], &[half(true)]].concat()).mesh;
    let high = crate::csg::combine(&[m.clone()], &[&holes[..], &[half(false)]].concat()).mesh;
    if low.tris.is_empty() || high.tris.is_empty() {
        return Err("Splitting failed — try moving the cut a little.".into());
    }
    Ok(SplitResult { low, high, pin_spots: spots })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    #[test]
    fn split_box_with_pins() {
        let b = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(100.0, 40.0, 20.0));
        let r = split(&b, &SplitOptions { axis: 0, at: 0.0, pins: true, pin_d: 5.0, pin_len: 10.0, gap: 0.15 }).unwrap();
        assert_eq!(r.pin_spots.len(), 2);
        let pin_vol = std::f64::consts::PI * 2.65f64.powi(2) * 5.5;
        // each half: 50·40·20 minus two half-pin holes
        for h in [&r.low, &r.high] {
            let v = h.volume();
            assert!((v - (40000.0 - 2.0 * pin_vol)).abs() < 150.0, "{v}");
        }
        assert!(r.low.bounds().max.x <= 1e-6 && r.high.bounds().min.x >= -1e-6);
        // the two pins are spread out
        assert!(r.pin_spots[0].distance(r.pin_spots[1]) > 15.0);
    }
}
