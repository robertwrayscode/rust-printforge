//! Quick makers: name keychains, cookie cutters, outline tracing.
//!
//! Outlines are grown/shrunk with a signed distance field on a fine grid, then turned
//! back into smooth polygons with marching squares.

use crate::mesh::Mesh;
use crate::shapes::{extrude, nest_contours, text_polygons, FontChoice};
use glam::DVec2;
use std::collections::HashMap;

/// Signed distances (mm, negative inside) sampled at the centres of a grid.
pub struct Field {
    pub nx: usize,
    pub ny: usize,
    pub origin: DVec2,
    pub cell: f64,
    pub v: Vec<f64>,
}

/// 1-D squared Euclidean distance transform (Felzenszwalb & Huttenlocher).
fn edt_1d(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let isect = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
    for q in 1..n {
        let mut s = isect(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = isect(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let d = q as f64 - v[k] as f64;
        *o = d * d + f[v[k]];
    }
}

/// Distance (in cells) from each cell to the nearest cell where `mask` is true.
fn edt(mask: &[bool], nx: usize, ny: usize) -> Vec<f64> {
    let inf = 1e12;
    let mut g: Vec<f64> = mask.iter().map(|&m| if m { 0.0 } else { inf }).collect();
    let mut col = vec![0.0; ny];
    let mut colo = vec![0.0; ny];
    for i in 0..nx {
        for j in 0..ny {
            col[j] = g[j * nx + i];
        }
        edt_1d(&col, &mut colo);
        for j in 0..ny {
            g[j * nx + i] = colo[j];
        }
    }
    let mut row = vec![0.0; nx];
    for j in 0..ny {
        row.copy_from_slice(&g[j * nx..(j + 1) * nx]);
        edt_1d(&row, &mut g[j * nx..(j + 1) * nx]);
    }
    g.iter().map(|d| d.sqrt()).collect()
}

impl Field {
    /// Grid covering `mn..mx` plus `margin`, with cells of about `cell` mm.
    pub fn grid(mn: DVec2, mx: DVec2, margin: f64, cell: f64) -> Field {
        let origin = mn - DVec2::splat(margin);
        let ext = mx - mn + DVec2::splat(2.0 * margin);
        let nx = (ext.x / cell).ceil() as usize + 1;
        let ny = (ext.y / cell).ceil() as usize + 1;
        Field { nx, ny, origin, cell, v: vec![1.0; nx * ny] }
    }

    pub fn pos(&self, i: usize, j: usize) -> DVec2 {
        self.origin + DVec2::new(i as f64, j as f64) * self.cell
    }

    /// Signed distance to polygons (even-odd fill: outlines and their holes together).
    pub fn of_polygons(contours: &[Vec<DVec2>], margin: f64, cell: f64) -> Field {
        let pts: Vec<DVec2> = contours.iter().flatten().copied().collect();
        let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for p in &pts {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        let mut f = Field::grid(mn, mx, margin, cell);
        let mut mask = vec![false; f.nx * f.ny];
        let mut xs: Vec<f64> = vec![];
        for j in 0..f.ny {
            let y = f.origin.y + j as f64 * cell;
            xs.clear();
            for c in contours {
                for k in 0..c.len() {
                    let (a, b) = (c[k], c[(k + 1) % c.len()]);
                    if (a.y <= y) != (b.y <= y) {
                        xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let i0 = ((pair[0] - f.origin.x) / cell).ceil().max(0.0) as usize;
                let i1 = (((pair[1] - f.origin.x) / cell).floor().max(-1.0) + 1.0) as usize;
                for i in i0..i1.min(f.nx) {
                    mask[j * f.nx + i] = true;
                }
            }
        }
        f.set_from_mask(&mask);
        f
    }

    /// Signed distance from a filled/empty mask on this grid.
    pub fn set_from_mask(&mut self, mask: &[bool]) {
        let d_out = edt(mask, self.nx, self.ny);
        let inv: Vec<bool> = mask.iter().map(|m| !m).collect();
        let d_in = edt(&inv, self.nx, self.ny);
        for k in 0..mask.len() {
            self.v[k] = if mask[k] { -(d_in[k] - 0.5) } else { d_out[k] - 0.5 } * self.cell;
        }
    }

    /// Closed outlines where the field crosses `level` (smooth, simplified).
    pub fn contours(&self, level: f64) -> Vec<Vec<DVec2>> {
        let (nx, ny) = (self.nx, self.ny);
        let val = |i: usize, j: usize| -> f64 {
            // treat everything outside the grid as empty
            if i == 0 || j == 0 || i >= nx - 1 || j >= ny - 1 {
                self.v[j * nx + i].max(level + 1e-6)
            } else {
                self.v[j * nx + i]
            }
        };
        // edge ids: horizontal (i,j)-(i+1,j) = 2*(j*nx+i); vertical (i,j)-(i,j+1) = 2*(j*nx+i)+1
        let h = |i: usize, j: usize| 2 * (j * nx + i);
        let vt = |i: usize, j: usize| 2 * (j * nx + i) + 1;
        let mut point: HashMap<usize, DVec2> = HashMap::new();
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        let cross = |a: f64, b: f64| ((level - a) / (b - a)).clamp(0.0, 1.0);
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let (v0, v1, v2, v3) = (val(i, j), val(i + 1, j), val(i + 1, j + 1), val(i, j + 1));
                let (b0, b1, b2, b3) = (v0 < level, v1 < level, v2 < level, v3 < level);
                let mut e = vec![];
                if b0 != b1 {
                    let id = h(i, j);
                    point.entry(id).or_insert_with(|| self.pos(i, j) + DVec2::new(cross(v0, v1), 0.0) * self.cell);
                    e.push((0, id));
                }
                if b1 != b2 {
                    let id = vt(i + 1, j);
                    point.entry(id).or_insert_with(|| self.pos(i + 1, j) + DVec2::new(0.0, cross(v1, v2)) * self.cell);
                    e.push((1, id));
                }
                if b3 != b2 {
                    let id = h(i, j + 1);
                    point.entry(id).or_insert_with(|| self.pos(i, j + 1) + DVec2::new(cross(v3, v2), 0.0) * self.cell);
                    e.push((2, id));
                }
                if b0 != b3 {
                    let id = vt(i, j);
                    point.entry(id).or_insert_with(|| self.pos(i, j) + DVec2::new(0.0, cross(v0, v3)) * self.cell);
                    e.push((3, id));
                }
                let mut link = |a: usize, b: usize| {
                    adj.entry(a).or_default().push(b);
                    adj.entry(b).or_default().push(a);
                };
                match e.len() {
                    2 => link(e[0].1, e[1].1),
                    4 => {
                        let center = (v0 + v1 + v2 + v3) / 4.0 < level;
                        let id = |s: usize| e.iter().find(|x| x.0 == s).unwrap().1;
                        if center == b0 {
                            link(id(0), id(1));
                            link(id(2), id(3));
                        } else {
                            link(id(0), id(3));
                            link(id(1), id(2));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut seen: std::collections::HashSet<usize> = Default::default();
        let mut keys: Vec<usize> = adj.keys().copied().collect();
        keys.sort_unstable();
        let mut loops = vec![];
        for start in keys {
            if seen.contains(&start) {
                continue;
            }
            let mut lp = vec![];
            let (mut prev, mut cur) = (usize::MAX, start);
            loop {
                seen.insert(cur);
                lp.push(point[&cur]);
                let nb = &adj[&cur];
                let next = if nb[0] != prev { nb[0] } else { *nb.get(1).unwrap_or(&nb[0]) };
                prev = cur;
                cur = next;
                if cur == start || seen.contains(&cur) {
                    break;
                }
            }
            lp.dedup_by(|a, b| a.distance_squared(*b) < 1e-10);
            while lp.len() > 1 && lp[0].distance_squared(*lp.last().unwrap()) < 1e-10 {
                lp.pop();
            }
            if lp.len() >= 3 {
                let s = simplify_closed(&lp, self.cell * 0.25);
                if s.len() >= 3 && area(&s).abs() > self.cell * self.cell * 2.0 {
                    loops.push(s);
                }
            }
        }
        loops
    }
}

pub fn area(p: &[DVec2]) -> f64 {
    let mut a = 0.0;
    for k in 0..p.len() {
        let (u, v) = (p[k], p[(k + 1) % p.len()]);
        a += u.x * v.y - v.x * u.y;
    }
    a * 0.5
}

fn dp(p: &[DVec2], tol: f64, keep: &mut [bool], a: usize, b: usize) {
    if b <= a + 1 {
        return;
    }
    let (pa, pb) = (p[a], p[b]);
    let d = pb - pa;
    let len = d.length().max(1e-12);
    let (mut best, mut bi) = (0.0, a);
    for (k, q) in p.iter().enumerate().take(b).skip(a + 1) {
        let dist = ((*q - pa).perp_dot(d) / len).abs();
        if dist > best {
            best = dist;
            bi = k;
        }
    }
    if best > tol {
        keep[bi] = true;
        dp(p, tol, keep, a, bi);
        dp(p, tol, keep, bi, b);
    }
}

/// Douglas–Peucker for a closed loop.
pub fn simplify_closed(p: &[DVec2], tol: f64) -> Vec<DVec2> {
    let n = p.len();
    if n < 8 {
        return p.to_vec();
    }
    // split at the point farthest from p[0]
    let far = (0..n).max_by(|a, b| p[*a].distance_squared(p[0]).total_cmp(&p[*b].distance_squared(p[0]))).unwrap();
    let mut q: Vec<DVec2> = p.to_vec();
    q.push(p[0]);
    let mut keep = vec![false; q.len()];
    keep[0] = true;
    keep[far] = true;
    keep[n] = true;
    dp(&q, tol, &mut keep, 0, far);
    dp(&q, tol, &mut keep, far, n);
    (0..n).filter(|k| keep[*k]).map(|k| q[k]).collect()
}

fn cell_for(ext: DVec2) -> f64 {
    (ext.x.max(ext.y) / 700.0).clamp(0.06, 0.4)
}

// ------------------------------------------------------------ keychain

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ring {
    None,
    Left,
    Top,
}

pub struct KeychainOptions<'a> {
    pub text: &'a str,
    pub font: FontChoice,
    pub letter_h: f64,
    pub border: f64,
    pub thick: f64,
    pub raise: f64,
    pub ring: Ring,
    pub text_color: u8,
}

/// Text outlines scaled so the text is `h` mm tall, bottom-left at the origin.
fn text_mm(text: &str, font: FontChoice, h: f64) -> Vec<Vec<DVec2>> {
    let groups = text_polygons(text, font, 0.0);
    let all: Vec<DVec2> = groups.iter().flatten().flatten().copied().collect();
    if all.is_empty() {
        return vec![];
    }
    let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in &all {
        mn = mn.min(*p);
        mx = mx.max(*p);
    }
    let s = h / (mx.y - mn.y).max(1e-9);
    groups.into_iter().flatten().map(|c| c.into_iter().map(|p| (p - mn) * s).collect()).collect()
}

/// A name keychain: a rounded tag that follows the letters, a key-ring hole and the
/// name raised on top in a second colour.
pub fn keychain(o: &KeychainOptions) -> Mesh {
    let text = if o.text.trim().is_empty() { "NAME" } else { o.text };
    let h = o.letter_h.clamp(4.0, 80.0);
    let letters = text_mm(text, o.font, h);
    if letters.is_empty() {
        return Mesh::new();
    }
    let border = o.border.clamp(1.0, 12.0);
    let (ri, wall) = (2.75, 3.0);
    let ro = ri + wall;
    let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in letters.iter().flatten() {
        mn = mn.min(*p);
        mx = mx.max(*p);
    }
    let ring_c = match o.ring {
        Ring::None => None,
        Ring::Left => Some(DVec2::new(mn.x - border - ri + 0.6, (mn.y + mx.y) / 2.0)),
        Ring::Top => Some(DVec2::new(mn.x + (mx.x - mn.x).min(h) * 0.5, mx.y + border + ri - 0.6)),
    };
    let ext = mx - mn + DVec2::splat(2.0 * (border + 2.0 * ro));
    let cell = cell_for(ext);
    let mut f = Field::of_polygons(&letters, border + 2.0 * ro + 1.0, cell);
    if let Some(c) = ring_c {
        for j in 0..f.ny {
            for i in 0..f.nx {
                let d = f.pos(i, j).distance(c);
                let k = j * f.nx + i;
                f.v[k] = (f.v[k] - border).min(d - ro).max(ri - d) + border;
            }
        }
    }
    let base_loops = f.contours(border);
    let thick = o.thick.clamp(1.2, 10.0);
    let raise = o.raise.clamp(0.4, 5.0);
    let base = extrude(&nest_contours(base_loops), 0.0, thick);
    let mut top = extrude(&nest_contours(letters), thick - 0.4, thick + raise);
    top.colors = vec![o.text_color; top.tris.len()];
    crate::csg::combine(&[base, top], &[]).mesh
}

// ------------------------------------------------------------ cookie cutter

pub struct CutterOptions {
    pub wall: f64,
    pub height: f64,
    pub flange: f64,
}

/// A cookie cutter whose cutting edge follows `outline` (the cookie's shape). The
/// outline is scaled so the whole cutter fits `w` × `d` mm.
pub fn cookie_cutter(outline: &[DVec2], w: f64, d: f64, o: &CutterOptions) -> Mesh {
    if outline.len() < 3 {
        return Mesh::new();
    }
    let wall = o.wall.clamp(0.6, 3.0);
    let flange = o.flange.clamp(0.0, 10.0);
    let height = o.height.clamp(3.0, 60.0);
    let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in outline {
        mn = mn.min(*p);
        mx = mx.max(*p);
    }
    let grow = wall + flange;
    let ext = (mx - mn).max(DVec2::splat(1e-6));
    let sx = ((w - 2.0 * grow) / ext.x).max(0.05);
    let sy = ((d - 2.0 * grow) / ext.y).max(0.05);
    let pts: Vec<DVec2> = outline.iter().map(|p| (*p - (mn + mx) / 2.0) * DVec2::new(sx, sy)).collect();
    let cell = cell_for(DVec2::new(w, d)).min(wall / 4.0);
    let mut f = Field::of_polygons(&[pts], grow + 2.0, cell);
    // keep only the outside edge (a cutter has no inner bits)
    let shell = |f: &Field, t: f64| -> Vec<Vec<Vec<DVec2>>> {
        // band 0..t outside the outline: |s - t/2| - t/2 < 0
        let g = Field { nx: f.nx, ny: f.ny, origin: f.origin, cell: f.cell, v: f.v.iter().map(|s| (s - t / 2.0).abs() - t / 2.0).collect() };
        nest_contours(g.contours(0.0))
    };
    // fill holes in the outline: everything inside counts as inside
    let outer = f.contours(0.0);
    if outer.len() > 1 {
        let groups = nest_contours(outer);
        let outs: Vec<Vec<DVec2>> = groups.into_iter().map(|g| g[0].clone()).collect();
        f = Field::of_polygons(&outs, grow + 2.0, cell);
    }
    let blade = extrude(&shell(&f, wall), 0.0, height);
    if flange <= 0.0 {
        return blade;
    }
    let lip = extrude(&shell(&f, grow), 0.0, 1.6_f64.min(height));
    crate::csg::combine(&[blade, lip], &[]).mesh
}

/// The drawn outline with its rounded corners worked out.
pub fn cutter_outline(pts: &[[f64; 2]], round: &[bool]) -> Vec<[f64; 2]> {
    if round.iter().any(|r| *r) && round.len() == pts.len() {
        crate::sketch::outline(pts, round).iter().map(|p| [p.x, p.y]).collect()
    } else {
        pts.to_vec()
    }
}

pub fn cutter_natural_size(outline: &[[f64; 2]], o: &CutterOptions) -> glam::DVec3 {
    let (mut mn, mut mx) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in outline {
        for k in 0..2 {
            mn[k] = mn[k].min(p[k]);
            mx[k] = mx[k].max(p[k]);
        }
    }
    let g = 2.0 * (o.wall + o.flange);
    glam::DVec3::new(mx[0] - mn[0] + g, mx[1] - mn[1] + g, o.height)
}

// ------------------------------------------------------------ tracing

/// Outline of the biggest dark (or light, if `invert`) shape in a picture, `size` mm
/// across, as sketch points.
pub fn trace_picture(pic: &crate::imaging::Picture, invert: bool, size: f64) -> Option<Vec<[f64; 2]>> {
    let scale = 400.0 / pic.w.max(pic.h) as f64;
    let (nx, ny) = (((pic.w as f64 * scale) as usize).max(8), ((pic.h as f64 * scale) as usize).max(8));
    let pad = 3;
    let (gx, gy) = (nx + 2 * pad, ny + 2 * pad);
    let lum: Vec<f64> = (0..nx * ny)
        .map(|k| {
            let (i, j) = (k % nx, k / nx);
            let [r, g, b, a] = pic.sample(i as f64 / (nx - 1) as f64, j as f64 / (ny - 1) as f64);
            let l = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            let a = a / 255.0;
            l * a + (1.0 - a)
        })
        .collect();
    // Otsu threshold
    let mut hist = [0usize; 64];
    for l in &lum {
        hist[((l * 63.0).round() as usize).min(63)] += 1;
    }
    let total = lum.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, h)| i as f64 * *h as f64).sum();
    let (mut wb, mut sb, mut best, mut thr) = (0.0, 0.0, 0.0, 32usize);
    for (t, h) in hist.iter().enumerate() {
        wb += *h as f64;
        if wb == 0.0 || wb == total {
            continue;
        }
        sb += t as f64 * *h as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / (total - wb));
        let between = wb * (total - wb) * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            thr = t;
        }
    }
    let thr = (thr as f64 + 0.5) / 63.0;
    // contour the (lightly blurred) brightness itself: smooth, sub-pixel edges
    let mut f = Field { nx: gx, ny: gy, origin: DVec2::ZERO, cell: 1.0, v: vec![1.0; gx * gy] };
    let at = |i: isize, j: isize| lum[(j.clamp(0, ny as isize - 1) as usize) * nx + i.clamp(0, nx as isize - 1) as usize];
    for j in 0..ny {
        for i in 0..nx {
            let (ii, jj) = (i as isize, j as isize);
            let mut s = 0.0;
            for (dx, dy, w) in [(0, 0, 4.0), (1, 0, 2.0), (-1, 0, 2.0), (0, 1, 2.0), (0, -1, 2.0), (1, 1, 1.0), (-1, 1, 1.0), (1, -1, 1.0), (-1, -1, 1.0)] {
                s += w * at(ii + dx, jj + dy);
            }
            let d = s / 16.0 - thr;
            // picture rows run downwards; flip so the outline isn't upside down
            f.v[(gy - 1 - (j + pad)) * gx + i + pad] = if invert { -d } else { d };
        }
    }
    let loops = f.contours(0.0);
    let biggest = loops.into_iter().max_by(|a, b| area(a).abs().total_cmp(&area(b).abs()))?;
    let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in &biggest {
        mn = mn.min(*p);
        mx = mx.max(*p);
    }
    let s = size / (mx - mn).max_element().max(1e-6);
    let pts: Vec<DVec2> = biggest.iter().map(|p| (*p - (mn + mx) / 2.0) * s).collect();
    // about 1/120 of the size: smooth, but few enough points to edit
    let mut tol = size / 250.0;
    let mut out = simplify_closed(&pts, tol);
    while out.len() > 160 {
        tol *= 1.4;
        out = simplify_closed(&pts, tol);
    }
    if area(&out) < 0.0 {
        out.reverse();
    }
    Some(out.iter().map(|p| [(p.x * 10.0).round() / 10.0, (p.y * 10.0).round() / 10.0]).collect())
}

/// A heart, used as the starting cookie cutter outline.
pub fn default_cutter_outline() -> Vec<[f64; 2]> {
    (0..48)
        .map(|k| {
            let t = std::f64::consts::TAU * k as f64 / 48.0;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            [(x * 2.0 * 10.0).round() / 10.0, (y * 2.0 * 10.0).round() / 10.0]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_solid(m: &Mesh) {
        assert!(m.tri_count() > 20);
        assert!(m.volume() > 0.0, "volume {}", m.volume());
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let bad = edges.values().filter(|c| **c != 2).count();
        assert_eq!(bad, 0, "open or non-manifold edges");
    }

    #[test]
    fn circle_offset() {
        let c: Vec<DVec2> = (0..90).map(|k| DVec2::from_angle(k as f64 * std::f64::consts::TAU / 90.0) * 10.0).collect();
        let f = Field::of_polygons(&[c], 5.0, 0.1);
        let l = f.contours(2.0);
        assert_eq!(l.len(), 1);
        let r: f64 = l[0].iter().map(|p| p.length()).sum::<f64>() / l[0].len() as f64;
        assert!((r - 12.0).abs() < 0.1, "{r}");
    }

    #[test]
    fn keychain_is_solid() {
        let m = keychain(&KeychainOptions { text: "Robert", font: FontChoice::Sans, letter_h: 12.0, border: 3.0, thick: 3.0, raise: 1.5, ring: Ring::Left, text_color: 1 });
        check_solid(&m);
        assert!(m.used_colors().contains(&1));
        let b = m.bounds();
        assert!((b.max.z - 4.5).abs() < 0.01);
    }

    #[test]
    fn trace_finds_the_shape() {
        // a dark disc with two ears on white, plus a small speck that must be ignored
        let (w, h) = (300u32, 250u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let p = DVec2::new(x as f64, y as f64);
                let dark = p.distance(DVec2::new(150.0, 140.0)) < 75.0 || p.distance(DVec2::new(90.0, 65.0)) < 28.0 || p.distance(DVec2::new(210.0, 65.0)) < 28.0 || p.distance(DVec2::new(20.0, 20.0)) < 5.0;
                if dark {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 3].copy_from_slice(&[10, 10, 10]);
                }
            }
        }
        let pic = crate::imaging::Picture { w, h, rgba };
        let pts = trace_picture(&pic, false, 60.0).unwrap();
        assert!(pts.len() > 12 && pts.len() <= 160, "{}", pts.len());
        let xs: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        let span = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
        assert!((span(&xs) - 60.0).abs() < 1.5, "{}", span(&xs));
        // ears are at the top: the highest point is off-centre
        let top = pts.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
        assert!(top[0].abs() > 10.0, "{top:?}");
        let pv: Vec<DVec2> = pts.iter().map(|p| DVec2::new(p[0], p[1])).collect();
        check_solid(&cookie_cutter(&pv, 70.0, 65.0, &CutterOptions { wall: 1.0, height: 14.0, flange: 3.0 }));
    }

    #[test]
    fn cutter_is_solid() {
        let o = default_cutter_outline();
        let pts: Vec<DVec2> = o.iter().map(|p| DVec2::new(p[0], p[1])).collect();
        let m = cookie_cutter(&pts, 70.0, 65.0, &CutterOptions { wall: 1.0, height: 14.0, flange: 3.0 });
        check_solid(&m);
        let s = m.bounds().size();
        assert!((s.x - 70.0).abs() < 0.5 && (s.y - 65.0).abs() < 0.5 && (s.z - 14.0).abs() < 0.01, "{s:?}");
    }
}
