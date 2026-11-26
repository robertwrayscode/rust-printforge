//! Print preview: layer-by-layer view plus rough estimates of filament, time and
//! colour changes for the Bambu Lab H2C. These are estimates, not a slicer: Bambu
//! Studio's numbers after slicing are the real ones.

use crate::mesh::Mesh;
use crate::scene::Filament;
use glam::DVec3;

pub const LAYER_H: f64 = 0.2;
const WALL_THICK: f64 = 2.0 * 0.42; // 2 walls of 0.42 mm lines
const TOPBOT_THICK: f64 = 4.0 * LAYER_H; // ~4 solid layers top and bottom
const INFILL: f64 = 0.15;
const VOL_SPEED: f64 = 12.0; // effective mm³/s including travel and slow outer walls
const LAYER_OVERHEAD_S: f64 = 1.5;
const START_S: f64 = 5.0 * 60.0; // heating, levelling, purge line
/// Vortek hotend swap incl. reload and prime tower (measured by users ~40-45 s).
const SWAP_S: f64 = 45.0;
const SWAP_WASTE_G: f64 = 0.15;
/// More than 7 filaments: some changes go through the AMS with a full purge.
const PURGE_SWAP_S: f64 = 120.0;
const PURGE_WASTE_G: f64 = 0.6;

#[derive(Clone, Debug, Default)]
pub struct Estimate {
    pub layers: usize,
    pub height: f64,
    /// (filament slot, grams)
    pub grams: Vec<(u8, f64)>,
    pub swaps: usize,
    pub waste_g: f64,
    pub time_s: f64,
    /// bitmask of filament slots printed on each layer
    pub layer_colors: Vec<u64>,
    pub swaps_per_layer: Vec<u16>,
    pub colors_used: usize,
}

pub fn density(material: &str) -> f64 {
    let m = material.to_ascii_uppercase();
    if m.contains("PETG") {
        1.27
    } else if m.contains("ABS") || m.contains("ASA") {
        1.05
    } else if m.contains("TPU") {
        1.21
    } else if m.contains("PC") {
        1.20
    } else if m.contains("PA") || m.contains("NYLON") {
        1.14
    } else {
        1.24
    }
}

/// `objects`: world-space meshes as they will print (colours = filament slots).
pub fn estimate(objects: &[Mesh], fils: &[Filament]) -> Estimate {
    let mut vol = vec![0.0f64; 64];
    let mut zmax = 0.0f64;
    for m in objects {
        zmax = zmax.max(m.bounds().max.z);
    }
    let layers = ((zmax / LAYER_H).ceil() as usize).max(1);
    let mut layer_colors = vec![0u64; layers];
    let mut total_print = 0.0;
    for m in objects {
        if m.tris.is_empty() {
            continue;
        }
        // shell volume per colour, from the painted surface
        let mut shell = vec![0.0f64; 64];
        let mut area_by = vec![0.0f64; 64];
        for t in 0..m.tris.len() {
            let (n, a) = m.tri_normal_area(t);
            let c = (m.colors[t] as usize).min(63);
            let th = if n.z.abs() > 0.6 { TOPBOT_THICK } else { WALL_THICK };
            shell[c] += a * th;
            area_by[c] += a;
        }
        let base = (0..64).max_by(|&a, &b| area_by[a].total_cmp(&area_by[b])).unwrap_or(0);
        let v = m.volume().abs();
        let shell_total: f64 = shell.iter().sum();
        let scale = if shell_total > v { v / shell_total } else { 1.0 };
        for c in 0..64 {
            vol[c] += shell[c] * scale;
        }
        let infill = (v - shell_total * scale).max(0.0) * INFILL;
        vol[base] += infill;
        total_print += shell_total * scale + infill;
        // which colours appear on which layers
        let b = m.bounds();
        let (l0, l1) = (layer_of(b.min.z, layers), layer_of(b.max.z - 1e-6, layers));
        for l in l0..=l1 {
            layer_colors[l] |= 1 << base.min(63);
        }
        for (t, tri) in m.tris.iter().enumerate() {
            let c = m.colors[t].min(63) as u64;
            if c as usize == base {
                continue;
            }
            let zs = tri.map(|i| m.verts[i as usize].z);
            let (lo, hi) = (zs[0].min(zs[1]).min(zs[2]), zs[0].max(zs[1]).max(zs[2]));
            for l in layer_of(lo, layers)..=layer_of(hi - 1e-9, layers) {
                layer_colors[l] |= 1 << c;
            }
        }
    }
    // colour changes: greedy, ending each layer on a colour the next layer also uses
    let used_mask = layer_colors.iter().fold(0u64, |a, &b| a | b);
    let colors_used = used_mask.count_ones() as usize;
    let mut swaps = 0usize;
    let mut swaps_per_layer = vec![0u16; layers];
    let mut current: Option<u32> = None;
    for l in 0..layers {
        let m = layer_colors[l];
        if m == 0 {
            continue;
        }
        let k = m.count_ones() as usize;
        let mut s = k - 1;
        let starts_on_current = current.map(|c| m & (1 << c) != 0).unwrap_or(true);
        if !starts_on_current {
            s += 1;
        }
        // finish on a colour the next layer needs, if possible
        let next = layer_colors.get(l + 1).copied().unwrap_or(0);
        let start = current.filter(|&c| m & (1 << c) != 0).unwrap_or(m.trailing_zeros());
        let rest = m & !(1 << start);
        let finish = if rest == 0 {
            start
        } else if rest & next != 0 {
            (rest & next).trailing_zeros()
        } else {
            rest.trailing_zeros()
        };
        current = Some(finish);
        swaps += s;
        swaps_per_layer[l] = s as u16;
    }
    let (swap_s, swap_g) = if colors_used > 7 { (PURGE_SWAP_S, PURGE_WASTE_G) } else { (SWAP_S, SWAP_WASTE_G) };
    let mut grams = vec![];
    for (c, v) in vol.iter().enumerate() {
        if *v > 1e-6 {
            let d = fils.get(c).map(|f| density(&f.material)).unwrap_or(1.24);
            grams.push((c as u8, v / 1000.0 * d));
        }
    }
    let time_s = START_S + total_print / VOL_SPEED + layers as f64 * LAYER_OVERHEAD_S + swaps as f64 * swap_s;
    Estimate { layers, height: zmax, grams, swaps, waste_g: swaps as f64 * swap_g, time_s, layer_colors, swaps_per_layer, colors_used }
}

fn layer_of(z: f64, layers: usize) -> usize {
    ((z / LAYER_H).floor().max(0.0) as usize).min(layers - 1)
}

pub fn fmt_time(s: f64) -> String {
    let m = (s / 60.0).round() as u64;
    if m < 60 { format!("{m} min") } else { format!("{} h {:02} min", m / 60, m % 60) }
}

/// Part of triangle (a,b,c) at or below height z (0, 1 or 2 triangles).
pub fn clip_below(p: [DVec3; 3], z: f64) -> Vec<[DVec3; 3]> {
    let below: Vec<bool> = p.iter().map(|v| v.z <= z).collect();
    let n = below.iter().filter(|&&b| b).count();
    if n == 3 {
        return vec![p];
    }
    if n == 0 {
        return vec![];
    }
    let mut poly = vec![];
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        if below[i] {
            poly.push(a);
        }
        if below[i] != below[(i + 1) % 3] {
            let t = (z - a.z) / (b.z - a.z);
            poly.push(a + (b - a) * t);
        }
    }
    (1..poly.len() - 1).map(|i| [poly[0], poly[i], poly[i + 1]]).collect()
}

/// Where a triangle crosses height z (the layer outline).
pub fn section(p: [DVec3; 3], z: f64) -> Option<(DVec3, DVec3)> {
    let mut pts = vec![];
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        if (a.z - z) * (b.z - z) < 0.0 {
            let t = (z - a.z) / (b.z - a.z);
            pts.push(a + (b - a) * t);
        }
    }
    (pts.len() == 2).then(|| (pts[0], pts[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    fn fils() -> Vec<Filament> {
        ["White", "Red", "Blue"].iter().map(|n| Filament { name: n.to_string(), color: [200, 200, 200], material: "PLA".into() }).collect()
    }

    #[test]
    fn one_colour_cube_has_no_swaps_and_sane_numbers() {
        let mut m = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        m.fill_unset_color(0);
        let e = estimate(&[m], &fils());
        assert_eq!(e.layers, 100);
        assert_eq!(e.swaps, 0);
        assert_eq!(e.grams.len(), 1);
        // 8 cm³ solid = 9.9 g; with 15% infill and shells it should be ~3-6 g
        assert!(e.grams[0].1 > 2.5 && e.grams[0].1 < 7.0, "{}", e.grams[0].1);
        assert!(e.time_s > 300.0 && e.time_s < 3600.0);
    }

    #[test]
    fn stacked_colours_change_once() {
        let mut a = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(20.0, 20.0, 10.0));
        a.fill_unset_color(0);
        let mut b = a.clone();
        b.colors.iter_mut().for_each(|c| *c = 1);
        b.verts.iter_mut().for_each(|v| v.z += 10.0);
        let e = estimate(&[a, b], &fils());
        assert_eq!(e.colors_used, 2);
        assert!(e.swaps <= 2, "{}", e.swaps);
    }

    #[test]
    fn clipping_keeps_the_part_below() {
        let t = [DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 10.0)];
        let c = clip_below(t, 5.0);
        let area: f64 = c.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0).sum();
        assert!((area - 37.5).abs() < 1e-9);
        assert!(section(t, 5.0).is_some());
    }
}
