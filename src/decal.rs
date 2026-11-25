//! Picture decals: project an image onto a part's surface as filament paint.
//!
//! The picture is projected like a slide projector aimed at the surface: every
//! triangle facing the projector inside the picture's rectangle gets the closest
//! allowed filament colour. The surface is subdivided locally first so the
//! picture keeps its detail on big flat faces.

use crate::imaging::{nearest_filament, Picture};
use crate::mesh::{Bvh, Mesh, NO_COLOR};
use crate::scene::Filament;
use glam::DVec3;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Decal {
    pub pic: std::sync::Arc<Picture>,
    pub width: f64,
    pub rotation: f64,
    pub allowed: Vec<bool>,
    pub skip_background: bool,
    pub from_view: bool,
}

/// Where the picture goes: centre, projection direction (pointing out of the surface,
/// towards the projector) and the picture's right / up directions.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub centre: DVec3,
    pub n: DVec3,
    pub right: DVec3,
    pub up: DVec3,
}

impl Decal {
    pub fn height(&self) -> f64 {
        self.width * self.pic.h as f64 / self.pic.w.max(1) as f64
    }

    /// Build a frame. `n` = surface normal (or direction to the viewer), `up_hint` =
    /// which way is up (world Z on walls, the screen's up on top faces).
    pub fn frame(&self, centre: DVec3, n: DVec3, up_hint: DVec3, screen_up: DVec3) -> Frame {
        let n = n.normalize_or(DVec3::Z);
        let mut up = up_hint - n * up_hint.dot(n);
        if up.length() < 0.3 {
            up = screen_up - n * screen_up.dot(n);
        }
        if up.length() < 1e-6 {
            up = n.any_orthonormal_vector();
        }
        let up = up.normalize();
        let right = up.cross(n).normalize();
        let (s, c) = self.rotation.to_radians().sin_cos();
        let r2 = right * c + up * s;
        let u2 = up * c - right * s;
        Frame { centre, n, right: r2, up: u2 }
    }

    /// Four corners (top-left, top-right, bottom-right, bottom-left), lifted slightly off
    /// the surface — for drawing a preview.
    pub fn corners(&self, f: &Frame) -> [DVec3; 4] {
        let (hw, hh) = (self.width / 2.0, self.height() / 2.0);
        let c = f.centre + f.n * 0.05;
        [c - f.right * hw + f.up * hh, c + f.right * hw + f.up * hh, c + f.right * hw - f.up * hh, c - f.right * hw - f.up * hh]
    }

    /// Paint the picture onto `mesh` (local coordinates, colours already applied).
    /// Returns the new mesh and how many triangles were painted.
    pub fn apply(&self, mesh: &Mesh, f: &Frame, fils: &[Filament]) -> (Mesh, usize) {
        let (w, h) = (self.width, self.height());
        let (hw, hh) = (w / 2.0, h / 2.0);
        // colour resolution: about 150 dots across, never finer than 0.25 mm
        let target = (w.max(h) / 150.0).clamp(0.25, 3.0);
        let depth = w.max(h) * 0.75 + 1.0;
        let st = |p: DVec3| {
            let d = p - f.centre;
            (d.dot(f.right), d.dot(f.up), d.dot(f.n))
        };
        let touches = |m: &Mesh, t: usize, margin: f64| -> bool {
            let (nrm, _) = m.tri_normal_area(t);
            if nrm.dot(f.n) < 0.12 {
                return false;
            }
            let pts = m.tri_points(t).map(st);
            let (mut s0, mut s1, mut t0, mut t1, mut d0, mut d1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for (s, t, d) in pts {
                s0 = s0.min(s);
                s1 = s1.max(s);
                t0 = t0.min(t);
                t1 = t1.max(t);
                d0 = d0.min(d);
                d1 = d1.max(d);
            }
            s1 >= -hw - margin && s0 <= hw + margin && t1 >= -hh - margin && t0 <= hh + margin && d1 >= -depth && d0 <= depth
        };
        // 1. subdivide the surface under the picture
        let mut m = mesh.clone();
        let t2 = target * target;
        for _ in 0..16 {
            if m.tris.len() > 3_000_000 {
                break;
            }
            let mut edges = HashSet::new();
            for t in 0..m.tris.len() {
                if !touches(&m, t, target) {
                    continue;
                }
                let tri = m.tris[t];
                for k in 0..3 {
                    let (i, j) = (tri[k], tri[(k + 1) % 3]);
                    if m.verts[i as usize].distance_squared(m.verts[j as usize]) > t2 {
                        edges.insert((i.min(j), i.max(j)));
                    }
                }
            }
            if edges.is_empty() {
                break;
            }
            m = m.split_edges(&edges);
        }
        // 2. background colour (for "skip background"): the picture's corners
        let bg = {
            let mut acc = [0.0; 4];
            for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let s = self.pic.sample(u, v);
                for k in 0..4 {
                    acc[k] += s[k] / 4.0;
                }
            }
            acc
        };
        // 3. colour every visible triangle inside the picture
        let bvh = Bvh::build(&m);
        let mut painted = 0;
        for t in 0..m.tris.len() {
            if !touches(&m, t, 0.0) {
                continue;
            }
            let [a, b, c] = m.tri_points(t);
            let cen = (a + b + c) / 3.0;
            let (s, tt, d) = st(cen);
            if s.abs() > hw || tt.abs() > hh || d.abs() > depth {
                continue;
            }
            // hidden behind another part of the surface?
            if bvh.raycast(&m, cen + f.n * 1e-4, f.n, Some(t as u32)).is_some() {
                continue;
            }
            let (u, v) = ((s + hw) / w, (hh - tt) / h);
            let px = self.pic.sample(u, v);
            if px[3] < 128.0 {
                continue;
            }
            if self.skip_background {
                let dist = (0..3).map(|k| (px[k] - bg[k]).powi(2)).sum::<f64>().sqrt();
                if bg[3] < 128.0 || dist < 40.0 {
                    continue;
                }
            }
            let slot = nearest_filament([px[0], px[1], px[2]], fils, &self.allowed);
            if m.colors[t] != slot {
                m.colors[t] = slot;
            }
            painted += 1;
        }
        let _ = NO_COLOR;
        (m, painted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn fils() -> Vec<Filament> {
        vec![
            Filament { name: "White".into(), color: [255, 255, 255], material: "PLA".into() },
            Filament { name: "Red".into(), color: [220, 30, 30], material: "PLA".into() },
            Filament { name: "Blue".into(), color: [30, 60, 220], material: "PLA".into() },
        ]
    }

    #[test]
    fn decal_paints_only_the_front_face_and_stays_watertight() {
        // left half red, right half blue, 20x10 px
        let (pw, ph) = (20u32, 10u32);
        let mut rgba = vec![];
        for _y in 0..ph {
            for x in 0..pw {
                rgba.extend_from_slice(if x < pw / 2 { &[220, 30, 30, 255] } else { &[30, 60, 220, 255] });
            }
        }
        let pic = Arc::new(Picture { w: pw, h: ph, rgba });
        let mut m = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(40.0));
        m.fill_unset_color(0);
        let d = Decal { pic, width: 20.0, rotation: 0.0, allowed: vec![true; 3], skip_background: false, from_view: false };
        // front face is y = -20, facing -Y
        let f = d.frame(DVec3::new(0.0, -20.0, 20.0), DVec3::NEG_Y, DVec3::Z, DVec3::Z);
        let (out, n) = d.apply(&m, &f, &fils());
        assert!(n > 100, "painted {n}");
        let vol = out.volume();
        assert!((vol - 64000.0).abs() < 1e-3, "{vol}");
        let mut e = HashSet::new();
        for t in &out.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        assert!(e.iter().all(|&(a, b)| e.contains(&(b, a))), "not watertight");
        for t in 0..out.tris.len() {
            let [a, b, c] = out.tri_points(t);
            let cen = (a + b + c) / 3.0;
            if out.colors[t] != 0 {
                assert!((cen.y + 20.0).abs() < 1e-6, "paint off the front face");
                assert!(cen.x.abs() <= 10.0 + 1e-6 && (cen.z - 20.0).abs() <= 5.0 + 1e-6);
                // seen from the front, picture left = -X
                assert_eq!(out.colors[t], if cen.x < 0.0 { 1 } else { 2 });
            }
        }
    }
}

#[cfg(test)]
mod app_math {
    use super::*;
    use crate::scene::{Kind, MeshCache, Object};
    use crate::shapes::Shape;
    #[test]
    fn decal_on_object_front_face() {
        let o = Object::new(1, "b", Kind::Shape(Shape::Box { radius: 0.0, steps: 1 }), DVec3::new(80.0, 50.0, 50.0));
        let mut c = MeshCache::default();
        let wm = c.world_mesh(&o);
        let p = DVec3::new(0.0, -25.0, 45.0);
        let bvh = Bvh::build(&wm);
        let (t, tri) = bvh.raycast(&wm, p - DVec3::Y * 10.0, DVec3::Y, None).unwrap();
        let hit = p - DVec3::Y * 10.0 + DVec3::Y * t;
        let (n, _) = wm.tri_normal_area(tri as usize);
        eprintln!("hit {hit:?} n {n:?}");
        let pic = std::sync::Arc::new(Picture { w: 2, h: 2, rgba: vec![255, 0, 0, 255].repeat(4) });
        let d = Decal { pic, width: 30.0, rotation: 0.0, allowed: vec![true; 2], skip_background: false, from_view: false };
        let wf = d.frame(hit, n, DVec3::Z, DVec3::Z);
        let inv = o.transform().inverse();
        let lf = Frame { centre: inv.transform_point3(wf.centre), n: inv.transform_vector3(wf.n), right: inv.transform_vector3(wf.right), up: inv.transform_vector3(wf.up) };
        eprintln!("{lf:?}");
        let mut m = (*c.base_mesh(&o)).clone();
        m.fill_unset_color(0);
        let fils = vec![
            Filament { name: "W".into(), color: [255, 255, 255], material: "PLA".into() },
            Filament { name: "R".into(), color: [255, 0, 0], material: "PLA".into() },
        ];
        let (out, k) = d.apply(&m, &lf, &fils);
        for t in 0..out.tris.len() {
            if out.colors[t] == 1 {
                let [a, b, cc] = out.tri_points(t);
                let cen = (a + b + cc) / 3.0;
                assert!((cen.y + 25.0).abs() < 1e-6, "painted at {cen:?}");
            }
        }
        assert!(k > 0);
    }

    #[test]
    fn decal_wraps_front_of_cylinder_only() {
        let mut m = Shape::Cylinder { sides: 64, bevel: 0.0 }.generate(DVec3::new(40.0, 40.0, 40.0));
        m.fill_unset_color(0);
        let pic = std::sync::Arc::new(Picture { w: 2, h: 2, rgba: vec![255, 0, 0, 255].repeat(4) });
        let d = Decal { pic, width: 30.0, rotation: 0.0, allowed: vec![true; 2], skip_background: false, from_view: true };
        let f = d.frame(DVec3::new(0.0, -20.0, 20.0), DVec3::NEG_Y, DVec3::Z, DVec3::Z);
        let fils = vec![
            Filament { name: "W".into(), color: [255, 255, 255], material: "PLA".into() },
            Filament { name: "R".into(), color: [255, 0, 0], material: "PLA".into() },
        ];
        let (out, k) = d.apply(&m, &f, &fils);
        assert!(k > 50);
        for t in 0..out.tris.len() {
            if out.colors[t] == 1 {
                let [a, b, c] = out.tri_points(t);
                let cen = (a + b + c) / 3.0;
                assert!(cen.y < 0.0 && cen.x.abs() <= 15.0 + 1e-6, "{cen:?}");
            }
        }
    }
}
