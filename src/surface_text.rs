//! Raised or cut-in text that follows a part's surface (flat or curved).

use crate::mesh::{Bvh, Mesh};
use crate::shapes::{FontChoice, Shape};
use glam::{DVec2, DVec3};
use std::collections::HashSet;

#[derive(Clone)]
pub struct SurfaceText {
    pub text: String,
    pub font: FontChoice,
    /// letter height (mm)
    pub height: f64,
    /// true = cut into the surface, false = raised
    pub engrave: bool,
    /// how far letters stick out / go in (mm)
    pub depth: f64,
    pub rotation: f64,
    pub color: u8,
    pub from_view: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub centre: DVec3,
    pub n: DVec3,
    pub right: DVec3,
    pub up: DVec3,
}

impl SurfaceText {
    pub fn width(&self) -> f64 {
        let s = Shape::Text { text: self.text.clone(), font: self.font, spacing: 0.0 };
        s.text_aspect().unwrap_or(1.0) * self.height
    }

    pub fn frame(&self, centre: DVec3, n: DVec3, screen_up: DVec3) -> Frame {
        let n = n.normalize_or(DVec3::Z);
        let mut up = DVec3::Z - n * n.z;
        if up.length() < 0.3 {
            up = screen_up - n * screen_up.dot(n);
        }
        if up.length() < 1e-6 {
            up = n.any_orthonormal_vector();
        }
        let up = up.normalize();
        let right = up.cross(n).normalize();
        let (s, c) = self.rotation.to_radians().sin_cos();
        Frame { centre, n, right: right * c + up * s, up: up * c - right * s }
    }

    pub fn corners(&self, f: &Frame) -> [DVec3; 4] {
        let (hw, hh) = (self.width() / 2.0, self.height / 2.0);
        let c = f.centre + f.n * 0.05;
        [c - f.right * hw + f.up * hh, c + f.right * hw + f.up * hh, c + f.right * hw - f.up * hh, c - f.right * hw - f.up * hh]
    }

    /// Put the text on `part` (world coordinates, coloured). Returns the new part mesh.
    pub fn apply(&self, part: &Mesh, f: &Frame) -> Result<Mesh, String> {
        if self.text.trim().is_empty() {
            return Err("Type some text first.".into());
        }
        let (w, h) = (self.width(), self.height);
        let mut letters = Shape::Text { text: self.text.clone(), font: self.font, spacing: 0.0 }.generate(DVec3::new(w, h, 1.0));
        // finer triangles so the letters can follow a curve
        let step = (h / 6.0).clamp(0.4, 2.0);
        for _ in 0..8 {
            let mut e = HashSet::new();
            for t in &letters.tris {
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    let (pa, pb) = (letters.verts[a as usize], letters.verts[b as usize]);
                    if DVec2::new(pa.x - pb.x, pa.y - pb.y).length() > step {
                        e.insert((a.min(b), a.max(b)));
                    }
                }
            }
            if e.is_empty() {
                break;
            }
            letters = letters.split_edges(&e);
        }
        let bvh = Bvh::build(part);
        let span = part.bounds().size().length() + 10.0;
        let mut missed = 0;
        let mut surface = |x: f64, y: f64| -> f64 {
            let p = f.centre + f.right * x + f.up * y + f.n * span;
            match bvh.raycast(part, p, -f.n, None) {
                Some((t, _)) => span - t,
                None => {
                    missed += 1;
                    0.0
                }
            }
        };
        let total = letters.verts.len();
        let inner = 1.5 + self.depth;
        for v in &mut letters.verts {
            let (x, y, top) = (v.x, v.y, v.z > 0.5);
            let s = surface(x, y);
            let z = match (self.engrave, top) {
                (false, true) => s + self.depth,     // raised: top sits `depth` above the surface
                (false, false) => s - inner,         // …and the bottom is buried in the part
                (true, true) => s + 2.0,             // engrave cutter: starts outside
                (true, false) => s - self.depth,     // …and stops `depth` below the surface
            };
            *v = f.centre + f.right * x + f.up * y + f.n * z;
        }
        if missed * 3 > total {
            return Err("Most of the text hangs off the edge of the part — make it smaller or click nearer the middle.".into());
        }
        crate::shapes::orient_outward(&mut letters);
        letters.colors = vec![self.color; letters.tris.len()];
        let mut out = if self.engrave {
            crate::csg::combine(&[part.clone()], &[letters]).mesh
        } else {
            crate::csg::combine(&[part.clone(), letters], &[]).mesh
        };
        if out.tris.is_empty() {
            return Err("Couldn't add the text here — try another spot or size.".into());
        }
        if self.engrave {
            // colour the inside of the cut letters
            let polys = crate::shapes::text_polygons(&self.text, self.font, 0.0);
            let all: Vec<DVec2> = polys.iter().flatten().flatten().copied().collect();
            if !all.is_empty() {
                let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
                for p in &all {
                    mn = mn.min(*p);
                    mx = mx.max(*p);
                }
                let ext = (mx - mn).max(DVec2::splat(1e-9));
                let inside = |x: f64, y: f64| {
                    let q = (DVec2::new(x / w + 0.5, y / h + 0.5)) * ext + mn;
                    let mut c = 0;
                    for g in &polys {
                        for ring in g {
                            if crate::shapes::point_in_poly(q, ring) {
                                c += 1;
                            }
                        }
                    }
                    c % 2 == 1
                };
                let bvh2 = Bvh::build(part);
                for t in 0..out.tris.len() {
                    let [a, b, c] = out.tri_points(t);
                    let m = (a + b + c) / 3.0;
                    let d = m - f.centre;
                    let (x, y) = (d.dot(f.right), d.dot(f.up));
                    if x.abs() > w / 2.0 + 0.5 || y.abs() > h / 2.0 + 0.5 {
                        continue;
                    }
                    // below the original surface?
                    let p = f.centre + f.right * x + f.up * y + f.n * span;
                    let s = bvh2.raycast(part, p, -f.n, None).map(|(t, _)| span - t).unwrap_or(0.0);
                    if d.dot(f.n) < s - 0.05 && inside(x, y) {
                        out.colors[t] = self.color;
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(m: &Mesh) -> bool {
        let mut e = HashSet::new();
        for t in &m.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        e.iter().all(|&(a, b)| e.contains(&(b, a)))
    }

    #[test]
    fn raised_and_engraved_on_box_and_cylinder() {
        let mut b = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(60.0, 20.0, 20.0));
        b.fill_unset_color(0);
        let st = SurfaceText { text: "HI".into(), font: FontChoice::Sans, height: 10.0, engrave: false, depth: 1.0, rotation: 0.0, color: 3, from_view: false };
        // front face y = -10
        let f = st.frame(DVec3::new(0.0, -10.0, 10.0), DVec3::NEG_Y, DVec3::Z);
        let r = st.apply(&b, &f).unwrap();
        assert!(closed(&r));
        assert!(r.volume() > b.volume() + 5.0);
        assert!((r.bounds().min.y + 11.0).abs() < 1e-3, "raised 1 mm: {:?}", r.bounds());
        assert!(r.colors.iter().any(|&c| c == 3));
        let eng = SurfaceText { engrave: true, ..st.clone() };
        let e = eng.apply(&b, &f).unwrap();
        assert!(closed(&e) && e.volume() < b.volume() - 5.0);
        assert!(e.colors.iter().any(|&c| c == 3), "engraving coloured");
        // round surface: letters follow the curve
        let mut cy = Shape::Cylinder { sides: 64, bevel: 0.0 }.generate(DVec3::new(30.0, 30.0, 30.0));
        cy.fill_unset_color(0);
        let f2 = st.frame(DVec3::new(0.0, -15.0, 15.0), DVec3::NEG_Y, DVec3::Z);
        let c = st.apply(&cy, &f2).unwrap();
        assert!(closed(&c) && c.volume() > cy.volume());
    }
}
