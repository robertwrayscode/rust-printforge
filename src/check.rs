//! Printability analysis. Runs on world-space meshes off the UI thread.

use crate::mesh::{Aabb, Bvh, Mesh, ray_tri};
use crate::scene::Printer;
use glam::DVec3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Object id this applies to (if any).
    pub object: Option<u64>,
    /// Triangles (in that object's exported world mesh) to highlight.
    pub tris: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub issues: Vec<Issue>,
    pub summary: Vec<String>,
}

pub struct CheckInput {
    pub objects: Vec<(u64, String, Mesh)>,
    pub loose_holes: Vec<String>,
    pub hidden: Vec<String>,
    pub csg_warnings: Vec<String>,
    pub filament_names: Vec<String>,
    pub printer: Printer,
    pub overhang_deg: f64,
}

fn push(r: &mut Report, sev: Severity, obj: Option<u64>, title: &str, detail: String, tris: Vec<u32>) {
    r.issues.push(Issue { severity: sev, title: title.into(), detail, object: obj, tris });
}

pub fn run(input: &CheckInput) -> Report {
    let mut r = Report::default();
    let p = &input.printer;
    if input.objects.is_empty() {
        push(&mut r, Severity::Error, None, "Nothing to print", "Add at least one visible solid shape.".into(), vec![]);
        return r;
    }

    let per: Vec<Report> = input.objects.par_iter().map(|(id, name, m)| check_object(*id, name, m, input)).collect();
    for x in per {
        r.issues.extend(x.issues);
        r.summary.extend(x.summary);
    }

    // ----- scene-level checks
    let mut all = Aabb::empty();
    for (_, _, m) in &input.objects {
        all = all.union(&m.bounds());
    }
    let mut used: Vec<u8> = input.objects.iter().flat_map(|(_, _, m)| m.used_colors()).collect();
    used.sort_unstable();
    used.dedup();
    let multi = used.len() > 1;
    let bed = if multi { p.bed_multi } else { p.bed };
    let (hx, hy) = (bed[0] / 2.0, bed[1] / 2.0);
    for (id, name, m) in &input.objects {
        let b = m.bounds();
        if b.min.x < -hx - 1e-6 || b.max.x > hx + 1e-6 || b.min.y < -hy - 1e-6 || b.max.y > hy + 1e-6 {
            let fits = b.size().x <= bed[0] && b.size().y <= bed[1];
            push(&mut r, if fits { Severity::Warning } else { Severity::Error }, Some(*id), "Outside the print area",
                format!("'{name}' extends past the {:.0} x {:.0} mm {} area of the {}. {}",
                    bed[0], bed[1], if multi { "multi-colour (dual-nozzle)" } else { "single-nozzle" }, p.name,
                    if fits { "Move it toward the centre (or use Arrange)." } else { "It's too big — scale it down or split it into parts." }), vec![]);
        }
        if b.max.z > p.height + 1e-6 {
            push(&mut r, Severity::Error, Some(*id), "Too tall",
                format!("'{name}' is {:.1} mm tall; the {} prints up to {:.0} mm.", b.max.z, p.name, p.height), vec![]);
        }
    }
    if used.len() > p.max_filaments {
        push(&mut r, Severity::Error, None, "Too many colours",
            format!("{} filaments are used; the {} with AMS units supports at most {}.", used.len(), p.name, p.max_filaments), vec![]);
    } else if used.len() > p.nozzle_slots {
        push(&mut r, Severity::Warning, None, "More colours than hotends",
            format!("{} filaments are used. The H2C holds {} filaments in hotends (1 left + 6 Vortek); beyond that it must purge to swap filament, which adds waste and time.", used.len(), p.nozzle_slots), vec![]);
    }
    // overlapping separate objects
    for i in 0..input.objects.len() {
        for j in i + 1..input.objects.len() {
            let (ia, na, ma) = &input.objects[i];
            let (_, nb, mb) = &input.objects[j];
            let (ba, bb) = (ma.bounds(), mb.bounds());
            if !ba.overlaps(&bb, 1e-4) {
                continue;
            }
            if meshes_intersect(ma, mb) || point_inside(mb, ma.verts.first().copied().unwrap_or_default()) || point_inside(ma, mb.verts.first().copied().unwrap_or_default()) {
                push(&mut r, Severity::Warning, Some(*ia), "Separate objects overlap",
                    format!("'{na}' and '{nb}' overlap but aren't grouped. The slicer will print overlapping regions unpredictably (and pick one colour). Select both and Group them."), vec![]);
            }
        }
    }
    for h in &input.loose_holes {
        push(&mut r, Severity::Warning, None, "Hole isn't cutting anything",
            format!("'{h}' is a hole but isn't grouped with a solid, so it has no effect and won't be exported. Select it with the shape it should cut and press Group."), vec![]);
    }
    for h in &input.hidden {
        push(&mut r, Severity::Info, None, "Hidden object not exported", format!("'{h}' is hidden and will be left out of the export."), vec![]);
    }
    for w in &input.csg_warnings {
        push(&mut r, Severity::Warning, None, "Boolean operation problem", w.clone(), vec![]);
    }
    let names: Vec<String> = used.iter().map(|&c| input.filament_names.get(c as usize).cloned().unwrap_or(format!("#{}", c + 1))).collect();
    r.summary.insert(0, format!("Overall size: {:.1} x {:.1} x {:.1} mm", all.size().x, all.size().y, all.size().z));
    r.summary.insert(1, format!("Filaments used ({}): {}", used.len(), names.join(", ")));
    r.issues.sort_by_key(|i| i.severity);
    if !r.issues.iter().any(|i| i.severity != Severity::Info) {
        r.summary.insert(0, "No problems found — ready to print.".into());
    }
    r
}

fn check_object(id: u64, name: &str, m: &Mesh, input: &CheckInput) -> Report {
    let mut r = Report::default();
    let p = &input.printer;
    let o = Some(id);
    let n = m.tris.len();
    if n == 0 {
        push(&mut r, Severity::Error, o, "Empty object", format!("'{name}' has no geometry (a hole may have removed everything)."), vec![]);
        return r;
    }
    let b = m.bounds();

    // ---- topology
    let mut directed: HashMap<(u32, u32), u32> = HashMap::with_capacity(n * 3);
    for t in &m.tris {
        for k in 0..3 {
            *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    let mut open = Vec::new();
    let mut nonman = Vec::new();
    let mut flipped = Vec::new();
    for (ti, t) in m.tris.iter().enumerate() {
        for k in 0..3 {
            let (a, c) = (t[k], t[(k + 1) % 3]);
            let fwd = directed[&(a, c)];
            let back = directed.get(&(c, a)).copied().unwrap_or(0);
            if fwd + back > 2 {
                nonman.push(ti as u32);
            } else if fwd > 1 {
                flipped.push(ti as u32);
            } else if back == 0 {
                open.push(ti as u32);
            }
        }
    }
    for v in [&mut open, &mut nonman, &mut flipped] {
        v.sort_unstable();
        v.dedup();
    }
    let watertight = open.is_empty() && nonman.is_empty() && flipped.is_empty();
    if !open.is_empty() {
        push(&mut r, Severity::Error, o, "Holes in the surface (not watertight)",
            format!("'{name}' has {} triangles along open edges. Slicers have to guess what's inside, which can drop walls or fill gaps wrongly. Imported meshes are the usual cause — re-export from the source program or use Repair.", open.len()), open);
    }
    if !nonman.is_empty() {
        push(&mut r, Severity::Error, o, "Non-manifold edges",
            format!("'{name}' has {} triangles on edges shared by more than two faces (e.g. two parts touching only along an edge). Thicken the connection or separate them slightly.", nonman.len()), nonman);
    }
    if !flipped.is_empty() {
        push(&mut r, Severity::Error, o, "Inconsistent face orientation",
            format!("'{name}' has {} triangles facing the wrong way relative to their neighbours.", flipped.len()), flipped);
    }

    // degenerate & duplicate faces
    let degenerate: Vec<u32> = (0..n).filter(|&t| m.tri_normal_area(t).1 < 1e-8).map(|t| t as u32).collect();
    if !degenerate.is_empty() {
        push(&mut r, Severity::Info, o, "Zero-area triangles",
            format!("{} degenerate triangles in '{name}' (harmless; slicers drop them).", degenerate.len()), degenerate);
    }
    let mut seen = HashSet::new();
    let dups: Vec<u32> = m.tris.iter().enumerate().filter(|(_, t)| {
        let mut s = **t;
        s.sort_unstable();
        !seen.insert(s)
    }).map(|(i, _)| i as u32).collect();
    if !dups.is_empty() {
        push(&mut r, Severity::Warning, o, "Duplicate faces", format!("'{name}' has {} duplicated triangles.", dups.len()), dups);
    }

    let vol = m.volume();
    if watertight && vol < 0.0 {
        push(&mut r, Severity::Error, o, "Inside-out mesh", format!("'{name}' has its normals pointing inward (negative volume). Use Repair to flip it."), (0..n as u32).collect());
    }

    // ---- size
    let s = b.size();
    let min_feature = p.nozzle * 2.0;
    if s.x < min_feature || s.y < min_feature {
        push(&mut r, Severity::Error, o, "Too small to print",
            format!("'{name}' is only {:.2} x {:.2} mm across; with a {} mm nozzle anything under ~{:.1} mm won't form.", s.x, s.y, p.nozzle, min_feature), vec![]);
    }
    if s.z < p.layer {
        push(&mut r, Severity::Error, o, "Thinner than one layer", format!("'{name}' is {:.2} mm tall, less than a {} mm layer.", s.z, p.layer), vec![]);
    }
    if s.max_element() > 0.0 && s.max_element() < 3.0 {
        push(&mut r, Severity::Info, o, "Very small model",
            format!("'{name}' is under 3 mm overall. If you expected something bigger, the units may be wrong (inches/metres) — scale it up."), vec![]);
    }

    // ---- bed placement & adhesion
    if b.min.z < -0.01 {
        push(&mut r, Severity::Warning, o, "Below the build plate",
            format!("'{name}' goes {:.2} mm under the plate; that part will be cut off. Use Drop to bed if that isn't intended.", -b.min.z), vec![]);
    } else if b.min.z > 0.01 {
        push(&mut r, Severity::Error, o, "Floating above the build plate",
            format!("'{name}' starts {:.2} mm above the plate. Use Drop to bed (D).", b.min.z), vec![]);
    }
    let minz = b.min.z;
    let mut contact = 0.0;
    for t in 0..n {
        let (nn, a) = m.tri_normal_area(t);
        if nn.z < -0.995 && m.tri_points(t).iter().all(|q| q.z < minz + 0.05) {
            contact += a;
        }
    }
    if contact < 1e-6 {
        push(&mut r, Severity::Warning, o, "No flat base",
            format!("'{name}' touches the plate only at a point or edge. Add a flat base, lay it flat on a face, or enable supports + brim."), vec![]);
    } else {
        if contact < 25.0 {
            push(&mut r, Severity::Warning, o, "Small contact with the plate",
                format!("Only {contact:.1} mm² of '{name}' touches the plate. It may come loose — consider a brim (Bambu Studio: Others > Brim)."), vec![]);
        }
        let ratio = s.z / contact.sqrt();
        if ratio > 8.0 && s.z > 30.0 {
            push(&mut r, Severity::Warning, o, "Tall and narrow",
                format!("'{name}' is {:.0} mm tall on a small base (height/base ≈ {ratio:.0}). It may wobble or snap at high speed; add a brim or print it lying down.", s.z), vec![]);
        }
    }

    // ---- shells
    let (comp, ncomp) = m.components();
    if ncomp > 1 {
        let mut shell_b = vec![Aabb::empty(); ncomp];
        let mut shell_v = vec![0.0; ncomp];
        for (t, &c) in comp.iter().enumerate() {
            for q in m.tri_points(t) {
                shell_b[c as usize].grow(q);
            }
            let [a, bb, cc] = m.tri_points(t);
            shell_v[c as usize] += a.dot(bb.cross(cc)) / 6.0;
        }
        let mut floating = Vec::new();
        let mut cavities = 0;
        for c in 0..ncomp {
            if shell_v[c] < 0.0 {
                cavities += 1;
                continue;
            }
            if shell_b[c].min.z > minz + 0.1 {
                // supported if it sits inside / on another shell
                let probe = DVec3::new(shell_b[c].center().x, shell_b[c].center().y, shell_b[c].min.z - 0.05);
                let inside_other = (0..ncomp).any(|o2| o2 != c && shell_v[o2] > 0.0 && shell_b[o2].min.z <= probe.z && shell_b[o2].max.z >= probe.z
                    && shell_b[o2].min.x <= probe.x && shell_b[o2].max.x >= probe.x && shell_b[o2].min.y <= probe.y && shell_b[o2].max.y >= probe.y);
                if !inside_other {
                    floating.extend(comp.iter().enumerate().filter(|(_, x)| **x as usize == c).map(|(t, _)| t as u32));
                }
            }
        }
        if !floating.is_empty() {
            push(&mut r, Severity::Error, o, "Part floating in mid-air",
                format!("'{name}' has a separate piece that isn't connected to anything below it. It will print onto nothing unless supports are enabled — connect it or move it down."), floating);
        }
        if cavities > 0 {
            push(&mut r, Severity::Info, o, "Sealed internal cavity",
                format!("'{name}' contains {cavities} fully enclosed hollow space(s). They'll print hollow (and can trap support material)."), vec![]);
        }
        r.summary.push(format!("'{name}': {ncomp} separate shells"));
    }

    // ---- overhangs and islands
    let cos_limit = (90.0 - input.overhang_deg).to_radians().cos(); // n.z below -this = too steep
    let mut over = Vec::new();
    let mut bridge = Vec::new();
    let (mut over_a, mut bridge_a) = (0.0, 0.0);
    for t in 0..n {
        let (nn, a) = m.tri_normal_area(t);
        if a < 1e-9 || m.tri_points(t).iter().all(|q| q.z < minz + 0.1) {
            continue;
        }
        if nn.z < -0.985 {
            bridge.push(t as u32);
            bridge_a += a;
        } else if nn.z < -cos_limit {
            over.push(t as u32);
            over_a += a;
        }
    }
    if over_a > 4.0 {
        push(&mut r, Severity::Warning, o, "Steep overhangs",
            format!("{over_a:.0} mm² of '{name}' leans out more than {:.0}° from vertical. Enable supports in Bambu Studio (Support > Enable, type tree(auto)) or rotate/chamfer those areas.", input.overhang_deg), over);
    }
    if bridge_a > 4.0 {
        push(&mut r, Severity::Info, o, "Flat downward faces (bridges)",
            format!("{bridge_a:.0} mm² of '{name}' faces straight down above the plate. Short spans bridge fine; wide ones (> ~10 mm) need supports."), bridge);
    }
    // islands: local minimum vertices above the bed
    let mut nbr: Vec<Vec<u32>> = vec![vec![]; m.verts.len()];
    for t in &m.tris {
        for k in 0..3 {
            nbr[t[k] as usize].push(t[(k + 1) % 3]);
        }
    }
    let islands: Vec<u32> = (0..m.verts.len())
        .filter(|&v| {
            let z = m.verts[v].z;
            z > minz + 0.2 && !nbr[v].is_empty() && nbr[v].iter().all(|&w| m.verts[w as usize].z > z + 1e-6)
        })
        .map(|v| v as u32)
        .collect();
    if !islands.is_empty() {
        let set: HashSet<u32> = islands.iter().copied().collect();
        let tris: Vec<u32> = m.tris.iter().enumerate().filter(|(_, t)| t.iter().any(|v| set.contains(v))).map(|(i, _)| i as u32).collect();
        push(&mut r, Severity::Warning, o, "Unsupported starting points",
            format!("'{name}' has {} point(s) where printing would begin in mid-air (lowest points of a feature that's above the plate). These need supports.", islands.len()), tris);
    }

    // ---- self-intersection
    if n <= 400_000 {
        let bvh = Bvh::build(m);
        let hits = self_intersections(m, &bvh);
        if !hits.is_empty() {
            push(&mut r, Severity::Warning, o, "Self-intersecting surface",
                format!("'{name}' has {} triangles passing through other parts of itself. Usually fine for slicers, but it can cause odd gaps. Grouping shapes (union) removes it.", hits.len()), hits);
        }
        // ---- wall thickness
        if watertight {
            let (thin, too_thin, thin_a, too_thin_a, min_t) = thickness(m, &bvh, p.nozzle);
            if too_thin_a > 0.5 {
                push(&mut r, Severity::Error, o, "Walls too thin to print",
                    format!("Parts of '{name}' are only ~{min_t:.2} mm thick — thinner than the {} mm nozzle, so they'll be missing from the print. Make them at least {:.1} mm.", p.nozzle, p.nozzle * 2.0), too_thin);
            }
            if thin_a > 1.0 {
                push(&mut r, Severity::Warning, o, "Thin walls",
                    format!("Some walls of '{name}' are under {:.1} mm (two lines of the {} mm nozzle). They'll be fragile and may print with gaps.", p.nozzle * 2.0, p.nozzle), thin);
            }
        }
    } else {
        push(&mut r, Severity::Info, o, "Detailed checks skipped", format!("'{name}' has {n} triangles; self-intersection and wall-thickness checks were skipped."), vec![]);
    }
    if n > 1_500_000 {
        push(&mut r, Severity::Warning, o, "Very heavy mesh", format!("'{name}' has {n} triangles; slicing and file transfer will be slow."), vec![]);
    }

    // ---- colour regions
    let used = m.used_colors();
    if used.len() > 1 {
        let nb = m.tri_neighbors();
        let mut region = vec![u32::MAX; n];
        let mut specks = Vec::new();
        let mut nreg = 0;
        for start in 0..n {
            if region[start] != u32::MAX {
                continue;
            }
            let col = m.colors[start];
            let mut stack = vec![start as u32];
            let mut members = Vec::new();
            let mut area = 0.0;
            region[start] = nreg;
            while let Some(t) = stack.pop() {
                members.push(t);
                area += m.tri_normal_area(t as usize).1;
                for &x in &nb[t as usize] {
                    if x != u32::MAX && region[x as usize] == u32::MAX && m.colors[x as usize] == col {
                        region[x as usize] = nreg;
                        stack.push(x);
                    }
                }
            }
            nreg += 1;
            if area < 1.0 {
                specks.extend(members);
            }
        }
        if !specks.is_empty() {
            push(&mut r, Severity::Warning, o, "Tiny colour specks",
                format!("'{name}' has colour areas under 1 mm² — too small to show up, and each adds filament swaps. Paint larger areas or erase them."), specks);
        }
    }

    let solid = vol.abs() / 1000.0;
    r.summary.push(format!("'{name}': {:.1} x {:.1} x {:.1} mm, {:.1} cm³ (~{:.0} g PLA if solid), {} triangles", s.x, s.y, s.z, solid, solid * 1.24, n));
    r
}

/// Segment/triangle based intersection between triangles that don't share vertices.
fn tris_intersect(a: [DVec3; 3], b: [DVec3; 3]) -> bool {
    let seg = |p: DVec3, q: DVec3, t: [DVec3; 3]| -> bool {
        let d = q - p;
        let len = d.length();
        if len < 1e-12 {
            return false;
        }
        match ray_tri(p, d / len, t[0], t[1], t[2]) {
            Some(x) => x > 1e-7 && x < len - 1e-7,
            None => false,
        }
    };
    for k in 0..3 {
        if seg(a[k], a[(k + 1) % 3], b) || seg(b[k], b[(k + 1) % 3], a) {
            return true;
        }
    }
    false
}

fn tri_box(m: &Mesh, t: usize) -> Aabb {
    let mut bb = Aabb::empty();
    for q in m.tri_points(t) {
        bb.grow(q);
    }
    bb
}

fn self_intersections(m: &Mesh, bvh: &Bvh) -> Vec<u32> {
    let mut hits: Vec<u32> = (0..m.tris.len())
        .into_par_iter()
        .filter(|&t| {
            let mut cand = Vec::new();
            bvh.query_box(&tri_box(m, t), &mut cand);
            let ta = m.tris[t];
            let pa = m.tri_points(t);
            cand.iter().any(|&c| {
                let c = c as usize;
                if c == t {
                    return false;
                }
                let tb = m.tris[c];
                if ta.iter().any(|v| tb.contains(v)) {
                    return false;
                }
                tris_intersect(pa, m.tri_points(c))
            })
        })
        .map(|t| t as u32)
        .collect();
    hits.sort_unstable();
    hits
}

fn meshes_intersect(a: &Mesh, b: &Mesh) -> bool {
    let bvh = Bvh::build(b);
    (0..a.tris.len()).into_par_iter().any(|t| {
        let mut cand = Vec::new();
        bvh.query_box(&tri_box(a, t), &mut cand);
        let pa = a.tri_points(t);
        cand.iter().any(|&c| tris_intersect(pa, b.tri_points(c as usize)))
    })
}

/// Ray-parity point-in-mesh test.
pub fn point_inside(m: &Mesh, p: DVec3) -> bool {
    let d = DVec3::new(0.5773, 0.5774, 0.5773).normalize();
    let mut count = 0;
    for t in 0..m.tris.len() {
        let [a, b, c] = m.tri_points(t);
        if ray_tri(p, d, a, b, c).is_some() {
            count += 1;
        }
    }
    count % 2 == 1
}

/// Measure wall thickness by casting rays inward from each face.
/// Returns (thin tris, too-thin tris, thin area, too-thin area, min thickness).
fn thickness(m: &Mesh, bvh: &Bvh, nozzle: f64) -> (Vec<u32>, Vec<u32>, f64, f64, f64) {
    let n = m.tris.len();
    let step = (n / 60_000).max(1);
    let res: Vec<(u32, f64, f64)> = (0..n)
        .into_par_iter()
        .step_by(step)
        .filter_map(|t| {
            let (nn, a) = m.tri_normal_area(t);
            if a < 1e-6 {
                return None;
            }
            let o = m.tri_centroid(t) - nn * 1e-5;
            let (d, hit) = bvh.raycast(m, o, -nn, Some(t as u32))?;
            let (hn, _) = m.tri_normal_area(hit as usize);
            if hn.dot(nn) > -0.3 {
                return None; // grazing / not an opposite wall
            }
            Some((t as u32, d, a))
        })
        .collect();
    let mut thin = Vec::new();
    let mut too = Vec::new();
    let (mut ta, mut tta, mut mn) = (0.0, 0.0, f64::INFINITY);
    for (t, d, a) in res {
        if d < nozzle * 0.95 {
            too.push(t);
            tta += a * step as f64;
            mn = mn.min(d);
        } else if d < nozzle * 2.0 - 1e-6 {
            thin.push(t);
            ta += a * step as f64;
        }
    }
    (thin, too, ta, tta, mn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    fn input(objs: Vec<Mesh>) -> CheckInput {
        CheckInput {
            objects: objs.into_iter().enumerate().map(|(i, m)| (i as u64, format!("o{i}"), m)).collect(),
            loose_holes: vec![],
            hidden: vec![],
            csg_warnings: vec![],
            filament_names: vec!["a".into(), "b".into()],
            printer: Printer::default(),
            overhang_deg: 45.0,
        }
    }

    #[test]
    fn clean_box_passes() {
        let m = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        let mut m = m;
        m.set_color(0);
        let r = run(&input(vec![m]));
        let bad: Vec<_> = r.issues.iter().filter(|i| i.severity != Severity::Info).collect();
        assert!(bad.is_empty(), "{:#?}", bad);
    }

    #[test]
    fn detects_problems() {
        let mut thin = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(30.0, 0.3, 10.0));
        thin.set_color(0);
        let mut floating = Shape::Sphere { steps: 16 }.generate(DVec3::splat(10.0));
        floating.translate(DVec3::new(60.0, 0.0, 5.0));
        floating.set_color(1);
        let mut open = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(10.0));
        open.translate(DVec3::new(-60.0, 0.0, 0.0));
        open.tris.pop();
        open.colors.pop();
        let mut big = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(400.0, 10.0, 10.0));
        big.translate(DVec3::new(0.0, 100.0, 0.0));
        big.set_color(0);
        let post = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(10.0, 10.0, 20.0));
        let mut ball = Shape::Sphere { steps: 16 }.generate(DVec3::splat(8.0));
        ball.translate(DVec3::new(8.0, 0.0, 11.0));
        let mut hang = crate::csg::combine(&[post, ball], &[]).mesh;
        hang.translate(DVec3::new(0.0, -60.0, 0.0));
        hang.set_color(0);
        let r = run(&input(vec![thin, floating, open, big, hang]));
        let titles: Vec<&str> = r.issues.iter().map(|i| i.title.as_str()).collect();
        for want in ["Walls too thin to print", "Floating above the build plate", "Holes in the surface (not watertight)", "Outside the print area", "Unsupported starting points"] {
            assert!(titles.contains(&want), "missing {want}: {titles:?}");
        }
    }

    #[test]
    fn overhang_detected() {
        // T shape: box on a narrow post -> flat downward face (bridge) and island-free
        let mut a = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(4.0, 4.0, 20.0));
        let mut top = Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::new(30.0, 30.0, 3.0));
        top.translate(DVec3::new(0.0, 0.0, 20.0));
        a.set_color(0);
        top.set_color(0);
        let u = crate::csg::combine(&[a, top], &[]).mesh;
        let r = run(&input(vec![u]));
        assert!(r.issues.iter().any(|i| i.title.contains("bridges")), "{:#?}", r.issues);
    }
}
