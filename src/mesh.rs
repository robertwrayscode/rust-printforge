//! Triangle mesh with one filament slot per triangle.

use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Marker for "no paint / use the object's own colour".
pub const NO_COLOR: u8 = 255;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub verts: Vec<DVec3>,
    pub tris: Vec<[u32; 3]>,
    /// Filament slot per triangle (0-based). Same length as `tris`.
    pub colors: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn empty() -> Self {
        Aabb { min: DVec3::splat(f64::INFINITY), max: DVec3::splat(f64::NEG_INFINITY) }
    }
    pub fn grow(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }
    pub fn size(&self) -> DVec3 {
        if self.is_empty() { DVec3::ZERO } else { self.max - self.min }
    }
    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }
    pub fn overlaps(&self, o: &Aabb, eps: f64) -> bool {
        self.min.x < o.max.x - eps && o.min.x < self.max.x - eps
            && self.min.y < o.max.y - eps && o.min.y < self.max.y - eps
            && self.min.z < o.max.z - eps && o.min.z < self.max.z - eps
    }
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tri_count(&self) -> usize {
        self.tris.len()
    }

    pub fn add_vert(&mut self, p: DVec3) -> u32 {
        self.verts.push(p);
        (self.verts.len() - 1) as u32
    }

    pub fn add_tri(&mut self, a: u32, b: u32, c: u32) {
        self.tris.push([a, b, c]);
        self.colors.push(NO_COLOR);
    }

    pub fn add_quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.add_tri(a, b, c);
        self.add_tri(a, c, d);
    }

    pub fn tri_points(&self, t: usize) -> [DVec3; 3] {
        let [a, b, c] = self.tris[t];
        [self.verts[a as usize], self.verts[b as usize], self.verts[c as usize]]
    }

    pub fn tri_normal_area(&self, t: usize) -> (DVec3, f64) {
        let [a, b, c] = self.tri_points(t);
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-300 { (DVec3::ZERO, 0.0) } else { (n / len, len * 0.5) }
    }

    pub fn tri_centroid(&self, t: usize) -> DVec3 {
        let [a, b, c] = self.tri_points(t);
        (a + b + c) / 3.0
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::empty();
        for t in &self.tris {
            for &i in t {
                b.grow(self.verts[i as usize]);
            }
        }
        b
    }

    /// Signed volume (positive for outward-facing triangles).
    pub fn volume(&self) -> f64 {
        let mut v = 0.0;
        for t in 0..self.tris.len() {
            let [a, b, c] = self.tri_points(t);
            v += a.dot(b.cross(c));
        }
        v / 6.0
    }

    pub fn surface_area(&self) -> f64 {
        (0..self.tris.len()).map(|t| self.tri_normal_area(t).1).sum()
    }

    pub fn transformed(&self, m: &DMat4) -> Mesh {
        let mut out = self.clone();
        for v in &mut out.verts {
            *v = m.transform_point3(*v);
        }
        if m.determinant() < 0.0 {
            out.flip();
        }
        out
    }

    pub fn flip(&mut self) {
        for t in &mut self.tris {
            t.swap(1, 2);
        }
    }

    pub fn scale_to(&mut self, s: DVec3) {
        for v in &mut self.verts {
            *v *= s;
        }
        if s.x * s.y * s.z < 0.0 {
            self.flip();
        }
    }

    pub fn translate(&mut self, d: DVec3) {
        for v in &mut self.verts {
            *v += d;
        }
    }

    pub fn append(&mut self, o: &Mesh) {
        let off = self.verts.len() as u32;
        self.verts.extend_from_slice(&o.verts);
        self.tris.extend(o.tris.iter().map(|t| [t[0] + off, t[1] + off, t[2] + off]));
        self.colors.extend_from_slice(&o.colors);
    }

    pub fn set_color(&mut self, c: u8) {
        for x in &mut self.colors {
            *x = c;
        }
    }

    /// Replace NO_COLOR triangles with `c`.
    pub fn fill_unset_color(&mut self, c: u8) {
        for x in &mut self.colors {
            if *x == NO_COLOR {
                *x = c;
            }
        }
    }

    /// Merge coincident vertices and drop triangles that become degenerate
    /// (repeated vertex). Keeps triangle colours aligned.
    pub fn weld(&mut self, eps: f64) {
        let inv = 1.0 / eps;
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut remap = vec![0u32; self.verts.len()];
        let mut nv = Vec::with_capacity(self.verts.len());
        for (i, v) in self.verts.iter().enumerate() {
            let key = ((v.x * inv).round() as i64, (v.y * inv).round() as i64, (v.z * inv).round() as i64);
            let id = *map.entry(key).or_insert_with(|| {
                nv.push(*v);
                (nv.len() - 1) as u32
            });
            remap[i] = id;
        }
        self.verts = nv;
        let mut tris = Vec::with_capacity(self.tris.len());
        let mut cols = Vec::with_capacity(self.tris.len());
        for (t, c) in self.tris.iter().zip(&self.colors) {
            let t = [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                tris.push(t);
                cols.push(*c);
            }
        }
        self.tris = tris;
        self.colors = cols;
    }

    /// Remove unreferenced vertices.
    pub fn compact(&mut self) {
        let mut used = vec![u32::MAX; self.verts.len()];
        let mut nv = Vec::new();
        for t in &mut self.tris {
            for i in t.iter_mut() {
                if used[*i as usize] == u32::MAX {
                    used[*i as usize] = nv.len() as u32;
                    nv.push(self.verts[*i as usize]);
                }
                *i = used[*i as usize];
            }
        }
        self.verts = nv;
    }

    /// Split every edge longer than `max_edge` (midpoint subdivision, crack-free:
    /// shared edges are split identically on both sides). `max_iters` bounds work.
    pub fn subdivide(&self, max_edge: f64, max_tris: usize) -> Mesh {
        self.subdivide_where(max_edge, max_tris / 4 + 1, &|_, _| true)
    }

    /// Like `subdivide`, but only edges for which `want(a, b)` is true are split.
    pub fn subdivide_where(&self, max_edge: f64, max_tris: usize, want: &dyn Fn(DVec3, DVec3) -> bool) -> Mesh {
        let mut m = self.clone();
        for _ in 0..12 {
            if m.tris.len() >= max_tris {
                break;
            }
            let mut mids: HashMap<(u32, u32), u32> = HashMap::new();
            let mut any = false;
            let max2 = max_edge * max_edge;
            // decide which edges to split
            for t in &m.tris {
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    let key = (a.min(b), a.max(b));
                    if mids.contains_key(&key) {
                        continue;
                    }
                    if m.verts[a as usize].distance_squared(m.verts[b as usize]) > max2 && want(m.verts[a as usize], m.verts[b as usize]) {
                        let p = (m.verts[a as usize] + m.verts[b as usize]) * 0.5;
                        m.verts.push(p);
                        mids.insert(key, (m.verts.len() - 1) as u32);
                        any = true;
                    }
                }
            }
            if !any {
                break;
            }
            let mut nt = Vec::with_capacity(m.tris.len() * 4);
            let mut nc = Vec::with_capacity(m.tris.len() * 4);
            for (t, &c) in m.tris.iter().zip(&m.colors) {
                let e = |a: u32, b: u32| mids.get(&(a.min(b), a.max(b))).copied();
                let (a, b, cc) = (t[0], t[1], t[2]);
                let (ab, bc, ca) = (e(a, b), e(b, cc), e(cc, a));
                let mut push = |x: [u32; 3]| {
                    nt.push(x);
                    nc.push(c);
                };
                match (ab, bc, ca) {
                    (None, None, None) => push([a, b, cc]),
                    (Some(x), None, None) => { push([a, x, cc]); push([x, b, cc]); }
                    (None, Some(y), None) => { push([a, b, y]); push([a, y, cc]); }
                    (None, None, Some(z)) => { push([a, b, z]); push([z, b, cc]); }
                    (Some(x), Some(y), None) => { push([a, x, cc]); push([x, b, y]); push([x, y, cc]); }
                    (None, Some(y), Some(z)) => { push([a, b, z]); push([z, b, y]); push([z, y, cc]); }
                    (Some(x), None, Some(z)) => { push([a, x, z]); push([x, b, cc]); push([x, cc, z]); }
                    (Some(x), Some(y), Some(z)) => {
                        push([a, x, z]); push([x, b, y]); push([z, y, cc]); push([x, y, z]);
                    }
                }
            }
            m.tris = nt;
            m.colors = nc;
        }
        m
    }

    /// Connected components by shared vertices. Returns component id per triangle and count.
    pub fn components(&self) -> (Vec<u32>, usize) {
        let mut parent: Vec<u32> = (0..self.verts.len() as u32).collect();
        fn find(p: &mut [u32], mut x: u32) -> u32 {
            while p[x as usize] != x {
                p[x as usize] = p[p[x as usize] as usize];
                x = p[x as usize];
            }
            x
        }
        for t in &self.tris {
            let a = find(&mut parent, t[0]);
            let b = find(&mut parent, t[1]);
            let c = find(&mut parent, t[2]);
            parent[b as usize] = a;
            let c2 = find(&mut parent, c);
            parent[c2 as usize] = a;
        }
        let mut ids: HashMap<u32, u32> = HashMap::new();
        let mut out = Vec::with_capacity(self.tris.len());
        for t in &self.tris {
            let r = find(&mut parent, t[0]);
            let n = ids.len() as u32;
            out.push(*ids.entry(r).or_insert(n));
        }
        (out, ids.len())
    }

    /// Triangle adjacency via shared undirected edges (only for manifold edges).
    pub fn tri_neighbors(&self) -> Vec<[u32; 3]> {
        let mut edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edge.entry((a.min(b), a.max(b))).or_default().push(i as u32);
            }
        }
        let mut out = vec![[u32::MAX; 3]; self.tris.len()];
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if let Some(v) = edge.get(&(a.min(b), a.max(b))) {
                    if v.len() == 2 {
                        out[i][k] = if v[0] == i as u32 { v[1] } else { v[0] };
                    }
                }
            }
        }
        out
    }

    pub fn used_colors(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.colors.iter().copied().filter(|&c| c != NO_COLOR).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Ray / triangle intersection (Möller–Trumbore). Returns distance along ray.
pub fn ray_tri(orig: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let s = orig - a;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(q) * inv;
    if t > 1e-9 { Some(t) } else { None }
}

/// Bounding volume hierarchy over a mesh's triangles, for ray queries.
pub struct Bvh {
    nodes: Vec<BvhNode>,
    order: Vec<u32>,
}

struct BvhNode {
    b: Aabb,
    left: u32,  // leaf: first index into order; inner: left child
    right: u32, // inner: right child
    count: u32, // >0 = leaf with `count` tris
}

impl Bvh {
    pub fn build(m: &Mesh) -> Bvh {
        let n = m.tris.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let cents: Vec<DVec3> = (0..n).map(|t| m.tri_centroid(t)).collect();
        let tb: Vec<Aabb> = (0..n)
            .map(|t| {
                let mut b = Aabb::empty();
                for p in m.tri_points(t) {
                    b.grow(p);
                }
                b
            })
            .collect();
        let mut nodes = Vec::with_capacity(2 * n / 4 + 1);
        fn rec(nodes: &mut Vec<BvhNode>, order: &mut [u32], start: usize, cents: &[DVec3], tb: &[Aabb]) -> u32 {
            let mut b = Aabb::empty();
            let mut cb = Aabb::empty();
            for &t in order.iter() {
                b = b.union(&tb[t as usize]);
                cb.grow(cents[t as usize]);
            }
            let id = nodes.len() as u32;
            nodes.push(BvhNode { b, left: start as u32, right: 0, count: order.len() as u32 });
            if order.len() <= 4 {
                return id;
            }
            let s = cb.size();
            let axis = if s.x >= s.y && s.x >= s.z { 0 } else if s.y >= s.z { 1 } else { 2 };
            let mid = order.len() / 2;
            order.select_nth_unstable_by(mid, |a, b| {
                cents[*a as usize][axis].partial_cmp(&cents[*b as usize][axis]).unwrap_or(std::cmp::Ordering::Equal)
            });
            let (l, r) = order.split_at_mut(mid);
            let li = rec(nodes, l, start, cents, tb);
            let ri = rec(nodes, r, start + mid, cents, tb);
            nodes[id as usize].left = li;
            nodes[id as usize].right = ri;
            nodes[id as usize].count = 0;
            id
        }
        if n > 0 {
            rec(&mut nodes, &mut order, 0, &cents, &tb);
        }
        Bvh { nodes, order }
    }

    fn ray_box(b: &Aabb, o: DVec3, inv: DVec3, tmax: f64) -> bool {
        let t1 = (b.min - o) * inv;
        let t2 = (b.max - o) * inv;
        let tmin = t1.min(t2).max_element().max(0.0);
        let tm = t1.max(t2).min_element().min(tmax);
        tmin <= tm
    }

    /// Closest hit: (distance, triangle). `skip` triangle is ignored.
    pub fn raycast(&self, m: &Mesh, o: DVec3, d: DVec3, skip: Option<u32>) -> Option<(f64, u32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = DVec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best: Option<(f64, u32)> = None;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let n = &self.nodes[ni as usize];
            let tmax = best.map(|b| b.0).unwrap_or(f64::INFINITY);
            if !Self::ray_box(&n.b, o, inv, tmax) {
                continue;
            }
            if n.count > 0 {
                for &t in &self.order[n.left as usize..(n.left + n.count) as usize] {
                    if Some(t) == skip {
                        continue;
                    }
                    let [a, b, c] = m.tri_points(t as usize);
                    if let Some(dist) = ray_tri(o, d, a, b, c) {
                        if dist < best.map(|b| b.0).unwrap_or(f64::INFINITY) {
                            best = Some((dist, t));
                        }
                    }
                }
            } else {
                stack.push(n.left);
                stack.push(n.right);
            }
        }
        best
    }

    /// All triangles whose bounding box overlaps `q`.
    pub fn query_box(&self, q: &Aabb, out: &mut Vec<u32>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let n = &self.nodes[ni as usize];
            if !(n.b.min.x <= q.max.x && q.min.x <= n.b.max.x && n.b.min.y <= q.max.y && q.min.y <= n.b.max.y
                && n.b.min.z <= q.max.z && q.min.z <= n.b.max.z)
            {
                continue;
            }
            if n.count > 0 {
                out.extend_from_slice(&self.order[n.left as usize..(n.left + n.count) as usize]);
            } else {
                stack.push(n.left);
                stack.push(n.right);
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RepairStats {
    pub welded: usize,
    pub removed_faces: usize,
    pub flipped_faces: usize,
    pub holes_filled: usize,
}

impl Mesh {
    /// Best-effort repair: weld, drop degenerate/duplicate faces, make winding
    /// consistent, orient outward, and close boundary loops.
    pub fn repair(&self, tol: f64) -> (Mesh, RepairStats) {
        let mut st = RepairStats::default();
        let mut m = self.clone();
        let nv0 = m.verts.len();
        let nt0 = m.tris.len();
        m.weld(tol);
        st.welded = nv0.saturating_sub(m.verts.len());
        // degenerate + duplicate
        let mut seen = std::collections::HashSet::new();
        let mut keep_t = Vec::new();
        let mut keep_c = Vec::new();
        for (i, t) in m.tris.iter().enumerate() {
            let mut s = *t;
            s.sort_unstable();
            if m.tri_normal_area(i).1 > 1e-12 && seen.insert(s) {
                keep_t.push(*t);
                keep_c.push(m.colors[i]);
            }
        }
        m.tris = keep_t;
        m.colors = keep_c;
        st.removed_faces = nt0 - m.tris.len();
        // consistent winding by BFS over manifold edges
        let mut edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (i, t) in m.tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edge.entry((a.min(b), a.max(b))).or_default().push(i as u32);
            }
        }
        let n = m.tris.len();
        let mut visited = vec![false; n];
        let (comp, ncomp) = m.components();
        let _ = ncomp;
        for s in 0..n {
            if visited[s] {
                continue;
            }
            visited[s] = true;
            let mut stack = vec![s];
            while let Some(t) = stack.pop() {
                let tri = m.tris[t];
                for k in 0..3 {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    let Some(adj) = edge.get(&(a.min(b), a.max(b))) else { continue };
                    if adj.len() != 2 {
                        continue;
                    }
                    let o = if adj[0] as usize == t { adj[1] } else { adj[0] } as usize;
                    if visited[o] {
                        continue;
                    }
                    // neighbour must traverse the shared edge as b->a
                    let ot = m.tris[o];
                    let same_dir = (0..3).any(|j| ot[j] == a && ot[(j + 1) % 3] == b);
                    if same_dir {
                        m.tris[o].swap(1, 2);
                        st.flipped_faces += 1;
                    }
                    visited[o] = true;
                    stack.push(o);
                }
            }
        }
        // close boundary loops
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        for (&(a, b), _) in directed.iter() {
            if !directed.contains_key(&(b, a)) {
                // boundary edge a->b; the fill must run b->a
                next.insert(b, a);
            }
        }
        let mut used = std::collections::HashSet::new();
        let starts: Vec<u32> = next.keys().copied().collect();
        for s in starts {
            if used.contains(&s) {
                continue;
            }
            let mut lp = vec![s];
            used.insert(s);
            let mut cur = s;
            let mut ok = false;
            for _ in 0..100_000 {
                let Some(&nx) = next.get(&cur) else { break };
                if nx == s {
                    ok = true;
                    break;
                }
                if !used.insert(nx) {
                    break;
                }
                lp.push(nx);
                cur = nx;
            }
            if ok && lp.len() >= 3 {
                let c = lp.iter().map(|&i| m.verts[i as usize]).sum::<DVec3>() / lp.len() as f64;
                let ci = m.add_vert(c);
                let col = m.colors.first().copied().unwrap_or(NO_COLOR);
                for i in 0..lp.len() {
                    m.tris.push([lp[i], lp[(i + 1) % lp.len()], ci]);
                    m.colors.push(col);
                }
                st.holes_filled += 1;
            }
        }
        // orient each shell outward
        let (comp2, nc2) = m.components();
        let mut vol = vec![0.0; nc2];
        for (t, &c) in comp2.iter().enumerate() {
            let [a, b, cc] = m.tri_points(t);
            vol[c as usize] += a.dot(b.cross(cc));
        }
        // only flip shells that aren't enclosed cavities: flip if negative and largest-ish
        let total: f64 = vol.iter().sum();
        if total < 0.0 {
            m.flip();
            st.flipped_faces += m.tris.len();
        }
        let _ = comp;
        m.compact();
        (m, st)
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    #[test]
    fn repair_closes_and_orients() {
        let mut m = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(10.0));
        m.tris.pop();
        m.colors.pop();
        m.tris[0].swap(1, 2);
        m.flip();
        let (r, st) = m.repair(1e-6);
        assert!(st.holes_filled == 1, "{st:?}");
        assert!((r.volume() - 1000.0).abs() < 1e-6, "{}", r.volume());
    }
}

#[cfg(test)]
mod bvh_tests {
    use super::*;
    #[test]
    fn vertical_ray_hits_box_top() {
        let m = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        let b = Bvh::build(&m);
        let o = DVec3::new(3.0, 2.0, 1.0e5);
        let hit = b.raycast(&m, o, DVec3::NEG_Z, None);
        assert!(hit.is_some(), "no hit");
        assert!((o.z - hit.unwrap().0 - 20.0).abs() < 1e-6, "{:?}", hit);
    }
}

/// Closest point on triangle abc to p (Ericson, Real-Time Collision Detection).
pub fn closest_on_tri(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

impl Mesh {
    /// Split the given (undirected) edges at their midpoints. Neighbouring triangles share
    /// the split, so the surface stays watertight. Triangle colours are inherited.
    pub fn split_edges(&self, edges: &std::collections::HashSet<(u32, u32)>) -> Mesh {
        let mut m = self.clone();
        let mut mids: HashMap<(u32, u32), u32> = HashMap::new();
        for &(a, b) in edges {
            let p = (m.verts[a as usize] + m.verts[b as usize]) * 0.5;
            m.verts.push(p);
            mids.insert((a.min(b), a.max(b)), (m.verts.len() - 1) as u32);
        }
        let mut nt = Vec::with_capacity(m.tris.len() + edges.len() * 2);
        let mut nc = Vec::with_capacity(nt.capacity());
        for (t, &c) in self.tris.iter().zip(&self.colors) {
            let e = |a: u32, b: u32| mids.get(&(a.min(b), a.max(b))).copied();
            let (a, b, cc) = (t[0], t[1], t[2]);
            let (ab, bc, ca) = (e(a, b), e(b, cc), e(cc, a));
            let mut push = |x: [u32; 3]| {
                nt.push(x);
                nc.push(c);
            };
            match (ab, bc, ca) {
                (None, None, None) => push([a, b, cc]),
                (Some(x), None, None) => { push([a, x, cc]); push([x, b, cc]); }
                (None, Some(y), None) => { push([a, b, y]); push([a, y, cc]); }
                (None, None, Some(z)) => { push([a, b, z]); push([z, b, cc]); }
                (Some(x), Some(y), None) => { push([a, x, cc]); push([x, b, y]); push([x, y, cc]); }
                (None, Some(y), Some(z)) => { push([a, b, z]); push([z, b, y]); push([z, y, cc]); }
                (Some(x), None, Some(z)) => { push([a, x, z]); push([x, b, cc]); push([x, cc, z]); }
                (Some(x), Some(y), Some(z)) => { push([a, x, z]); push([x, b, y]); push([z, y, cc]); push([x, y, z]); }
            }
        }
        m.tris = nt;
        m.colors = nc;
        m
    }

    /// Refine only near point `p` (within `radius`) until edges there are <= `target`.
    pub fn refine_near(&self, p: DVec3, radius: f64, target: f64, max_tris: usize) -> Mesh {
        let mut m = self.clone();
        for _ in 0..14 {
            if m.tris.len() >= max_tris {
                break;
            }
            let mut edges = std::collections::HashSet::new();
            let t2 = target * target;
            for t in 0..m.tris.len() {
                let [a, b, c] = m.tri_points(t);
                if closest_on_tri(p, a, b, c).distance(p) > radius {
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
        m
    }
}

#[cfg(test)]
mod refine_tests {
    use super::*;
    #[test]
    fn local_refine_stays_watertight_and_small() {
        let m = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        let p = DVec3::new(10.0, 0.0, 10.0); // centre of the +X face
        let r = m.refine_near(p, 2.0, 1.0, 1_000_000);
        // watertight: every directed edge has its opposite
        let mut e = std::collections::HashSet::new();
        for t in &r.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        assert!(e.iter().all(|&(a, b)| e.contains(&(b, a))));
        assert!((r.volume() - 8000.0).abs() < 1e-6);
        // triangles touching the brush are small
        for t in 0..r.tris.len() {
            let [a, b, c] = r.tri_points(t);
            if closest_on_tri(p, a, b, c).distance(p) < 2.0 {
                assert!(a.distance(b).max(b.distance(c)).max(c.distance(a)) <= 1.0 + 1e-9);
            }
        }
        assert!(r.tris.len() < 4000, "{}", r.tris.len());
    }
}

impl Mesh {
    /// Connected flat area around `start`: neighbours within `angle_deg` of the start
    /// face's normal and lying in (nearly) the same plane.
    pub fn flat_region(&self, start: usize, angle_deg: f64) -> Vec<u32> {
        let nb = self.tri_neighbors();
        let (n0, _) = self.tri_normal_area(start);
        let p0 = self.tri_points(start)[0];
        let lim = angle_deg.to_radians().cos();
        let mut seen = vec![false; self.tris.len()];
        let mut out = vec![start as u32];
        seen[start] = true;
        let mut stack = vec![start];
        while let Some(t) = stack.pop() {
            for &x in &nb[t] {
                if x == u32::MAX || seen[x as usize] {
                    continue;
                }
                let (nx, _) = self.tri_normal_area(x as usize);
                let off = self.tri_centroid(x as usize) - p0;
                if nx.dot(n0) >= lim && off.dot(n0).abs() < 1e-3 * (1.0 + off.length()) {
                    seen[x as usize] = true;
                    out.push(x);
                    stack.push(x as usize);
                }
            }
        }
        out
    }

    /// Closed prism made by sweeping the given flat region along `n` from offset `a` to `b`.
    pub fn region_prism(&self, region: &[u32], n: DVec3, a: f64, b: f64, color: u8) -> Mesh {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut m = Mesh::new();
        let mut map: HashMap<u32, (u32, u32)> = HashMap::new();
        let mut get = |v: u32, m: &mut Mesh| -> (u32, u32) {
            *map.entry(v).or_insert_with(|| {
                let p = self.verts[v as usize];
                (m.add_vert(p + n * lo), m.add_vert(p + n * hi))
            })
        };
        let mut directed = std::collections::HashSet::new();
        for &t in region {
            let tri = self.tris[t as usize];
            for k in 0..3 {
                directed.insert((tri[k], tri[(k + 1) % 3]));
            }
        }
        for &t in region {
            let [x, y, z] = self.tris[t as usize];
            let (xl, xh) = get(x, &mut m);
            let (yl, yh) = get(y, &mut m);
            let (zl, zh) = get(z, &mut m);
            m.add_tri(xh, yh, zh);
            m.add_tri(xl, zl, yl);
        }
        for &(p, q) in &directed {
            if directed.contains(&(q, p)) {
                continue; // interior edge
            }
            let (pl, ph) = get(p, &mut m);
            let (ql, qh) = get(q, &mut m);
            m.add_quad(pl, ql, qh, ph);
        }
        m.set_color(color);
        m.weld(1e-9);
        if m.volume() < 0.0 {
            m.flip();
        }
        m
    }
}

#[cfg(test)]
mod pushpull_tests {
    use super::*;
    #[test]
    fn extrude_box_face() {
        let mut m = crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(20.0));
        m.set_color(0);
        // a triangle on the +X face
        let start = (0..m.tris.len()).find(|&t| m.tri_normal_area(t).0.x > 0.99).unwrap();
        let region = m.flat_region(start, 1.0);
        assert_eq!(region.len(), 2);
        let prism = m.region_prism(&region, DVec3::X, -0.01, 10.0, 0);
        assert!((prism.volume() - 20.0 * 20.0 * 10.01).abs() < 1e-6, "{}", prism.volume());
        let r = crate::csg::combine(&[m.clone(), prism], &[]);
        assert!((r.mesh.volume() - 20.0 * 20.0 * 30.0).abs() < 1e-3, "{}", r.mesh.volume());
        // push in
        let cut = m.region_prism(&region, DVec3::X, -5.0, 0.01, 0);
        let r2 = crate::csg::combine(&[m], &[cut]);
        assert!((r2.mesh.volume() - 20.0 * 20.0 * 15.0).abs() < 1e-3, "{}", r2.mesh.volume());
    }
}

impl Mesh {
    /// Split triangles that cross the horizontal plane z = h so that no triangle spans it.
    /// Shared edges are cut at the same new vertex, so the mesh stays watertight.
    pub fn split_at_z(&self, h: f64) -> Mesh {
        let eps = 1e-7;
        let mut m = Mesh::new();
        m.verts = self.verts.clone();
        let mut cut: HashMap<(u32, u32), u32> = HashMap::new();
        let side = |v: &DVec3| if v.z > h + eps { 1 } else if v.z < h - eps { -1 } else { 0 };
        let mut edge_point = |a: u32, b: u32, m: &mut Mesh| -> u32 {
            let key = (a.min(b), a.max(b));
            if let Some(&i) = cut.get(&key) {
                return i;
            }
            let (pa, pb) = (m.verts[a as usize], m.verts[b as usize]);
            let t = (h - pa.z) / (pb.z - pa.z);
            let p = pa + (pb - pa) * t;
            let i = m.add_vert(DVec3::new(p.x, p.y, h));
            cut.insert(key, i);
            i
        };
        for (t, &c) in self.tris.iter().zip(&self.colors) {
            let s: Vec<i32> = t.iter().map(|&i| side(&self.verts[i as usize])).collect();
            let crosses = |i: usize, j: usize| s[i] * s[j] < 0;
            let n_cross = (0..3).filter(|&k| crosses(k, (k + 1) % 3)).count();
            let push = |x: [u32; 3], m: &mut Mesh| {
                m.tris.push(x);
                m.colors.push(c);
            };
            if n_cross == 0 {
                push(*t, &mut m);
                continue;
            }
            // rotate so the "odd one out" vertex is first
            let mut k0 = 0;
            for k in 0..3 {
                let (a, b, cc) = (s[k], s[(k + 1) % 3], s[(k + 2) % 3]);
                if n_cross == 2 && a != 0 && b != 0 && cc != 0 && a != b && a != cc {
                    k0 = k;
                }
                if n_cross == 1 && a == 0 {
                    k0 = k;
                }
            }
            let v = [t[k0], t[(k0 + 1) % 3], t[(k0 + 2) % 3]];
            if n_cross == 2 {
                // v0 alone on one side
                let p01 = edge_point(v[0], v[1], &mut m);
                let p02 = edge_point(v[0], v[2], &mut m);
                push([v[0], p01, p02], &mut m);
                push([p01, v[1], v[2]], &mut m);
                push([p01, v[2], p02], &mut m);
            } else {
                // v0 on the plane, edge v1-v2 crosses
                let p12 = edge_point(v[1], v[2], &mut m);
                push([v[0], v[1], p12], &mut m);
                push([v[0], p12, v[2]], &mut m);
            }
        }
        m
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;
    #[test]
    fn split_box_keeps_volume_and_closure() {
        let m = crate::shapes::Shape::Sphere { steps: 16 }.generate(DVec3::splat(20.0));
        let s = m.split_at_z(7.3).split_at_z(12.0);
        assert!((s.volume() - m.volume()).abs() < 1e-6);
        let mut e = std::collections::HashSet::new();
        for t in &s.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        assert!(e.iter().all(|&(a, b)| e.contains(&(b, a))), "not watertight");
        for t in 0..s.tris.len() {
            let [a, b, c] = s.tri_points(t);
            let (lo, hi) = (a.z.min(b.z).min(c.z), a.z.max(b.z).max(c.z));
            assert!(!(lo < 7.3 - 1e-6 && hi > 7.3 + 1e-6), "triangle spans plane");
        }
    }
}
