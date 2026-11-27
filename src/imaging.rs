//! Pictures to 3D: lithophanes, raised reliefs, colour photo plates, and SVG logos.

use crate::mesh::{Mesh, NO_COLOR};
use crate::scene::Filament;
use crate::shapes::{extrude, nest_contours};
use glam::{DVec2, DVec3};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageMode {
    /// Thin where bright, thick where dark — shows the picture when lit from behind.
    Lithophane,
    /// Raised where bright (or dark, if inverted) on a base plate.
    Relief,
    /// Flat plate whose top is coloured with your filaments (nearest match per pixel).
    ColourPlate,
    /// A round lithophane: the picture wraps round a cylinder (light goes inside).
    LampShade,
}

#[derive(Clone, Debug)]
pub struct ImageOptions {
    pub mode: ImageMode,
    pub width_mm: f64,
    /// lithophane: thinnest / thickest; relief: base / extra height; plate: plate thickness
    pub min_mm: f64,
    pub max_mm: f64,
    pub invert: bool,
    pub border_mm: f64,
    /// grid resolution (points per mm)
    pub detail: f64,
    /// filaments allowed for the colour plate
    pub use_filaments: Vec<bool>,
    /// lamp shade: how many times the picture goes round
    pub repeats: u32,
}

impl Default for ImageOptions {
    fn default() -> Self {
        ImageOptions {
            mode: ImageMode::Lithophane,
            width_mm: 100.0,
            min_mm: 0.8,
            max_mm: 3.0,
            invert: false,
            border_mm: 3.0,
            detail: 4.0,
            use_filaments: vec![true; 24],
            repeats: 1,
        }
    }
}

pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

pub fn load_picture(path: &std::path::Path) -> Result<Picture, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Picture { w: img.width(), h: img.height(), rgba: img.into_raw() })
}

impl Picture {
    pub fn sample(&self, u: f64, v: f64) -> [f64; 4] {
        // bilinear sample, u,v in [0,1], v=0 top of the image
        let x = (u * (self.w - 1) as f64).clamp(0.0, (self.w - 1) as f64);
        let y = (v * (self.h - 1) as f64).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let px = |x: u32, y: u32| {
            let i = ((y * self.w + x) * 4) as usize;
            [self.rgba[i] as f64, self.rgba[i + 1] as f64, self.rgba[i + 2] as f64, self.rgba[i + 3] as f64]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            out[k] = a[k] * (1.0 - fx) * (1.0 - fy) + b[k] * fx * (1.0 - fy) + c[k] * (1.0 - fx) * fy + d[k] * fx * fy;
        }
        out
    }
    /// 0 = black, 1 = white (transparent counts as white)
    fn lum(&self, u: f64, v: f64) -> f64 {
        let [r, g, b, a] = self.sample(u, v);
        let l = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        let a = a / 255.0;
        l * a + (1.0 - a)
    }
}

/// Closed heightfield: top surface z = height(i, j) over an nx × ny grid spanning
/// [0, w] × [0, d], flat bottom at z = 0. `color(i, j)` colours the top cell.
pub fn heightfield(nx: usize, ny: usize, w: f64, d: f64, height: &dyn Fn(usize, usize) -> f64, color: &dyn Fn(usize, usize) -> u8) -> Mesh {
    let mut m = Mesh::new();
    let x = |i: usize| w * i as f64 / (nx - 1) as f64;
    let y = |j: usize| d * j as f64 / (ny - 1) as f64;
    let mut top = vec![0u32; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            top[j * nx + i] = m.add_vert(DVec3::new(x(i), y(j), height(i, j).max(0.05)));
        }
    }
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let (a, b, c, dd) = (top[j * nx + i], top[j * nx + i + 1], top[(j + 1) * nx + i + 1], top[(j + 1) * nx + i]);
            let col = color(i, j);
            m.add_tri(a, b, c);
            *m.colors.last_mut().unwrap() = col;
            m.add_tri(a, c, dd);
            *m.colors.last_mut().unwrap() = col;
        }
    }
    // perimeter (counter-clockwise seen from above)
    let mut ring: Vec<(usize, usize)> = Vec::new();
    for i in 0..nx - 1 {
        ring.push((i, 0));
    }
    for j in 0..ny - 1 {
        ring.push((nx - 1, j));
    }
    for i in (1..nx).rev() {
        ring.push((i, ny - 1));
    }
    for j in (1..ny).rev() {
        ring.push((0, j));
    }
    let bottom: Vec<u32> = ring.iter().map(|&(i, j)| m.add_vert(DVec3::new(x(i), y(j), 0.0))).collect();
    let centre = m.add_vert(DVec3::new(w / 2.0, d / 2.0, 0.0));
    let n = ring.len();
    for k in 0..n {
        let k2 = (k + 1) % n;
        let (i, j) = ring[k];
        let (i2, j2) = ring[k2];
        let (ta, tb) = (top[j * nx + i], top[j2 * nx + i2]);
        m.add_quad(bottom[k], bottom[k2], tb, ta);
        m.add_tri(centre, bottom[k2], bottom[k]);
    }
    crate::shapes::orient_outward(&mut m);
    m
}

pub fn nearest_filament(rgb: [f64; 3], fils: &[Filament], allowed: &[bool]) -> u8 {
    let mut best = (f64::INFINITY, 0usize);
    for (i, f) in fils.iter().enumerate() {
        if !allowed.get(i).copied().unwrap_or(true) {
            continue;
        }
        // perceptual-ish weighting ("redmean")
        let rm = (rgb[0] + f.color[0] as f64) / 2.0;
        let (dr, dg, db) = (rgb[0] - f.color[0] as f64, rgb[1] - f.color[1] as f64, rgb[2] - f.color[2] as f64);
        let dist = (2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db;
        if dist < best.0 {
            best = (dist, i);
        }
    }
    best.1 as u8
}

/// Build a part from a picture. Returns (mesh at its natural size on z=0, description).
/// Height of a lamp shade for this picture (including the solid rims).
pub fn lamp_height(pic: &Picture, o: &ImageOptions) -> f64 {
    let circ = std::f64::consts::PI * o.width_mm;
    circ / o.repeats.max(1) as f64 * pic.h as f64 / pic.w as f64 + 2.0 * o.border_mm.max(0.0)
}

/// Round lithophane: `width_mm` is the outside diameter, `border_mm` solid rims top and
/// bottom. Open at both ends so it slips over a tea light or LED puck.
pub fn lamp_shade(pic: &Picture, o: &ImageOptions) -> Mesh {
    use std::f64::consts::TAU;
    let r_out = o.width_mm / 2.0;
    let (tmin, tmax) = (o.min_mm.min(o.max_mm), o.max_mm.max(o.min_mm));
    let r_in = r_out - tmax;
    let reps = o.repeats.max(1) as f64;
    let b = o.border_mm.max(0.0);
    let height = lamp_height(pic, o);
    let circ = TAU * r_out;
    let nu = ((circ * o.detail) as usize).clamp(48, 600);
    let nv = ((height * o.detail) as usize).clamp(8, 400);
    let thick = |i: usize, j: usize| -> f64 {
        let z = height * j as f64 / (nv - 1) as f64;
        if z < b || z > height - b {
            return tmax;
        }
        let u = (i as f64 / nu as f64 * reps).fract();
        let v = 1.0 - (z - b) / (height - 2.0 * b).max(1e-6);
        let mut l = pic.lum(u, v);
        if o.invert {
            l = 1.0 - l;
        }
        tmin + (1.0 - l) * (tmax - tmin)
    };
    let mut m = Mesh::new();
    let ang = |i: usize| TAU * i as f64 / nu as f64;
    // outside surface: nu × nv, wrapping round
    let outer: Vec<u32> = (0..nv)
        .flat_map(|j| (0..nu).map(move |i| (i, j)))
        .map(|(i, j)| {
            let r = r_in + thick(i, j);
            let a = ang(i);
            m.add_vert(DVec3::new(r * a.cos(), r * a.sin(), height * j as f64 / (nv - 1) as f64))
        })
        .collect();
    let inner_bot: Vec<u32> = (0..nu).map(|i| m.add_vert(DVec3::new(r_in * ang(i).cos(), r_in * ang(i).sin(), 0.0))).collect();
    let inner_top: Vec<u32> = (0..nu).map(|i| m.add_vert(DVec3::new(r_in * ang(i).cos(), r_in * ang(i).sin(), height))).collect();
    let o_at = |i: usize, j: usize| outer[j * nu + i % nu];
    for j in 0..nv - 1 {
        for i in 0..nu {
            m.add_quad(o_at(i, j), o_at(i + 1, j), o_at(i + 1, j + 1), o_at(i, j + 1));
        }
    }
    for i in 0..nu {
        let k = (i + 1) % nu;
        // inside wall (faces inwards)
        m.add_quad(inner_bot[k], inner_bot[i], inner_top[i], inner_top[k]);
        // bottom ring (faces down)
        m.add_quad(inner_bot[i], inner_bot[k], o_at(i + 1, 0), o_at(i, 0));
        // top ring (faces up)
        m.add_quad(inner_top[k], inner_top[i], o_at(i, nv - 1), o_at(i + 1, nv - 1));
    }
    crate::shapes::orient_outward(&mut m);
    m
}

pub fn picture_to_mesh(pic: &Picture, o: &ImageOptions, fils: &[Filament]) -> Mesh {
    if o.mode == ImageMode::LampShade {
        return lamp_shade(pic, o);
    }
    let aspect = pic.h as f64 / pic.w as f64;
    let (w, d) = (o.width_mm, o.width_mm * aspect);
    let max_pts = 420.0;
    let nx = ((w * o.detail).min(max_pts) as usize).max(8);
    let ny = (((d * o.detail).min(max_pts)) as usize).max(8);
    let b = o.border_mm.max(0.0);
    let inside = |i: usize, j: usize| -> Option<(f64, f64)> {
        let (xm, ym) = (w * i as f64 / (nx - 1) as f64, d * j as f64 / (ny - 1) as f64);
        if xm < b || xm > w - b || ym < b || ym > d - b {
            None
        } else {
            Some(((xm - b) / (w - 2.0 * b).max(1e-6), 1.0 - (ym - b) / (d - 2.0 * b).max(1e-6)))
        }
    };
    match o.mode {
        ImageMode::LampShade => lamp_shade(pic, o),
        ImageMode::Lithophane => {
            let h = |i: usize, j: usize| match inside(i, j) {
                None => o.max_mm,
                Some((u, v)) => {
                    let mut l = pic.lum(u, v);
                    if o.invert {
                        l = 1.0 - l;
                    }
                    o.min_mm + (1.0 - l) * (o.max_mm - o.min_mm)
                }
            };
            heightfield(nx, ny, w, d, &h, &|_, _| NO_COLOR)
        }
        ImageMode::Relief => {
            let h = |i: usize, j: usize| match inside(i, j) {
                None => o.min_mm,
                Some((u, v)) => {
                    let mut l = pic.lum(u, v);
                    if o.invert {
                        l = 1.0 - l;
                    }
                    o.min_mm + l * o.max_mm
                }
            };
            heightfield(nx, ny, w, d, &h, &|_, _| NO_COLOR)
        }
        ImageMode::ColourPlate => {
            let col = |i: usize, j: usize| match inside(i, j) {
                None => NO_COLOR,
                Some((u, v)) => {
                    let [r, g, bb, a] = pic.sample(u, v);
                    let a = a / 255.0;
                    let rgb = [r * a + 255.0 * (1.0 - a), g * a + 255.0 * (1.0 - a), bb * a + 255.0 * (1.0 - a)];
                    nearest_filament(rgb, fils, &o.use_filaments)
                }
            };
            heightfield(nx, ny, w, d, &|_, _| o.min_mm.max(0.4), &col)
        }
    }
}

// ------------------------------------------------------------------ SVG

/// One colour layer of a logo: (fill colour, polygon groups in mm, y up).
pub struct SvgLayer {
    pub color: [u8; 3],
    pub groups: Vec<Vec<Vec<DVec2>>>,
}

fn flatten(path: &usvg::tiny_skia_path::Path) -> Vec<Vec<DVec2>> {
    use usvg::tiny_skia_path::PathSegment;
    let mut out = Vec::new();
    let mut cur: Vec<DVec2> = Vec::new();
    let p = |pt: usvg::tiny_skia_path::Point| DVec2::new(pt.x as f64, pt.y as f64);
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(a) => {
                if cur.len() > 2 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                cur.push(p(a));
            }
            PathSegment::LineTo(a) => cur.push(p(a)),
            PathSegment::QuadTo(c, a) => {
                let p0 = *cur.last().unwrap_or(&DVec2::ZERO);
                let (c, a) = (p(c), p(a));
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    cur.push(p0 * (1.0 - t) * (1.0 - t) + c * 2.0 * t * (1.0 - t) + a * t * t);
                }
            }
            PathSegment::CubicTo(c1, c2, a) => {
                let p0 = *cur.last().unwrap_or(&DVec2::ZERO);
                let (c1, c2, a) = (p(c1), p(c2), p(a));
                for k in 1..=12 {
                    let t = k as f64 / 12.0;
                    let u = 1.0 - t;
                    cur.push(p0 * u * u * u + c1 * 3.0 * u * u * t + c2 * 3.0 * u * t * t + a * t * t * t);
                }
            }
            PathSegment::Close => {
                if cur.len() > 2 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() > 2 {
        out.push(cur);
    }
    for c in &mut out {
        c.dedup_by(|a, b| a.distance(*b) < 1e-6);
        if c.len() > 1 && c[0].distance(*c.last().unwrap()) < 1e-6 {
            c.pop();
        }
    }
    out.retain(|c| c.len() >= 3);
    out
}

/// Parse an SVG into colour layers, in drawing order (later layers are on top).
pub fn load_svg(data: &[u8]) -> Result<Vec<SvgLayer>, String> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut layers: Vec<SvgLayer> = Vec::new();
    fn walk(g: &usvg::Group, layers: &mut Vec<SvgLayer>) {
        for n in g.children() {
            match n {
                usvg::Node::Group(g) => walk(g, layers),
                usvg::Node::Path(p) => {
                    let ts = p.abs_transform();
                    let mut add = |path: &usvg::tiny_skia_path::Path, color: [u8; 3]| {
                        if let Some(tp) = path.clone().transform(ts) {
                            let contours = flatten(&tp);
                            if contours.is_empty() {
                                return;
                            }
                            let groups = nest_contours(contours);
                            match layers.last_mut() {
                                Some(l) if l.color == color => l.groups.extend(groups),
                                _ => layers.push(SvgLayer { color, groups }),
                            }
                        }
                    };
                    let paint_rgb = |paint: &usvg::Paint| match paint {
                        usvg::Paint::Color(c) => [c.red, c.green, c.blue],
                        _ => [128, 128, 128],
                    };
                    if let Some(f) = p.fill() {
                        add(p.data(), paint_rgb(f.paint()));
                    }
                    if let Some(s) = p.stroke() {
                        if let Some(stroked) = p.data().stroke(&s.to_tiny_skia(), 1.0) {
                            add(&stroked, paint_rgb(s.paint()));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    walk(tree.root(), &mut layers);
    if layers.is_empty() {
        return Err("No filled shapes found in that SVG.".into());
    }
    Ok(layers)
}

/// Extrude SVG layers to meshes scaled to `width_mm`, centred, y flipped (SVG y is down).
/// Returns (colour, mesh) per layer; later layers are cut out of earlier ones so they don't overlap.
pub fn svg_meshes(layers: &[SvgLayer], width_mm: f64, height_mm: f64) -> Vec<([u8; 3], Mesh)> {
    let mut mn = DVec2::splat(f64::INFINITY);
    let mut mx = DVec2::splat(f64::NEG_INFINITY);
    for l in layers {
        for p in l.groups.iter().flatten().flatten() {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
    }
    let s = width_mm / (mx.x - mn.x).max(1e-9);
    let c = (mn + mx) * 0.5;
    let mut meshes: Vec<([u8; 3], Mesh)> = layers
        .iter()
        .map(|l| {
            let groups: Vec<Vec<Vec<DVec2>>> = l
                .groups
                .iter()
                .map(|g| g.iter().map(|ct| ct.iter().map(|p| DVec2::new((p.x - c.x) * s, (c.y - p.y) * s)).collect()).collect())
                .collect();
            let mut m = extrude(&groups, 0.0, height_mm);
            m.weld(1e-7);
            crate::shapes::orient_outward(&mut m);
            (l.color, m)
        })
        .collect();
    // remove what later (on-top) layers cover
    for i in 0..meshes.len() {
        let later: Vec<Mesh> = meshes[i + 1..].iter().map(|x| {
            let mut m = x.1.clone();
            // slightly taller cutter so coplanar tops/bottoms cut cleanly
            for v in &mut m.verts {
                v.z = if v.z > height_mm * 0.5 { height_mm + 1.0 } else { -1.0 };
            }
            m
        }).collect();
        if later.is_empty() {
            continue;
        }
        let r = crate::csg::combine(&[meshes[i].1.clone()], &later);
        if r.warnings.is_empty() {
            meshes[i].1 = r.mesh;
        }
    }
    meshes.retain(|m| !m.1.tris.is_empty());
    meshes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamp_shade_closed() {
        let pic = Picture { w: 4, h: 2, rgba: (0..32).map(|k| if k % 8 < 4 { 0 } else { 255 }).collect() };
        let o = ImageOptions { mode: ImageMode::LampShade, width_mm: 80.0, detail: 2.0, ..Default::default() };
        let m = lamp_shade(&pic, &o);
        let mut edges: std::collections::HashMap<(u32, u32), i32> = Default::default();
        for t in &m.tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|c| *c == 2));
        assert!(m.volume() > 0.0);
        let b = m.bounds();
        assert!((b.max.x - 40.0).abs() < 0.01 && (b.max.z - lamp_height(&pic, &o)).abs() < 0.01);
    }

    #[test]
    fn heightfield_is_watertight() {
        let m = heightfield(20, 12, 50.0, 30.0, &|i, j| 1.0 + ((i * j) % 5) as f64 * 0.3, &|_, _| NO_COLOR);
        let mut e = std::collections::HashSet::new();
        for t in &m.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        assert!(e.iter().all(|&(a, b)| e.contains(&(b, a))));
        assert!(m.volume() > 0.0);
    }

    #[test]
    fn svg_two_colours() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect x="0" y="0" width="100" height="100" fill="#ff0000"/>
            <circle cx="50" cy="50" r="25" fill="#0000ff"/>
            <path d="M10 90 L30 90" stroke="#00ff00" stroke-width="4" fill="none"/>
        </svg>"##;
        let layers = load_svg(svg).unwrap();
        assert_eq!(layers.len(), 3);
        let ms = svg_meshes(&layers, 50.0, 2.0);
        assert_eq!(ms.len(), 3);
        // red square with the circle (and stroke) cut out: 50*50 - pi*12.5^2 - stroke
        let red = ms[0].1.volume();
        let expect = (2500.0 - std::f64::consts::PI * 156.25) * 2.0;
        assert!((red - expect).abs() < 60.0, "{red} vs {expect}");
    }

    #[test]
    fn colour_plate_maps_to_filaments() {
        let pic = Picture { w: 2, h: 1, rgba: vec![250, 10, 10, 255, 10, 10, 240, 255] };
        let fils = crate::scene::default_filaments();
        let o = ImageOptions { mode: ImageMode::ColourPlate, width_mm: 20.0, border_mm: 0.0, ..Default::default() };
        let m = picture_to_mesh(&pic, &o, &fils);
        let used = m.used_colors();
        assert!(used.contains(&2) && used.contains(&3), "{used:?}");
    }
}
