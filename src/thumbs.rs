//! Small software-rendered pictures of each library shape for the Shapes panel.

use crate::shapes::LIBRARY;
use glam::{DMat3, DVec3};

pub const THUMB: usize = 64;

/// Distinct Tinkercad-like colours per shape tile.
const TILE_COLORS: [[u8; 3]; 8] = [
    [226, 64, 52],
    [242, 140, 40],
    [246, 196, 48],
    [70, 170, 90],
    [50, 140, 220],
    [128, 90, 210],
    [220, 80, 150],
    [70, 180, 190],
];

/// Render shape `idx` to an RGBA image of THUMB x THUMB (2x supersampled).
pub fn render(idx: usize) -> Vec<u8> {
    let info = &LIBRARY[idx];
    let mut size = DVec3::from(info.default_size);
    let mut shape = (info.shape)();
    if let crate::shapes::Shape::Text { font, spacing, .. } = &shape {
        // a single bold letter reads better than a word at icon size
        shape = crate::shapes::Shape::Text { text: "A".into(), font: *font, spacing: *spacing };
        size = DVec3::new(14.0, 16.0, 5.0);
    }
    let m = shape.generate(size);
    let col = TILE_COLORS[idx % TILE_COLORS.len()];
    render_mesh(&m, &|_| col)
}

/// Render any mesh; `color(t)` gives triangle t's colour.
pub fn render_mesh(m: &crate::mesh::Mesh, color: &dyn Fn(usize) -> [u8; 3]) -> Vec<u8> {
    let b = m.bounds();
    let c = b.center();
    // view rotation: yaw then pitch (looking slightly down at the front-right corner)
    let rot = DMat3::from_rotation_x(-58f64.to_radians()) * DMat3::from_rotation_z(-35f64.to_radians());
    let verts: Vec<DVec3> = m.verts.iter().map(|v| rot * (*v - c)).collect();
    let (mut mn, mut mx) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for v in &verts {
        mn = mn.min(*v);
        mx = mx.max(*v);
    }
    let ss = 3; // supersampling
    let n = THUMB * ss;
    let ext = (mx.x - mn.x).max(mx.y - mn.y).max(1e-9);
    let scale = (n as f64 * 0.86) / ext;
    let mid = (mn + mx) * 0.5;
    let to_px = |v: DVec3| DVec3::new((v.x - mid.x) * scale + n as f64 / 2.0, (mid.y - v.y) * scale + n as f64 / 2.0, v.z);
    let pts: Vec<DVec3> = verts.iter().map(|v| to_px(*v)).collect();
    let mut zbuf = vec![f64::NEG_INFINITY; n * n];
    let mut rgba = vec![0u8; n * n * 4];
    let light = DVec3::new(-0.35, 0.5, 0.8).normalize();
    for (ti, t) in m.tris.iter().enumerate() {
        let col = color(ti);
        let (a, b2, c2) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
        let nrm = (verts[t[1] as usize] - verts[t[0] as usize]).cross(verts[t[2] as usize] - verts[t[0] as usize]).normalize_or_zero();
        if nrm.z <= 0.0 {
            continue; // facing away
        }
        let shade = 0.45 + 0.55 * nrm.dot(light).max(0.0);
        let px = [(col[0] as f64 * shade).min(255.0) as u8, (col[1] as f64 * shade).min(255.0) as u8, (col[2] as f64 * shade).min(255.0) as u8];
        let x0 = a.x.min(b2.x).min(c2.x).floor().max(0.0) as usize;
        let x1 = (a.x.max(b2.x).max(c2.x).ceil() as usize).min(n - 1);
        let y0 = a.y.min(b2.y).min(c2.y).floor().max(0.0) as usize;
        let y1 = (a.y.max(b2.y).max(c2.y).ceil() as usize).min(n - 1);
        let area = (b2.x - a.x) * (c2.y - a.y) - (b2.y - a.y) * (c2.x - a.x);
        if area.abs() < 1e-12 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((b2.x - fx) * (c2.y - fy) - (b2.y - fy) * (c2.x - fx)) / area;
                let w1 = ((c2.x - fx) * (a.y - fy) - (c2.y - fy) * (a.x - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let z = w0 * a.z + w1 * b2.z + w2 * c2.z;
                let i = y * n + x;
                if z > zbuf[i] {
                    zbuf[i] = z;
                    rgba[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
    }
    // downsample (premultiplied average for clean edges)
    let mut out = vec![0u8; THUMB * THUMB * 4];
    for y in 0..THUMB {
        for x in 0..THUMB {
            let (mut r, mut g, mut bl, mut al) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..ss {
                for dx in 0..ss {
                    let i = ((y * ss + dy) * n + x * ss + dx) * 4;
                    let a = rgba[i + 3] as u32;
                    r += rgba[i] as u32 * a / 255;
                    g += rgba[i + 1] as u32 * a / 255;
                    bl += rgba[i + 2] as u32 * a / 255;
                    al += a;
                }
            }
            let k = (ss * ss) as u32;
            let o = (y * THUMB + x) * 4;
            out[o..o + 4].copy_from_slice(&[(r / k) as u8, (g / k) as u8, (bl / k) as u8, (al / k) as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_shape_draws_something() {
        for i in 0..super::LIBRARY.len() {
            let img = super::render(i);
            let filled = img.chunks(4).filter(|p| p[3] > 0).count();
            assert!(filled > 300, "{} drew only {filled} px", super::LIBRARY[i].name);
        }
    }
}
