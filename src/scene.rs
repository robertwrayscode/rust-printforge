//! Scene graph: objects, groups, holes, colours, paint, and baked meshes.

use crate::csg;
use crate::mesh::{Aabb, Mesh, NO_COLOR};
use crate::shapes::Shape;
use glam::{DMat4, DQuat, DVec3, EulerRot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Kind {
    Shape(Shape),
    /// Frozen triangle mesh (imported file, or a shape converted for detailed painting).
    /// Stored at its natural size; scaled to `Object::size`.
    Mesh(Arc<Mesh>),
    /// Children are stored in the group's local frame.
    Group { children: Vec<Object>, keep_colors: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    /// Bounding-box size in mm (W, D, H) before rotation.
    pub size: DVec3,
    /// Position of the bottom-centre of the object's box (before rotation it sits on z=pos.z).
    pub pos: DVec3,
    /// Rotation in degrees (applied X, then Y, then Z).
    pub rot: DVec3,
    pub mirror: [bool; 3],
    pub hole: bool,
    /// Filament slot (0-based).
    pub color: u8,
    pub visible: bool,
    pub locked: bool,
    /// Per-triangle painted filament (NO_COLOR = unpainted) for the object's local mesh.
    #[serde(default)]
    pub paint: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Filament {
    pub name: String,
    pub color: [u8; 3],
    pub material: String,
}

impl Filament {
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.color[0], self.color[1], self.color[2])
    }
}

pub fn default_filaments() -> Vec<Filament> {
    let c = [
        ("White", [240, 240, 240]),
        ("Black", [30, 30, 30]),
        ("Red", [200, 30, 40]),
        ("Blue", [20, 90, 200]),
        ("Yellow", [245, 200, 20]),
        ("Green", [30, 160, 70]),
        ("Orange", [240, 120, 20]),
    ];
    c.iter().map(|(n, col)| Filament { name: n.to_string(), color: *col, material: "PLA".into() }).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Printer {
    pub name: String,
    /// Usable bed when printing with one nozzle (X, Y) and max height.
    pub bed: [f64; 2],
    pub bed_multi: [f64; 2],
    pub height: f64,
    pub nozzle: f64,
    pub layer: f64,
    /// Filaments that can be loaded without purging/swapping (H2C: 1 left + 6 Vortek hotends).
    pub nozzle_slots: usize,
    pub max_filaments: usize,
}

impl Default for Printer {
    fn default() -> Self {
        Printer {
            name: "Bambu Lab H2C".into(),
            bed: [305.0, 320.0],
            bed_multi: [300.0, 320.0],
            height: 325.0,
            nozzle: 0.4,
            layer: 0.2,
            nozzle_slots: 7,
            max_filaments: 24,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub objects: Vec<Object>,
    pub filaments: Vec<Filament>,
    pub printer: Printer,
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Document { version: 1, objects: vec![], filaments: default_filaments(), printer: Printer::default(), next_id: 1 }
    }
}

impl Object {
    pub fn new(id: u64, name: &str, kind: Kind, size: DVec3) -> Self {
        Object {
            id,
            name: name.into(),
            kind,
            size,
            pos: DVec3::ZERO,
            rot: DVec3::ZERO,
            mirror: [false; 3],
            hole: false,
            color: 0,
            visible: true,
            locked: false,
            paint: None,
        }
    }

    pub fn rotation(&self) -> DQuat {
        DQuat::from_euler(
            EulerRot::ZYX,
            self.rot.z.to_radians(),
            self.rot.y.to_radians(),
            self.rot.x.to_radians(),
        )
    }

    pub fn set_rotation(&mut self, q: DQuat) {
        let (z, y, x) = q.to_euler(EulerRot::ZYX);
        let clean = |a: f64| {
            let d = a.to_degrees();
            let r = (d * 1000.0).round() / 1000.0;
            if r == -0.0 { 0.0 } else { r }
        };
        self.rot = DVec3::new(clean(x), clean(y), clean(z));
    }

    /// Local mesh space -> parent space. The local mesh's box centre-bottom sits at the
    /// origin; we rotate about the box centre so rotation doesn't drift the object.
    pub fn transform(&self) -> DMat4 {
        let half = DVec3::new(0.0, 0.0, self.size.z * 0.5);
        let mir = DVec3::new(
            if self.mirror[0] { -1.0 } else { 1.0 },
            if self.mirror[1] { -1.0 } else { 1.0 },
            if self.mirror[2] { -1.0 } else { 1.0 },
        );
        DMat4::from_translation(self.pos + half)
            * DMat4::from_quat(self.rotation())
            * DMat4::from_scale(mir)
            * DMat4::from_translation(-half)
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, Kind::Group { .. })
    }

    /// Hash of everything that affects the local mesh (geometry + colours).
    pub fn geom_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash_geom(&mut h);
        h.finish()
    }

    fn hash_geom<H: Hasher>(&self, h: &mut H) {
        for v in self.size.to_array() {
            v.to_bits().hash(h);
        }
        match &self.kind {
            Kind::Shape(s) => format!("{s:?}").hash(h),
            Kind::Mesh(m) => {
                (Arc::as_ptr(m) as usize).hash(h);
                m.tris.len().hash(h);
            }
            Kind::Group { children, keep_colors } => {
                keep_colors.hash(h);
                for c in children {
                    c.hash_geom(h);
                    c.hole.hash(h);
                    c.color.hash(h);
                    c.visible.hash(h);
                    for v in [c.pos, c.rot].iter().flat_map(|v| v.to_array()) {
                        v.to_bits().hash(h);
                    }
                    c.mirror.hash(h);
                    c.paint.hash(h);
                }
            }
        }
    }
}

/// Cache of baked local meshes keyed by geometry hash.
#[derive(Default)]
pub struct MeshCache {
    map: HashMap<u64, Arc<Mesh>>,
    bakes: HashMap<u64, (Arc<Mesh>, DVec3, DVec3)>,
    pub warnings: HashMap<u64, Vec<String>>,
    used: std::collections::HashSet<u64>,
}

impl MeshCache {
    /// Local mesh of an object at its size, with triangle colours resolved
    /// (object colour / group colours) but WITHOUT paint applied.
    pub fn base_mesh(&mut self, o: &Object) -> Arc<Mesh> {
        let key = o.geom_key();
        self.used.insert(key);
        if let Some(m) = self.map.get(&key) {
            return m.clone();
        }
        let mesh = match &o.kind {
            Kind::Shape(s) => s.generate(o.size),
            Kind::Mesh(m) => fit_to_size(m, o.size),
            Kind::Group { children, keep_colors } => {
                let (m, _natural, _off) = self.bake_group(children, *keep_colors);
                fit_to_size(&m, o.size)
            }
        };
        let arc = Arc::new(mesh);
        self.map.insert(key, arc.clone());
        arc
    }

    /// CSG-combine a group's children (in the group frame). Returns the result normalised
    /// to bottom-centre origin, its natural size, and the offset of that origin in the group frame.
    pub fn bake_group(&mut self, children: &[Object], keep_colors: bool) -> (Arc<Mesh>, DVec3, DVec3) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let tmp = Object::new(0, "", Kind::Group { children: children.to_vec(), keep_colors }, DVec3::ZERO);
        tmp.hash_geom(&mut h);
        let key = h.finish();
        if let Some(b) = self.bakes.get(&key) {
            return b.clone();
        }
        let mut solids = Vec::new();
        let mut holes = Vec::new();
        for c in children.iter().filter(|c| c.visible) {
            let m = self.colored_mesh(c).transformed(&c.transform());
            if c.hole { holes.push(m) } else { solids.push(m) }
        }
        let r = csg::combine(&solids, &holes);
        if !r.warnings.is_empty() {
            self.warnings.insert(key, r.warnings);
        }
        let mut m = r.mesh;
        if !keep_colors {
            m.set_color(NO_COLOR);
        }
        let (nm, size, off) = normalize_mesh(m);
        let v = (Arc::new(nm), size.max(DVec3::splat(1e-6)), off);
        if self.bakes.len() > 64 {
            self.bakes.clear();
        }
        self.bakes.insert(key, v.clone());
        v
    }

    /// Local mesh with object colour and paint applied.
    pub fn colored_mesh(&mut self, o: &Object) -> Mesh {
        let mut m = (*self.base_mesh(o)).clone();
        if let Some(p) = &o.paint {
            if p.len() == m.colors.len() {
                for (c, &pc) in m.colors.iter_mut().zip(p) {
                    if pc != NO_COLOR {
                        *c = pc;
                    }
                }
            }
        }
        m.fill_unset_color(o.color);
        m
    }

    pub fn world_mesh(&mut self, o: &Object) -> Mesh {
        self.colored_mesh(o).transformed(&o.transform())
    }

    /// Drop entries not used since the last call.
    pub fn gc(&mut self) {
        if self.map.len() > 64 {
            let used = std::mem::take(&mut self.used);
            self.map.retain(|k, _| used.contains(k));
        }
        self.used.clear();
    }
}

/// Scale a mesh (with box min z = 0 and centred XY) to the requested size.
pub fn fit_to_size(m: &Mesh, size: DVec3) -> Mesh {
    let b = m.bounds();
    let s = b.size().max(DVec3::splat(1e-9));
    let mut out = m.clone();
    let c = b.center();
    for v in &mut out.verts {
        *v = DVec3::new((v.x - c.x) / s.x * size.x, (v.y - c.y) / s.y * size.y, (v.z - b.min.z) / s.z * size.z);
    }
    out
}

/// Normalise an arbitrary mesh so it's centred on XY with min z = 0. Returns (mesh, size, offset).
pub fn normalize_mesh(mut m: Mesh) -> (Mesh, DVec3, DVec3) {
    let b = m.bounds();
    let c = b.center();
    let off = DVec3::new(c.x, c.y, b.min.z);
    m.translate(-off);
    let size = b.size();
    (m, size, off)
}

impl Document {
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn find(&self, id: u64) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn world_bounds(&self, cache: &mut MeshCache, o: &Object) -> Aabb {
        cache.world_mesh(o).bounds()
    }

    /// Meshes that will be printed: every visible top-level solid. Returns (object index, world mesh).
    pub fn printable_meshes(&self, cache: &mut MeshCache) -> Vec<(usize, Mesh)> {
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.visible && !o.hole)
            .map(|(i, o)| (i, cache.world_mesh(o)))
            .filter(|(_, m)| !m.tris.is_empty())
            .collect()
    }

    /// Group the given objects into one new object (Tinkercad "Group").
    pub fn group(&mut self, ids: &[u64], cache: &mut MeshCache) -> Option<u64> {
        if ids.len() < 2 {
            return None;
        }
        let mut members: Vec<Object> = Vec::new();
        let first_idx = self.objects.iter().position(|o| ids.contains(&o.id))?;
        let mut bounds = Aabb::empty();
        for o in self.objects.iter().filter(|o| ids.contains(&o.id)) {
            bounds = bounds.union(&cache.world_mesh(o).bounds());
            members.push(o.clone());
        }
        let solid_colors: Vec<u8> = members.iter().filter(|m| !m.hole).map(|m| m.color).collect();
        self.objects.retain(|o| !ids.contains(&o.id));
        let origin = DVec3::new(bounds.center().x, bounds.center().y, bounds.min.z);
        for m in &mut members {
            m.pos -= origin;
        }
        let id = self.alloc_id();
        let (_m, natural, off) = cache.bake_group(&members, true);
        let mut g = Object::new(id, "Group", Kind::Group { children: members, keep_colors: true }, natural);
        g.color = solid_colors.first().copied().unwrap_or(0);
        g.pos = origin + off;
        self.objects.insert(first_idx.min(self.objects.len()), g);
        Some(id)
    }

    /// Undo a group: children are placed back into the scene (only when the group
    /// hasn't been rotated/scaled in a way children can't represent, we bake instead).
    pub fn ungroup(&mut self, id: u64, cache: &mut MeshCache) -> Vec<u64> {
        let Some(idx) = self.objects.iter().position(|o| o.id == id) else { return vec![] };
        let g = self.objects[idx].clone();
        let Kind::Group { children, .. } = &g.kind else { return vec![] };
        let (_m, natural, off) = cache.bake_group(children, true);
        let scale = g.size / natural.max(DVec3::splat(1e-9));
        let simple = (scale - DVec3::ONE).abs().max_element() < 1e-6 && !g.mirror.iter().any(|&m| m);
        let local_to_world = g.transform() * DMat4::from_translation(-off);
        let mut out = Vec::new();
        let mut new_objs = Vec::new();
        for c in children {
            let mut c = c.clone();
            c.id = self.alloc_id();
            if simple {
                // keep child transform form: T(p'+half) R' M T(-half) == L * T(p+half) R M T(-half)
                let (_s, lr, _t) = local_to_world.to_scale_rotation_translation();
                let half = DVec3::new(0.0, 0.0, c.size.z * 0.5);
                let q = lr * c.rotation();
                c.pos = local_to_world.transform_point3(c.pos + half) - half;
                c.set_rotation(q);
            } else {
                // bake: freeze child geometry into world space
                let wm = cache.colored_mesh(&c).transformed(&(local_to_world * c.transform()));
                let (nm, size, off) = normalize_mesh(wm);
                c.kind = Kind::Mesh(Arc::new(nm));
                c.size = size;
                c.pos = off;
                c.rot = DVec3::ZERO;
                c.mirror = [false; 3];
                c.paint = None;
            }
            out.push(c.id);
            new_objs.push(c);
        }
        self.objects.splice(idx..idx + 1, new_objs);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_keeps_world_position() {
        let mut d = Document::default();
        let mut cache = MeshCache::default();
        let id1 = d.alloc_id();
        let mut a = Object::new(id1, "a", Kind::Shape(Shape::Box { radius: 0.0, steps: 1 }), DVec3::splat(10.0));
        a.pos = DVec3::new(30.0, 5.0, 0.0);
        let id2 = d.alloc_id();
        let mut b = Object::new(id2, "b", Kind::Shape(Shape::Box { radius: 0.0, steps: 1 }), DVec3::splat(10.0));
        b.pos = DVec3::new(35.0, 5.0, 5.0);
        b.color = 2;
        d.objects.push(a);
        d.objects.push(b);
        let before = d.objects.iter().map(|o| cache.world_mesh(o).bounds()).fold(Aabb::empty(), |x, y| x.union(&y));
        let g = d.group(&[id1, id2], &mut cache).unwrap();
        let gm = cache.world_mesh(d.find(g).unwrap());
        let after = gm.bounds();
        assert!((before.min - after.min).abs().max_element() < 1e-6, "{:?} {:?}", before, after);
        assert!((before.max - after.max).abs().max_element() < 1e-6);
        assert_eq!(gm.used_colors(), vec![0, 2]);
        let ids = d.ungroup(g, &mut cache);
        assert_eq!(ids.len(), 2);
        let back = d.objects.iter().map(|o| cache.world_mesh(o).bounds()).fold(Aabb::empty(), |x, y| x.union(&y));
        assert!((before.min - back.min).abs().max_element() < 1e-6);
    }

    #[test]
    fn rotation_about_center() {
        let mut o = Object::new(1, "a", Kind::Shape(Shape::Box { radius: 0.0, steps: 1 }), DVec3::new(10.0, 10.0, 20.0));
        o.rot = DVec3::new(90.0, 0.0, 0.0);
        let mut c = MeshCache::default();
        let b = c.world_mesh(&o).bounds();
        assert!((b.center().z - 10.0).abs() < 1e-9);
    }
}
