//! The PrintForge desktop application.

use crate::check::{self, CheckInput, Report, Severity};
use crate::io;
use crate::mesh::{Aabb, Bvh, Mesh, NO_COLOR};
use crate::render::{Batch, Pass, Renderer, SharedRenderer};
use crate::scene::{Document, Filament, Kind, MeshCache, Object, normalize_mesh};
use crate::shapes::{FontChoice, LIBRARY, Shape};
use eframe::egui::{self, Color32, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use glam::{DMat4, DQuat, DVec3, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use transform_gizmo_egui::prelude::*;

// ------------------------------------------------------------------ camera

#[derive(Clone)]
pub struct Camera {
    pub target: DVec3,
    pub yaw: f64,
    pub pitch: f64,
    pub dist: f64,
    pub ortho: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { target: DVec3::new(0.0, 0.0, 10.0), yaw: -60.0, pitch: 32.0, dist: 320.0, ortho: false }
    }
}

impl Camera {
    pub fn eye(&self) -> DVec3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        self.target + DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin()) * self.dist
    }
    pub fn view(&self) -> DMat4 {
        let up = if self.pitch.abs() > 89.0 { DVec3::new(-self.yaw.to_radians().cos(), -self.yaw.to_radians().sin(), 0.0) * self.pitch.signum() } else { DVec3::Z };
        DMat4::look_at_rh(self.eye(), self.target, up)
    }
    pub fn proj(&self, aspect: f64) -> DMat4 {
        let fov = 35f64.to_radians();
        if self.ortho {
            let h = self.dist * (fov / 2.0).tan();
            DMat4::orthographic_rh_gl(-h * aspect, h * aspect, -h, h, -20000.0, 20000.0)
        } else {
            DMat4::perspective_rh_gl(fov, aspect, (self.dist * 0.01).max(0.5), 20000.0)
        }
    }
    pub fn ray(&self, rect: Rect, p: Pos2) -> (DVec3, DVec3) {
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let inv = (self.proj(aspect) * self.view()).inverse();
        let nx = ((p.x - rect.left()) / rect.width()) as f64 * 2.0 - 1.0;
        let ny = 1.0 - ((p.y - rect.top()) / rect.height()) as f64 * 2.0;
        let a = inv.project_point3(DVec3::new(nx, ny, -1.0));
        let b = inv.project_point3(DVec3::new(nx, ny, 1.0));
        (a, (b - a).normalize())
    }
    pub fn project(&self, rect: Rect, w: DVec3) -> Option<Pos2> {
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let c = (self.proj(aspect) * self.view()) * w.extend(1.0);
        if c.w <= 1e-9 {
            return None;
        }
        let n = c.truncate() / c.w;
        Some(Pos2::new(
            rect.left() + ((n.x + 1.0) * 0.5) as f32 * rect.width(),
            rect.top() + ((1.0 - n.y) * 0.5) as f32 * rect.height(),
        ))
    }
}

// ------------------------------------------------------------------ app state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Paint,
    Fill,
    Measure,
    LayFlat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GizmoKind {
    Off,
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportKind {
    Bambu,
    Generic3mf,
    Stl,
    Obj,
}

struct Entry {
    id: u64,
    key: u64,
    hole: bool,
    mesh: Mesh,
    bvh: Option<Arc<Bvh>>,
}

struct Hit {
    entry: usize,
    tri: u32,
    point: DVec3,
}

enum Drag {
    None,
    Orbit,
    Pan,
    Move { plane_z: f64, start_hit: DVec3, start: Vec<(u64, DVec3)> },
    Box { start: Pos2 },
    Paint,
}

pub struct App {
    pub doc: Document,
    cache: MeshCache,
    pub selection: Vec<u64>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    committed: Document,
    changed: bool,
    clipboard: Vec<Object>,
    pub camera: Camera,
    pub tool: Tool,
    gizmo: Gizmo,
    gizmo_kind: GizmoKind,
    gizmo_start: Option<Vec<(u64, DVec3, DVec3, DVec3)>>,
    gizmo_grab: bool,
    snap: f64,
    renderer: SharedRenderer,
    entries: Vec<Entry>,
    geometry_dirty: bool,
    drag: Drag,
    hover: Option<Hit>,
    // painting
    paint_slot: u8,
    brush_mm: f64,
    fill_angle: f64,
    erase: bool,
    refine_mm: f64,
    // measure
    measure: Vec<DVec3>,
    // check
    report: Option<Report>,
    check_rx: Option<mpsc::Receiver<Report>>,
    selected_issue: Option<usize>,
    overhang_deg: f64,
    pending_export: Option<(ExportKind, PathBuf)>,
    export_confirm: Option<(ExportKind, PathBuf)>,
    // misc
    file: Option<PathBuf>,
    status: String,
    uniform_scale: bool,
    show_array: bool,
    array_count: u32,
    array_offset: DVec3,
    array_circular: bool,
    array_radius_angle: f64,
    show_help: bool,
    screenshot: Option<(PathBuf, u32)>,
}

fn rgba(c: [u8; 3], a: f32) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a]
}

fn c32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, doc: Document, file: Option<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        cc.egui_ctx.all_styles_mut(|style| style.spacing.item_spacing = Vec2::new(6.0, 5.0));
        let committed = doc.clone();
        App {
            doc,
            cache: MeshCache::default(),
            selection: vec![],
            undo: vec![],
            redo: vec![],
            committed,
            changed: false,
            clipboard: vec![],
            camera: Camera::default(),
            tool: Tool::Select,
            gizmo: Gizmo::default(),
            gizmo_kind: GizmoKind::Move,
            gizmo_start: None,
            gizmo_grab: false,
            snap: 1.0,
            renderer: Arc::new(Mutex::new(Renderer::default())),
            entries: vec![],
            geometry_dirty: true,
            drag: Drag::None,
            hover: None,
            paint_slot: 2,
            brush_mm: 2.0,
            fill_angle: 30.0,
            erase: false,
            refine_mm: 1.0,
            measure: vec![],
            report: None,
            check_rx: None,
            selected_issue: None,
            overhang_deg: 45.0,
            pending_export: None,
            export_confirm: None,
            file,
            status: "Welcome to PrintForge — pick a shape on the left to start.".into(),
            uniform_scale: false,
            show_array: false,
            array_count: 4,
            array_offset: DVec3::new(25.0, 0.0, 0.0),
            array_circular: false,
            array_radius_angle: 360.0,
            show_help: false,
            screenshot: screenshot.map(|p| (p, 0)),
        }
    }

    // ---------------------------------------------------------- doc helpers

    fn touch(&mut self) {
        self.changed = true;
        self.geometry_dirty = true;
        self.report = None;
        self.selected_issue = None;
    }

    fn commit_if_idle(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input() && ctx.input(|i| i.events.is_empty());
        if self.changed && !busy {
            let prev = std::mem::replace(&mut self.committed, self.doc.clone());
            self.undo.push(prev);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.changed = false;
        }
    }

    fn do_undo(&mut self) {
        if self.changed {
            let cur = std::mem::replace(&mut self.doc, self.committed.clone());
            self.redo.push(cur);
            self.changed = false;
        } else if let Some(prev) = self.undo.pop() {
            let cur = std::mem::replace(&mut self.doc, prev);
            self.redo.push(cur);
            self.committed = self.doc.clone();
        }
        self.after_history();
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let cur = std::mem::replace(&mut self.doc, next);
            self.undo.push(cur);
            self.committed = self.doc.clone();
            self.changed = false;
        }
        self.after_history();
    }

    fn after_history(&mut self) {
        let ids: Vec<u64> = self.doc.objects.iter().map(|o| o.id).collect();
        self.selection.retain(|s| ids.contains(s));
        self.geometry_dirty = true;
        self.report = None;
    }

    fn selected(&self) -> Vec<&Object> {
        self.doc.objects.iter().filter(|o| self.selection.contains(&o.id)).collect()
    }

    fn world_bounds(&mut self, id: u64) -> Aabb {
        if !self.geometry_dirty {
            if let Some(e) = self.entries.iter().find(|e| e.id == id) {
                return e.mesh.bounds();
            }
        }
        match self.doc.find(id) {
            Some(o) => {
                let o = o.clone();
                self.cache.world_mesh(&o).bounds()
            }
            None => Aabb::empty(),
        }
    }

    fn selection_bounds(&mut self) -> Aabb {
        let ids = self.selection.clone();
        ids.iter().fold(Aabb::empty(), |b, &id| {
            let wb = self.world_bounds(id);
            b.union(&wb)
        })
    }

    pub fn add_shape(&mut self, info_idx: usize) {
        let info = &LIBRARY[info_idx];
        let shape = (info.shape)();
        let mut size = DVec3::from(info.default_size);
        if let Some(a) = shape.text_aspect() {
            size.x = size.y * a;
        }
        let id = self.doc.alloc_id();
        let mut o = Object::new(id, info.name, Kind::Shape(shape), size);
        o.color = (self.doc.objects.len() % self.doc.filaments.len().max(1)) as u8;
        // place next to existing objects, Tinkercad-style
        let mut x = 0.0;
        let used: Vec<Aabb> = self.doc.objects.clone().iter().map(|o| self.cache.world_mesh(o).bounds()).collect();
        for _ in 0..40 {
            let b = Aabb { min: DVec3::new(x - size.x / 2.0, -size.y / 2.0, 0.0), max: DVec3::new(x + size.x / 2.0, size.y / 2.0, size.z) };
            if !used.iter().any(|u| u.overlaps(&b, -1.0)) {
                break;
            }
            x += size.x.max(10.0) + 5.0;
        }
        o.pos = DVec3::new(x, 0.0, 0.0);
        self.doc.objects.push(o);
        self.selection = vec![id];
        self.touch();
        self.status = format!("Added {}.", info.name);
    }

    fn delete_selection(&mut self) {
        let sel = self.selection.clone();
        self.doc.objects.retain(|o| !sel.contains(&o.id) || o.locked);
        self.selection.clear();
        self.touch();
    }

    fn copy(&mut self) {
        self.clipboard = self.selected().into_iter().cloned().collect();
        self.status = format!("Copied {} object(s).", self.clipboard.len());
    }

    fn paste(&mut self, offset: DVec3) {
        if self.clipboard.is_empty() {
            return;
        }
        let mut new = vec![];
        for o in self.clipboard.clone() {
            let mut o = o;
            o.id = self.doc.alloc_id();
            o.pos += offset;
            o.locked = false;
            new.push(o.id);
            self.doc.objects.push(o);
        }
        self.selection = new;
        self.touch();
    }

    fn duplicate(&mut self) {
        self.copy();
        self.paste(DVec3::new(self.snap.max(1.0) * 5.0, self.snap.max(1.0) * 5.0, 0.0));
    }

    fn set_hole(&mut self, hole: bool) {
        for id in self.selection.clone() {
            if let Some(o) = self.doc.find_mut(id) {
                o.hole = hole;
            }
        }
        self.touch();
    }

    fn group(&mut self) {
        let sel = self.selection.clone();
        if sel.len() < 2 {
            self.status = "Select two or more objects to group (Shift-click to add).".into();
            return;
        }
        if let Some(id) = self.doc.group(&sel, &mut self.cache) {
            self.selection = vec![id];
            let warn = self.cache.warnings.values().flatten().cloned().collect::<Vec<_>>();
            self.status = if warn.is_empty() { "Grouped.".into() } else { format!("Grouped with warnings: {}", warn.join("; ")) };
            self.touch();
        }
    }

    fn ungroup(&mut self) {
        let mut new = vec![];
        for id in self.selection.clone() {
            new.extend(self.doc.ungroup(id, &mut self.cache));
        }
        if !new.is_empty() {
            self.selection = new;
            self.touch();
        }
    }

    fn drop_to_bed(&mut self) {
        for id in self.selection.clone() {
            let b = self.world_bounds(id);
            if let Some(o) = self.doc.find_mut(id) {
                o.pos.z -= b.min.z;
            }
        }
        self.touch();
    }

    fn center_on_bed(&mut self) {
        let b = self.selection_bounds();
        if b.is_empty() {
            return;
        }
        let d = DVec3::new(-b.center().x, -b.center().y, -b.min.z);
        for id in self.selection.clone() {
            if let Some(o) = self.doc.find_mut(id) {
                o.pos += d;
            }
        }
        self.touch();
    }

    fn mirror(&mut self, axis: usize) {
        for id in self.selection.clone() {
            if let Some(o) = self.doc.find_mut(id) {
                o.mirror[axis] = !o.mirror[axis];
            }
        }
        self.touch();
    }

    fn rotate_sel(&mut self, axis: DVec3, deg: f64) {
        let q = DQuat::from_axis_angle(axis, deg.to_radians());
        let b = self.selection_bounds();
        let pivot = b.center();
        let multi = self.selection.len() > 1;
        for id in self.selection.clone() {
            if let Some(o) = self.doc.find_mut(id) {
                let half = DVec3::new(0.0, 0.0, o.size.z / 2.0);
                let c = o.pos + half;
                let nc = if multi { pivot + q * (c - pivot) } else { c };
                o.set_rotation(q * o.rotation());
                o.pos = nc - half;
            }
        }
        self.touch();
    }

    fn align(&mut self, axis: usize, mode: i32) {
        let b = self.selection_bounds();
        let target = match mode { -1 => b.min[axis], 0 => b.center()[axis], _ => b.max[axis] };
        for id in self.selection.clone() {
            let ob = self.world_bounds(id);
            let cur = match mode { -1 => ob.min[axis], 0 => ob.center()[axis], _ => ob.max[axis] };
            if let Some(o) = self.doc.find_mut(id) {
                o.pos[axis] += target - cur;
            }
        }
        self.touch();
    }

    fn arrange(&mut self) {
        let bed = self.doc.printer.bed_multi;
        let gap = 5.0;
        let ids: Vec<u64> = self.doc.objects.iter().filter(|o| o.visible && !o.hole).map(|o| o.id).collect();
        let mut items: Vec<(u64, Aabb)> = ids.iter().map(|&id| (id, self.world_bounds(id))).collect();
        items.sort_by(|a, b| b.1.size().y.partial_cmp(&a.1.size().y).unwrap());
        let (mut x, mut y, mut row_h) = (-bed[0] / 2.0, -bed[1] / 2.0, 0.0f64);
        for (id, b) in items {
            let s = b.size();
            if x + s.x > bed[0] / 2.0 && x > -bed[0] / 2.0 {
                x = -bed[0] / 2.0;
                y += row_h + gap;
                row_h = 0.0;
            }
            let d = DVec3::new(x + s.x / 2.0 - b.center().x, y + s.y / 2.0 - b.center().y, -b.min.z);
            if let Some(o) = self.doc.find_mut(id) {
                o.pos += d;
            }
            x += s.x + gap;
            row_h = row_h.max(s.y);
        }
        // centre the whole layout
        let all: Vec<u64> = self.doc.objects.iter().filter(|o| o.visible && !o.hole).map(|o| o.id).collect();
        let tb = all.iter().fold(Aabb::empty(), |acc, &id| {
            let b = self.world_bounds(id);
            acc.union(&b)
        });
        if !tb.is_empty() {
            for id in all {
                if let Some(o) = self.doc.find_mut(id) {
                    o.pos.x -= tb.center().x;
                    o.pos.y -= tb.center().y;
                }
            }
        }
        self.touch();
        self.status = "Arranged all parts on the plate.".into();
    }

    fn make_array(&mut self) {
        let base: Vec<Object> = self.selected().into_iter().cloned().collect();
        if base.is_empty() {
            return;
        }
        let pivot = self.selection_bounds().center();
        let mut new = vec![];
        for k in 1..self.array_count.max(2) {
            for o in &base {
                let mut c = o.clone();
                c.id = self.doc.alloc_id();
                c.locked = false;
                if self.array_circular {
                    let ang = self.array_radius_angle / self.array_count as f64 * k as f64;
                    let q = DQuat::from_rotation_z(ang.to_radians());
                    let half = DVec3::new(0.0, 0.0, c.size.z / 2.0);
                    let cc = c.pos + half;
                    let centre = DVec3::new(pivot.x + self.array_offset.x, pivot.y + self.array_offset.y, cc.z);
                    c.pos = centre + q * (cc - centre) - half;
                    c.set_rotation(q * c.rotation());
                } else {
                    c.pos += self.array_offset * k as f64;
                }
                new.push(c.id);
                self.doc.objects.push(c);
            }
        }
        self.selection.extend(new);
        self.touch();
    }

    /// Convert a selected object into a finely subdivided mesh so paint can follow detail.
    fn refine_for_paint(&mut self) {
        let Some(&id) = self.selection.first() else { return };
        let Some(o) = self.doc.find(id).cloned() else { return };
        let mut m = (*self.cache.base_mesh(&o)).clone();
        if let Some(p) = &o.paint {
            if p.len() == m.colors.len() {
                for (c, &pc) in m.colors.iter_mut().zip(p) {
                    if pc != NO_COLOR {
                        *c = pc;
                    }
                }
            }
        }
        let r = m.subdivide(self.refine_mm.max(0.2), 3_000_000);
        let n = r.tris.len();
        if let Some(o) = self.doc.find_mut(id) {
            o.kind = Kind::Mesh(Arc::new(r));
            o.paint = None;
        }
        self.status = format!("Refined to {n} triangles for painting (shape is now a mesh; size/position still editable).");
        self.touch();
    }

    fn repair_selection(&mut self) {
        for id in self.selection.clone() {
            let Some(o) = self.doc.find(id).cloned() else { continue };
            let m = self.cache.colored_mesh(&o);
            let (r, st) = m.repair(1e-4);
            let (nm, size, _off) = normalize_mesh(r);
            if let Some(o) = self.doc.find_mut(id) {
                o.kind = Kind::Mesh(Arc::new(nm));
                o.size = size;
                o.paint = None;
            }
            self.status = format!(
                "Repair: welded {} vertices, removed {} bad faces, flipped {}, closed {} holes.",
                st.welded, st.removed_faces, st.flipped_faces, st.holes_filled
            );
        }
        self.touch();
    }

    // ---------------------------------------------------------- files

    fn new_doc(&mut self) {
        self.doc = Document::default();
        self.committed = self.doc.clone();
        self.undo.clear();
        self.redo.clear();
        self.selection.clear();
        self.file = None;
        self.geometry_dirty = true;
        self.report = None;
    }

    fn open(&mut self) {
        let Some(p) = rfd::FileDialog::new().add_filter("PrintForge project", &["pforge"]).pick_file() else { return };
        match io::load_project(&p) {
            Ok(d) => {
                self.new_doc();
                self.doc = d;
                self.committed = self.doc.clone();
                self.status = format!("Opened {}", p.display());
                self.file = Some(p);
            }
            Err(e) => self.status = format!("Couldn't open: {e}"),
        }
    }

    fn save(&mut self, as_new: bool) {
        let path = if as_new || self.file.is_none() {
            rfd::FileDialog::new().add_filter("PrintForge project", &["pforge"]).set_file_name("design.pforge").save_file()
        } else {
            self.file.clone()
        };
        let Some(p) = path else { return };
        let p = if p.extension().is_none() { p.with_extension("pforge") } else { p };
        match io::save_project(&p, &self.doc) {
            Ok(()) => {
                self.status = format!("Saved {}", p.display());
                self.file = Some(p);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn import(&mut self) {
        let Some(paths) = rfd::FileDialog::new().add_filter("3D models", &["stl", "obj", "3mf"]).pick_files() else { return };
        for p in paths {
            match io::import_file(&p) {
                Ok(r) => self.add_imported(r),
                Err(e) => self.status = format!("Import failed for {}: {e}", p.display()),
            }
        }
    }

    pub fn add_imported(&mut self, r: io::ImportResult) {
        // map imported colours onto filament slots
        let mut slot_of: Vec<u8> = Vec::new();
        for c in &r.colors {
            let slot = match self.doc.filaments.iter().position(|f| f.color == *c) {
                Some(i) => i,
                None if self.doc.filaments.len() < 24 => {
                    self.doc.filaments.push(Filament { name: format!("Imported {}", self.doc.filaments.len() + 1), color: *c, material: "PLA".into() });
                    self.doc.filaments.len() - 1
                }
                None => self
                    .doc
                    .filaments
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, f)| (0..3).map(|k| (f.color[k] as i32 - c[k] as i32).pow(2)).sum::<i32>())
                    .map(|x| x.0)
                    .unwrap_or(0),
            };
            slot_of.push(slot as u8);
        }
        let all = r.objects.iter().fold(Aabb::empty(), |b, o| b.union(&o.mesh.bounds()));
        let shift = DVec3::new(-all.center().x, -all.center().y, -all.min.z);
        let mut ids = vec![];
        let mut warn_units = false;
        for mut ob in r.objects {
            for c in &mut ob.mesh.colors {
                if *c != NO_COLOR {
                    *c = slot_of.get(*c as usize).copied().unwrap_or(0);
                }
            }
            ob.mesh.translate(shift);
            let (m, size, off) = normalize_mesh(ob.mesh);
            if size.max_element() < 3.0 {
                warn_units = true;
            }
            let id = self.doc.alloc_id();
            let mut o = Object::new(id, &ob.name, Kind::Mesh(Arc::new(m)), size);
            o.pos = off;
            o.color = 0;
            ids.push(id);
            self.doc.objects.push(o);
        }
        self.status = format!("Imported {} object(s).{}", ids.len(), if warn_units { " It's tiny — it may be in inches or metres; scale it up in the inspector." } else { "" });
        self.selection = ids;
        self.touch();
    }

    fn request_export(&mut self, kind: ExportKind) {
        let (filter, ext, def) = match kind {
            ExportKind::Bambu => ("Bambu Studio project", "3mf", "design_bambu_H2C.3mf"),
            ExportKind::Generic3mf => ("3MF with colours", "3mf", "design.3mf"),
            ExportKind::Stl => ("STL", "stl", "design.stl"),
            ExportKind::Obj => ("OBJ with colours", "obj", "design.obj"),
        };
        let Some(p) = rfd::FileDialog::new().add_filter(filter, &[ext]).set_file_name(def).save_file() else { return };
        let p = if p.extension().is_none() { p.with_extension(ext) } else { p };
        self.pending_export = Some((kind, p));
        self.start_check();
    }

    fn do_export(&mut self, kind: ExportKind, p: PathBuf) {
        let objs = io::collect_export(&self.doc, &mut self.cache);
        let r = match kind {
            ExportKind::Bambu => io::write_bambu_3mf(&p, &self.doc, &objs),
            ExportKind::Generic3mf => io::write_generic_3mf(&p, &self.doc, &objs),
            ExportKind::Stl => io::write_stl(&p, &objs),
            ExportKind::Obj => io::write_obj(&p, &self.doc, &objs),
        };
        self.status = match r {
            Ok(()) => match kind {
                ExportKind::Bambu => format!("Exported {} — open it in Bambu Studio and press Slice; colours are already assigned.", p.display()),
                ExportKind::Stl => format!("Exported {} (STL has no colour information).", p.display()),
                _ => format!("Exported {}", p.display()),
            },
            Err(e) => format!("Export failed: {e}"),
        };
    }

    // ---------------------------------------------------------- checking

    pub fn build_check_input(&mut self) -> CheckInput {
        let mut objects = vec![];
        for o in self.doc.objects.clone() {
            if o.visible && !o.hole {
                let mut m = self.cache.world_mesh(&o);
                m.fill_unset_color(o.color);
                objects.push((o.id, o.name.clone(), m));
            }
        }
        let loose_holes = self.doc.objects.iter().filter(|o| o.visible && o.hole).map(|o| o.name.clone()).collect();
        let hidden = self.doc.objects.iter().filter(|o| !o.visible).map(|o| o.name.clone()).collect();
        CheckInput {
            objects,
            loose_holes,
            hidden,
            csg_warnings: self.cache.warnings.values().flatten().cloned().collect(),
            filament_names: self.doc.filaments.iter().map(|f| f.name.clone()).collect(),
            printer: self.doc.printer.clone(),
            overhang_deg: self.overhang_deg,
        }
    }

    fn start_check(&mut self) {
        let input = self.build_check_input();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(check::run(&input));
        });
        self.check_rx = Some(rx);
        self.status = "Checking for print problems…".into();
    }

    fn poll_check(&mut self) {
        let Some(rx) = &self.check_rx else { return };
        if let Ok(r) = rx.try_recv() {
            self.check_rx = None;
            let errors = r.issues.iter().filter(|i| i.severity == Severity::Error).count();
            let warns = r.issues.iter().filter(|i| i.severity == Severity::Warning).count();
            self.status = format!("Check finished: {errors} error(s), {warns} warning(s).");
            self.report = Some(r);
            self.selected_issue = None;
            self.geometry_dirty = true;
            if let Some((k, p)) = self.pending_export.take() {
                if errors > 0 {
                    self.export_confirm = Some((k, p));
                } else {
                    self.do_export(k, p);
                }
            }
        }
    }

    // ---------------------------------------------------------- world / rendering

    fn refresh_world(&mut self) {
        if !self.geometry_dirty {
            return;
        }
        self.geometry_dirty = false;
        let mut old: HashMap<u64, Entry> = self.entries.drain(..).map(|e| (e.id, e)).collect();
        for o in self.doc.objects.clone() {
            if !o.visible {
                continue;
            }
            let mut h = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            o.geom_key().hash(&mut h);
            for v in [o.pos, o.rot].iter().flat_map(|v| v.to_array()) {
                v.to_bits().hash(&mut h);
            }
            o.mirror.hash(&mut h);
            let key = h.finish();
            let mesh = self.cache.world_mesh(&o);
            // picking structure is rebuilt lazily (not every frame while dragging)
            let bvh = match old.remove(&o.id) {
                Some(e) if e.key == key && e.mesh.tris.len() == mesh.tris.len() => e.bvh,
                _ => None,
            };
            self.entries.push(Entry { id: o.id, key, hole: o.hole, mesh, bvh });
        }
        self.cache.gc();
        self.rebuild_batches();
    }

    fn rebuild_batches(&mut self) {
        let mut solid = Batch::new(Pass::Opaque);
        let mut holes = Batch::new(Pass::Transparent);
        let mut lines = Batch::new(Pass::Lines);
        let mut overlay = Batch::new(Pass::Overlay);
        let fil = &self.doc.filaments;
        for e in &self.entries {
            let sel = self.selection.contains(&e.id);
            for t in 0..e.mesh.tris.len() {
                let [a, b, c] = e.mesh.tri_points(t);
                let (a, b, c) = (a.as_vec3(), b.as_vec3(), c.as_vec3());
                if e.hole {
                    holes.tri(a, b, c, [0.55, 0.58, 0.62, if sel { 0.45 } else { 0.3 }]);
                } else {
                    let ci = e.mesh.colors[t];
                    let col = fil.get(ci as usize).map(|f| f.color).unwrap_or([200, 200, 200]);
                    solid.tri(a, b, c, rgba(col, 1.0));
                }
            }
            if sel {
                let bb = e.mesh.bounds();
                box_lines(&mut lines, bb, [0.1, 0.45, 0.95, 1.0]);
            }
        }
        // issue highlight
        if let (Some(r), Some(i)) = (&self.report, self.selected_issue) {
            if let Some(issue) = r.issues.get(i) {
                if let Some(e) = issue.object.and_then(|id| self.entries.iter().find(|e| e.id == id)) {
                    for &t in &issue.tris {
                        if (t as usize) < e.mesh.tris.len() {
                            let [a, b, c] = e.mesh.tri_points(t as usize);
                            overlay.tri(a.as_vec3(), b.as_vec3(), c.as_vec3(), [1.0, 0.1, 0.6, 1.0]);
                        }
                    }
                }
            }
        }
        // bed
        let p = &self.doc.printer;
        let (hx, hy) = (330.0f32 / 2.0, 320.0f32 / 2.0);
        let z = -0.05;
        let bc = [0.82, 0.84, 0.87, 1.0];
        solid.tri(Vec3::new(-hx, -hy, z), Vec3::new(hx, -hy, z), Vec3::new(hx, hy, z), bc);
        solid.tri(Vec3::new(-hx, -hy, z), Vec3::new(hx, hy, z), Vec3::new(-hx, hy, z), bc);
        let gc = [0.62, 0.66, 0.72, 0.7];
        let mut x = -150.0f32;
        while x <= 150.0 {
            lines.line(Vec3::new(x, -hy, 0.0), Vec3::new(x, hy, 0.0), if x == 0.0 { [0.45, 0.5, 0.6, 1.0] } else { gc });
            x += 10.0;
        }
        let mut y = -160.0f32;
        while y <= 160.0 {
            lines.line(Vec3::new(-hx, y, 0.0), Vec3::new(hx, y, 0.0), if y == 0.0 { [0.45, 0.5, 0.6, 1.0] } else { gc });
            y += 10.0;
        }
        let (mx, my) = (p.bed_multi[0] as f32 / 2.0, p.bed_multi[1] as f32 / 2.0);
        let rc = [0.2, 0.55, 0.3, 1.0];
        for (a, b) in [((-mx, -my), (mx, -my)), ((mx, -my), (mx, my)), ((mx, my), (-mx, my)), ((-mx, my), (-mx, -my))] {
            lines.line(Vec3::new(a.0, a.1, 0.02), Vec3::new(b.0, b.1, 0.02), rc);
        }
        lines.line(Vec3::new(-hx, -hy, 0.03), Vec3::new(-hx + 30.0, -hy, 0.03), [0.9, 0.1, 0.1, 1.0]);
        lines.line(Vec3::new(-hx, -hy, 0.03), Vec3::new(-hx, -hy + 30.0, 0.03), [0.1, 0.7, 0.1, 1.0]);
        lines.line(Vec3::new(-hx, -hy, 0.03), Vec3::new(-hx, -hy, 30.0), [0.1, 0.3, 0.9, 1.0]);
        self.renderer.lock().unwrap().set_batches(vec![solid, lines, overlay, holes]);
    }

    fn pick(&mut self, rect: Rect, pos: Pos2) -> Option<Hit> {
        let (o, d) = self.camera.ray(rect, pos);
        let mut best: Option<(f64, Hit)> = None;
        for (i, e) in self.entries.iter_mut().enumerate() {
            let bvh = e.bvh.get_or_insert_with(|| Arc::new(Bvh::build(&e.mesh))).clone();
            if let Some((t, tri)) = bvh.raycast(&e.mesh, o, d, None) {
                // prefer solids over holes when both are hit at similar depth
                let t_adj = if e.hole { t + 0.01 } else { t };
                if best.as_ref().map(|b| t_adj < b.0).unwrap_or(true) {
                    best = Some((t_adj, Hit { entry: i, tri, point: o + d * t }));
                }
            }
        }
        best.map(|b| b.1)
    }

    // ---------------------------------------------------------- painting

    fn paint_at(&mut self, hit: &Hit, fill: bool) {
        let id = self.entries[hit.entry].id;
        let Some(o) = self.doc.find(id).cloned() else { return };
        if o.hole || o.locked {
            return;
        }
        let local = (*self.cache.base_mesh(&o)).clone();
        let n = local.tris.len();
        let mut paint = o.paint.clone().filter(|p| p.len() == n).unwrap_or_else(|| vec![NO_COLOR; n]);
        let value = if self.erase { NO_COLOR } else { self.paint_slot };
        let inv = o.transform().inverse();
        let lp = inv.transform_point3(hit.point);
        let mut changed = false;
        if fill {
            let nb = local.tri_neighbors();
            let start = hit.tri as usize;
            if start >= n {
                return;
            }
            let (n0, _) = local.tri_normal_area(start);
            let lim = self.fill_angle.to_radians().cos();
            let mut stack = vec![start];
            let mut seen = vec![false; n];
            seen[start] = true;
            while let Some(t) = stack.pop() {
                if paint[t] != value {
                    paint[t] = value;
                    changed = true;
                }
                let (nt, _) = local.tri_normal_area(t);
                for &x in &nb[t] {
                    if x == u32::MAX || seen[x as usize] {
                        continue;
                    }
                    let (nx, _) = local.tri_normal_area(x as usize);
                    let ok = if self.fill_angle >= 180.0 { true } else { nx.dot(nt) >= lim * 0.999 || nx.dot(n0) >= lim };
                    if ok {
                        seen[x as usize] = true;
                        stack.push(x as usize);
                    }
                }
            }
        } else {
            let r2 = self.brush_mm * self.brush_mm;
            for t in 0..n {
                let pts = local.tri_points(t);
                let near = pts.iter().any(|p| p.distance_squared(lp) <= r2) || local.tri_centroid(t).distance_squared(lp) <= r2 || t == hit.tri as usize;
                if near && paint[t] != value {
                    paint[t] = value;
                    changed = true;
                }
            }
        }
        if changed {
            if let Some(o) = self.doc.find_mut(id) {
                o.paint = if paint.iter().all(|&p| p == NO_COLOR) { None } else { Some(paint) };
            }
            self.touch();
        }
    }

    // ---------------------------------------------------------- UI: panels

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.new_doc();
                    ui.close();
                }
                if ui.button("Open project…").clicked() {
                    self.open();
                    ui.close();
                }
                if ui.button("Save").clicked() {
                    self.save(false);
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    self.save(true);
                    ui.close();
                }
                ui.separator();
                if ui.button("Import STL / OBJ / 3MF…").clicked() {
                    self.import();
                    ui.close();
                }
                ui.separator();
                if ui.button("Export for Bambu Studio (H2C, colour) …").clicked() {
                    self.request_export(ExportKind::Bambu);
                    ui.close();
                }
                if ui.button("Export 3MF with colours (other slicers)…").clicked() {
                    self.request_export(ExportKind::Generic3mf);
                    ui.close();
                }
                if ui.button("Export STL…").clicked() {
                    self.request_export(ExportKind::Stl);
                    ui.close();
                }
                if ui.button("Export OBJ + MTL…").clicked() {
                    self.request_export(ExportKind::Obj);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Undo").clicked() {
                    self.do_undo();
                    ui.close();
                }
                if ui.button("Redo").clicked() {
                    self.do_redo();
                    ui.close();
                }
                ui.separator();
                if ui.button("Copy").clicked() {
                    self.copy();
                    ui.close();
                }
                if ui.button("Paste").clicked() {
                    self.paste(DVec3::new(10.0, 10.0, 0.0));
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    self.duplicate();
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    self.delete_selection();
                    ui.close();
                }
                if ui.button("Select all").clicked() {
                    self.selection = self.doc.objects.iter().map(|o| o.id).collect();
                    self.geometry_dirty = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Pattern / array copies…").clicked() {
                    self.show_array = true;
                    ui.close();
                }
                if ui.button("Arrange all on plate").clicked() {
                    self.arrange();
                    ui.close();
                }
                if ui.button("Repair selected mesh").clicked() {
                    self.repair_selection();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                for (n, y, p) in [("Home", -60.0, 32.0), ("Top", -90.0, 90.0), ("Front", -90.0, 0.0), ("Right", 0.0, 0.0), ("Back", 90.0, 0.0), ("Left", 180.0, 0.0)] {
                    if ui.button(n).clicked() {
                        self.camera.yaw = y;
                        self.camera.pitch = p;
                        ui.close();
                    }
                }
                if ui.button("Fit all (F)").clicked() {
                    self.fit_view(false);
                    ui.close();
                }
                ui.checkbox(&mut self.camera.ortho, "Orthographic");
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Controls & shortcuts").clicked() {
                    self.show_help = true;
                    ui.close();
                }
            });
            ui.separator();
            let mut tool = self.tool;
            ui.selectable_value(&mut tool, Tool::Select, "Select");
            ui.selectable_value(&mut tool, Tool::Paint, "Paint");
            ui.selectable_value(&mut tool, Tool::Fill, "Fill");
            ui.selectable_value(&mut tool, Tool::LayFlat, "Lay flat");
            ui.selectable_value(&mut tool, Tool::Measure, "Measure");
            if tool != self.tool {
                self.tool = tool;
                self.measure.clear();
            }
            ui.separator();
            if self.tool == Tool::Select {
                ui.selectable_value(&mut self.gizmo_kind, GizmoKind::Move, "Move");
                ui.selectable_value(&mut self.gizmo_kind, GizmoKind::Rotate, "Rotate");
                ui.selectable_value(&mut self.gizmo_kind, GizmoKind::Scale, "Scale");
                ui.selectable_value(&mut self.gizmo_kind, GizmoKind::Off, "No gizmo");
                ui.separator();
            }
            if ui.button("Group").on_hover_text("Combine selected (holes cut solids) — Ctrl/Cmd+G").clicked() {
                self.group();
            }
            if ui.button("Ungroup").on_hover_text("Ctrl/Cmd+Shift+G").clicked() {
                self.ungroup();
            }
            ui.separator();
            ui.label("Snap");
            egui::ComboBox::from_id_salt("snap").width(60.0).selected_text(if self.snap == 0.0 { "off".to_string() } else { format!("{}", self.snap) }).show_ui(ui, |ui| {
                for s in [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0] {
                    ui.selectable_value(&mut self.snap, s, if s == 0.0 { "off".to_string() } else { format!("{s} mm") });
                }
            });
            ui.separator();
            let checking = self.check_rx.is_some();
            if ui.add_enabled(!checking, egui::Button::new(RichText::new("✔ Check printability").strong())).clicked() {
                self.start_check();
            }
            if ui.button(RichText::new("⬆ Export to Bambu").strong().color(Color32::from_rgb(20, 120, 60))).clicked() {
                self.request_export(ExportKind::Bambu);
            }
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Shapes");
            ui.label(RichText::new("Click to add. Make any shape a Hole to cut with it.").small().weak());
            egui::Grid::new("shapes").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
                for (i, info) in LIBRARY.iter().enumerate() {
                    if ui.add_sized([108.0, 26.0], egui::Button::new(info.name)).clicked() {
                        self.add_shape(i);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            if ui.add_sized([220.0, 24.0], egui::Button::new("Import STL / OBJ / 3MF…")).clicked() {
                self.import();
            }
            ui.separator();
            ui.heading("Filaments");
            ui.label(RichText::new("These become the filament list in Bambu Studio. Set them to match your AMS spools.").small().weak());
            let mut remove = false;
            let n = self.doc.filaments.len();
            let mut changed = false;
            for i in 0..n {
                ui.horizontal(|ui| {
                    let f = &mut self.doc.filaments[i];
                    ui.label(format!("{:>2}", i + 1));
                    if ui.color_edit_button_srgb(&mut f.color).changed() {
                        changed = true;
                    }
                    if ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(90.0)).changed() {
                        changed = true;
                    }
                    let active = self.paint_slot as usize == i;
                    if ui.radio(active, "brush").clicked() {
                        self.paint_slot = i as u8;
                        self.erase = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(n < 24, egui::Button::new("+ Add")).clicked() {
                    self.doc.filaments.push(Filament { name: format!("Filament {}", n + 1), color: [128, 128, 128], material: "PLA".into() });
                    changed = true;
                }
                if ui.add_enabled(n > 1, egui::Button::new("− Remove last")).clicked() {
                    remove = true;
                }
            });
            if remove {
                self.doc.filaments.pop();
                let max = self.doc.filaments.len() as u8 - 1;
                for o in &mut self.doc.objects {
                    o.color = o.color.min(max);
                }
                self.paint_slot = self.paint_slot.min(max);
                changed = true;
            }
            if changed {
                self.touch();
            }
            ui.separator();
            ui.heading("Objects");
            let mut click: Option<(u64, bool)> = None;
            let mut toggles: Vec<(u64, u8)> = vec![];
            for o in &self.doc.objects {
                ui.horizontal(|ui| {
                    if ui.small_button(if o.visible { "show" } else { "hidden" }).on_hover_text("Show/hide").clicked() {
                        toggles.push((o.id, 0));
                    }
                    if ui.small_button(if o.locked { "locked" } else { "lock" }).on_hover_text("Lock").clicked() {
                        toggles.push((o.id, 1));
                    }
                    let col = if o.hole { Color32::GRAY } else { c32(self.doc.filaments.get(o.color as usize).map(|f| f.color).unwrap_or([128; 3])) };
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, col);
                    let label = format!("{}{}", o.name, if o.hole { " (hole)" } else if o.is_group() { " (group)" } else { "" });
                    let resp = ui.selectable_label(self.selection.contains(&o.id), label);
                    if resp.clicked() {
                        click = Some((o.id, ui.input(|i| i.modifiers.shift || i.modifiers.command)));
                    }
                });
            }
            for (id, what) in toggles {
                if let Some(o) = self.doc.find_mut(id) {
                    if what == 0 { o.visible = !o.visible } else { o.locked = !o.locked }
                }
                self.touch();
            }
            if let Some((id, add)) = click {
                self.select_click(Some(id), add);
            }
        });
    }

    fn select_click(&mut self, id: Option<u64>, add: bool) {
        match (id, add) {
            (Some(id), true) => {
                if let Some(p) = self.selection.iter().position(|&s| s == id) {
                    self.selection.remove(p);
                } else {
                    self.selection.push(id);
                }
            }
            (Some(id), false) => self.selection = vec![id],
            (None, false) => self.selection.clear(),
            _ => {}
        }
        self.geometry_dirty = true;
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            match self.tool {
                Tool::Paint | Tool::Fill => self.paint_panel(ui),
                Tool::Measure => {
                    ui.heading("Measure");
                    ui.label("Click two points on your model.");
                    if self.measure.len() == 2 {
                        let d = self.measure[1] - self.measure[0];
                        ui.label(RichText::new(format!("Distance: {:.2} mm", d.length())).strong());
                        ui.label(format!("ΔX {:.2}   ΔY {:.2}   ΔZ {:.2}", d.x.abs(), d.y.abs(), d.z.abs()));
                    }
                    if ui.button("Clear").clicked() {
                        self.measure.clear();
                    }
                }
                Tool::LayFlat => {
                    ui.heading("Lay flat");
                    ui.label("Click a face of an object; it'll be rotated so that face sits on the plate. Great for reducing supports.");
                }
                Tool::Select => {}
            }
            if self.tool == Tool::Select || !self.selection.is_empty() {
                self.inspector(ui);
            }
            ui.separator();
            self.check_panel(ui);
        });
    }

    fn paint_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(if self.tool == Tool::Paint { "Paint brush" } else { "Bucket fill" });
        ui.label("Pick a filament, then click/drag on a model. Painting is per-face; use Refine for finer detail.");
        ui.horizontal_wrapped(|ui| {
            for (i, f) in self.doc.filaments.iter().enumerate() {
                let sel = self.paint_slot as usize == i && !self.erase;
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                ui.painter().rect_filled(r, 4.0, c32(f.color));
                if sel {
                    ui.painter().rect_stroke(r.expand(2.0), 4.0, Stroke::new(2.5, Color32::from_rgb(20, 90, 220)), egui::StrokeKind::Outside);
                }
                if resp.on_hover_text(format!("{} — {}", i + 1, f.name)).clicked() {
                    self.paint_slot = i as u8;
                    self.erase = false;
                }
            }
        });
        ui.checkbox(&mut self.erase, "Eraser (back to object colour)");
        if self.tool == Tool::Paint {
            ui.add(egui::Slider::new(&mut self.brush_mm, 0.2..=30.0).text("brush radius (mm)").logarithmic(true));
        } else {
            ui.add(egui::Slider::new(&mut self.fill_angle, 1.0..=180.0).text("edge angle limit (°)"));
            ui.label(RichText::new("Fill spreads across faces until it meets an edge sharper than this. 180° = whole connected part.").small().weak());
        }
        ui.separator();
        ui.label("Detail for the selected object:");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.refine_mm).range(0.2..=10.0).speed(0.05).suffix(" mm"));
            if ui.button("Refine for painting").on_hover_text("Splits faces so no edge is longer than this, so the brush can paint small areas.").clicked() {
                self.refine_for_paint();
            }
        });
        if ui.button("Clear paint on selection").clicked() {
            for id in self.selection.clone() {
                if let Some(o) = self.doc.find_mut(id) {
                    o.paint = None;
                }
            }
            self.touch();
        }
        ui.separator();
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let sel = self.selection.clone();
        if sel.is_empty() {
            ui.heading("Inspector");
            ui.label("Select an object to edit it.");
            return;
        }
        if sel.len() > 1 {
            ui.heading(format!("{} objects selected", sel.len()));
            ui.horizontal(|ui| {
                if ui.button("Solid").clicked() {
                    self.set_hole(false);
                }
                if ui.button("Hole").clicked() {
                    self.set_hole(true);
                }
                if ui.button("Group").clicked() {
                    self.group();
                }
            });
            self.color_picker(ui, &sel);
            ui.label(RichText::new("Align").strong());
            for (axis, name) in [(0, "X"), (1, "Y"), (2, "Z")] {
                ui.horizontal(|ui| {
                    ui.label(name);
                    if ui.button("min").clicked() {
                        self.align(axis, -1);
                    }
                    if ui.button("centre").clicked() {
                        self.align(axis, 0);
                    }
                    if ui.button("max").clicked() {
                        self.align(axis, 1);
                    }
                });
            }
            self.common_buttons(ui);
            return;
        }
        let id = sel[0];
        let Some(mut o) = self.doc.find(id).cloned() else { return };
        let before = format!("{:?}", (&o.name, &o.kind_debug(), o.size, o.pos, o.rot, o.hole, o.color, &o.mirror));
        ui.horizontal(|ui| {
            ui.heading("Inspector");
            if o.locked {
                ui.label("(locked)");
            }
        });
        ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(200.0));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut o.hole, false, "■ Solid");
            ui.selectable_value(&mut o.hole, true, "▨ Hole");
        });
        let mut keep_colors_changed = false;
        if let Kind::Group { keep_colors, .. } = &mut o.kind {
            if ui.checkbox(keep_colors, "Keep each part's colour").on_hover_text("Off = the whole group uses one colour (Tinkercad's default)").changed() {
                keep_colors_changed = true;
            }
        }
        // colour
        if !o.hole {
            ui.horizontal_wrapped(|ui| {
                ui.label("Colour");
                for (i, f) in self.doc.filaments.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                    ui.painter().rect_filled(r, 3.0, c32(f.color));
                    if o.color as usize == i {
                        ui.painter().rect_stroke(r.expand(1.5), 3.0, Stroke::new(2.0, Color32::from_rgb(20, 90, 220)), egui::StrokeKind::Outside);
                    }
                    if resp.on_hover_text(format!("{} — {}", i + 1, f.name)).clicked() {
                        o.color = i as u8;
                    }
                }
            });
        }
        // shape parameters
        if let Kind::Shape(s) = &mut o.kind {
            ui.label(RichText::new(s.label()).strong());
            shape_params(ui, s);
        } else if let Kind::Mesh(m) = &o.kind {
            ui.label(format!("Mesh · {} triangles", m.tris.len()));
        }
        ui.separator();
        ui.label(RichText::new("Size (mm)").strong());
        let old_size = o.size;
        ui.horizontal(|ui| {
            ui.label("W");
            ui.add(egui::DragValue::new(&mut o.size.x).speed(0.2).range(0.05..=2000.0).max_decimals(2));
            ui.label("D");
            ui.add(egui::DragValue::new(&mut o.size.y).speed(0.2).range(0.05..=2000.0).max_decimals(2));
            ui.label("H");
            ui.add(egui::DragValue::new(&mut o.size.z).speed(0.2).range(0.05..=2000.0).max_decimals(2));
        });
        ui.checkbox(&mut self.uniform_scale, "Keep proportions");
        if self.uniform_scale && o.size != old_size {
            let f = (0..3).find(|&k| o.size[k] != old_size[k]).map(|k| o.size[k] / old_size[k]).unwrap_or(1.0);
            o.size = old_size * f;
        }
        ui.label(RichText::new("Position (mm, bottom centre)").strong());
        ui.horizontal(|ui| {
            ui.label("X");
            ui.add(egui::DragValue::new(&mut o.pos.x).speed(0.2).max_decimals(2));
            ui.label("Y");
            ui.add(egui::DragValue::new(&mut o.pos.y).speed(0.2).max_decimals(2));
            ui.label("Z");
            ui.add(egui::DragValue::new(&mut o.pos.z).speed(0.2).max_decimals(2));
        });
        ui.label(RichText::new("Rotation (°)").strong());
        ui.horizontal(|ui| {
            ui.label("X");
            ui.add(egui::DragValue::new(&mut o.rot.x).speed(1.0).max_decimals(1));
            ui.label("Y");
            ui.add(egui::DragValue::new(&mut o.rot.y).speed(1.0).max_decimals(1));
            ui.label("Z");
            ui.add(egui::DragValue::new(&mut o.rot.z).speed(1.0).max_decimals(1));
        });
        let after = format!("{:?}", (&o.name, &o.kind_debug(), o.size, o.pos, o.rot, o.hole, o.color, &o.mirror));
        if (after != before || keep_colors_changed) && !o.locked {
            if o.size != old_size {
                o.paint = o.paint.take(); // paint survives scaling (same topology)
            }
            if let Some(slot) = self.doc.find_mut(id) {
                *slot = o.clone();
            }
            self.touch();
        }
        ui.horizontal(|ui| {
            ui.label("Rotate 90°:");
            for (n, ax) in [("X", DVec3::X), ("Y", DVec3::Y), ("Z", DVec3::Z)] {
                if ui.small_button(format!("↻{n}")).clicked() {
                    self.rotate_sel(ax, 90.0);
                }
            }
        });
        self.color_picker_paint_info(ui, &o);
        self.common_buttons(ui);
    }

    fn color_picker(&mut self, ui: &mut egui::Ui, sel: &[u64]) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Colour all");
            let fl = self.doc.filaments.clone();
            for (i, f) in fl.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                ui.painter().rect_filled(r, 3.0, c32(f.color));
                if resp.on_hover_text(&f.name).clicked() {
                    for id in sel {
                        if let Some(o) = self.doc.find_mut(*id) {
                            o.color = i as u8;
                        }
                    }
                    self.touch();
                }
            }
        });
    }

    fn color_picker_paint_info(&self, ui: &mut egui::Ui, o: &Object) {
        if let Some(p) = &o.paint {
            let n = p.iter().filter(|&&c| c != NO_COLOR).count();
            ui.label(RichText::new(format!("{n} painted faces")).small());
        }
    }

    fn common_buttons(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Drop to bed").clicked() {
                self.drop_to_bed();
            }
            if ui.button("Centre on bed").clicked() {
                self.center_on_bed();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate();
            }
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Mirror:");
            for (n, ax) in [("X", 0), ("Y", 1), ("Z", 2)] {
                if ui.small_button(n).clicked() {
                    self.mirror(ax);
                }
            }
            if ui.small_button("Array…").clicked() {
                self.show_array = true;
            }
            if ui.small_button("Repair").on_hover_text("Fix holes / flipped faces in imported meshes").clicked() {
                self.repair_selection();
            }
        });
    }

    fn check_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Print check");
        ui.horizontal(|ui| {
            if ui.add_enabled(self.check_rx.is_none(), egui::Button::new("Run check")).clicked() {
                self.start_check();
            }
            if self.check_rx.is_some() {
                ui.spinner();
            }
            ui.add(egui::DragValue::new(&mut self.overhang_deg).range(20.0..=80.0).suffix("° overhang limit"));
        });
        let Some(r) = self.report.clone() else {
            ui.label(RichText::new("Not checked since the last change.").weak());
            return;
        };
        for s in &r.summary {
            ui.label(RichText::new(s).small());
        }
        ui.add_space(4.0);
        let mut clicked = None;
        for (i, is) in r.issues.iter().enumerate() {
            let (icon, col) = match is.severity {
                Severity::Error => ("✖", Color32::from_rgb(200, 30, 30)),
                Severity::Warning => ("⚠", Color32::from_rgb(200, 120, 0)),
                Severity::Info => ("ℹ", Color32::from_rgb(40, 100, 180)),
            };
            let sel = self.selected_issue == Some(i);
            let who = is.object.and_then(|id| self.doc.find(id)).map(|o| format!(" · {}", o.name)).unwrap_or_default();
            let resp = ui.selectable_label(sel, RichText::new(format!("{icon} {}{who}", is.title)).color(col).strong());
            if sel {
                ui.label(RichText::new(&is.detail).small());
                if !is.tris.is_empty() {
                    ui.label(RichText::new("Highlighted in pink in the view.").small().weak());
                }
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.selected_issue = if self.selected_issue == Some(i) { None } else { Some(i) };
            if let Some(id) = r.issues[i].object {
                self.selection = vec![id];
            }
            self.geometry_dirty = true;
        }
    }

    fn fit_view(&mut self, selection_only: bool) {
        let b = if selection_only && !self.selection.is_empty() {
            self.selection_bounds()
        } else {
            self.entries.iter().fold(Aabb::empty(), |a, e| a.union(&e.mesh.bounds()))
        };
        if b.is_empty() {
            self.camera.target = DVec3::new(0.0, 0.0, 10.0);
            self.camera.dist = 320.0;
            return;
        }
        self.camera.target = b.center();
        self.camera.dist = (b.size().length() * 1.8).max(30.0);
    }

    // ---------------------------------------------------------- viewport

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let view = self.camera.view();
        let proj = self.camera.proj(aspect);
        // GL paint
        let renderer = self.renderer.clone();
        let cb = egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| unsafe {
                renderer.lock().unwrap().paint(painter.gl());
            })),
        };
        ui.painter().add(cb);


        // gizmo
        let mut gizmo_active = false;
        if self.tool == Tool::Select && self.gizmo_kind != GizmoKind::Off && !self.selection.is_empty() && !self.selected().iter().any(|o| o.locked) {
            let b = self.selection_bounds();
            let single = if self.selection.len() == 1 { self.doc.find(self.selection[0]).cloned() } else { None };
            let rot = single.as_ref().map(|o| o.rotation()).unwrap_or(DQuat::IDENTITY);
            let modes = match self.gizmo_kind {
                GizmoKind::Move => GizmoMode::all_from_kind(transform_gizmo_egui::config::GizmoModeKind::Translate),
                GizmoKind::Rotate => GizmoMode::all_from_kind(transform_gizmo_egui::config::GizmoModeKind::Rotate),
                _ => GizmoMode::all_from_kind(transform_gizmo_egui::config::GizmoModeKind::Scale),
            };
            let shift = ui.input(|i| i.modifiers.shift);
            self.gizmo.update_config(GizmoConfig {
                view_matrix: view.into(),
                projection_matrix: proj.into(),
                viewport: rect,
                modes,
                orientation: if self.gizmo_kind == GizmoKind::Scale { GizmoOrientation::Local } else { GizmoOrientation::Global },
                snapping: !shift,
                snap_angle: 15f32.to_radians(),
                snap_distance: self.snap.max(0.01) as f32,
                snap_scale: 0.05,
                visuals: GizmoVisuals { gizmo_size: 110.0, stroke_width: 5.0, ..Default::default() },
                ..Default::default()
            });
            let t = transform_gizmo_egui::math::Transform::from_scale_rotation_translation(
                DVec3::ONE,
                if self.gizmo_kind == GizmoKind::Scale { rot } else { DQuat::IDENTITY },
                b.center(),
            );
            if let Some(result) = self.drive_gizmo(ui, &resp, rect, t) {
                gizmo_active = true;
                if self.gizmo_start.is_none() {
                    self.gizmo_start = Some(self.selected().iter().map(|o| (o.id, o.pos, o.rot, o.size)).collect());
                }
                let start = self.gizmo_start.clone().unwrap();
                let pivot = b.center();
                match result {
                    GizmoResult::Translation { total, .. } => {
                        let d = DVec3::from(total);
                        for (id, p, _, _) in &start {
                            if let Some(o) = self.doc.find_mut(*id) {
                                o.pos = *p + d;
                            }
                        }
                    }
                    GizmoResult::Rotation { axis, total, .. } => {
                        let q = DQuat::from_axis_angle(DVec3::from(axis).normalize(), -total);
                        let multi = start.len() > 1;
                        // rotate about the selection centre at drag start
                        let sp: DVec3 = if multi {
                            pivot
                        } else {
                            DVec3::ZERO
                        };
                        for (id, p, r, s) in &start {
                            if let Some(o) = self.doc.find_mut(*id) {
                                let half = DVec3::new(0.0, 0.0, s.z / 2.0);
                                o.rot = *r;
                                let r0 = o.rotation();
                                o.set_rotation(q * r0);
                                let c = *p + half;
                                o.pos = if multi { sp + q * (c - sp) - half } else { *p };
                            }
                        }
                    }
                    GizmoResult::Scale { total } => {
                        let f = DVec3::from(total);
                        for (id, p, _, s) in &start {
                            if let Some(o) = self.doc.find_mut(*id) {
                                o.size = (*s * f).max(DVec3::splat(0.05));
                                let half0 = s.z / 2.0;
                                let half1 = o.size.z / 2.0;
                                o.pos = DVec3::new(p.x, p.y, p.z + half0 - half1);
                            }
                        }
                    }
                    _ => {}
                }
                self.touch();
            }
            if self.gizmo_grab {
                gizmo_active = true;
            }
        } else {
            self.gizmo_grab = false;
        }
        if !gizmo_active && !ui.input(|i| i.pointer.primary_down()) {
            self.gizmo_start = None;
        }

        // hover
        let pointer = resp.hover_pos();
        let dragging_obj = matches!(self.drag, Drag::Move { .. } | Drag::Orbit | Drag::Pan) || gizmo_active;
        self.hover = if dragging_obj { None } else { pointer.and_then(|p| self.pick(rect, p)) };

        // drags
        let mods = ui.input(|i| i.modifiers);
        if !gizmo_active {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                let press = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
                let hit = self.pick(rect, press);
                self.drag = match (&self.tool, hit) {
                    (Tool::Paint, Some(h)) => {
                        self.paint_at(&h, false);
                        Drag::Paint
                    }
                    (Tool::Select, Some(h)) => {
                        let id = self.entries[h.entry].id;
                        if !self.selection.contains(&id) {
                            self.selection = vec![id];
                            self.geometry_dirty = true;
                        }
                        if mods.alt {
                            self.duplicate_in_place();
                        }
                        if self.selected().iter().any(|o| o.locked) {
                            Drag::Orbit
                        } else {
                            let start = self.selected().iter().map(|o| (o.id, o.pos)).collect();
                            Drag::Move { plane_z: h.point.z, start_hit: h.point, start }
                        }
                    }
                    (_, None) if mods.shift => Drag::Box { start: press },
                    _ => Drag::Orbit,
                };
            }
            if resp.drag_started_by(egui::PointerButton::Secondary) {
                self.drag = if mods.shift { Drag::Pan } else { Drag::Orbit };
            }
            if resp.drag_started_by(egui::PointerButton::Middle) {
                self.drag = Drag::Pan;
            }
        }
        let delta = resp.drag_delta();
        match &self.drag {
            Drag::Orbit => {
                self.camera.yaw -= delta.x as f64 * 0.35;
                self.camera.pitch = (self.camera.pitch + delta.y as f64 * 0.35).clamp(-89.9, 89.9);
            }
            Drag::Pan => {
                let s = self.camera.dist * 0.0016;
                let v = self.camera.view().inverse();
                let right = v.transform_vector3(DVec3::X);
                let up = v.transform_vector3(DVec3::Y);
                self.camera.target += (-right * delta.x as f64 + up * delta.y as f64) * s;
            }
            Drag::Move { plane_z, start_hit, start } => {
                if let Some(p) = pointer {
                    let (o, d) = self.camera.ray(rect, p);
                    if d.z.abs() > 1e-6 {
                        let t = (plane_z - o.z) / d.z;
                        if t > 0.0 {
                            let h = o + d * t;
                            let mut dd = h - *start_hit;
                            if self.snap > 0.0 {
                                dd.x = (dd.x / self.snap).round() * self.snap;
                                dd.y = (dd.y / self.snap).round() * self.snap;
                            }
                            for (id, sp) in start.clone() {
                                if let Some(ob) = self.doc.find_mut(id) {
                                    ob.pos = DVec3::new(sp.x + dd.x, sp.y + dd.y, sp.z);
                                }
                            }
                            self.touch();
                        }
                    }
                }
            }
            Drag::Paint => {
                if let Some(h) = self.hover.take() {
                    self.paint_at(&h, false);
                }
            }
            Drag::Box { start } => {
                if let Some(p) = pointer {
                    let r = Rect::from_two_pos(*start, p);
                    ui.painter().rect(r, 0.0, Color32::from_rgba_unmultiplied(40, 110, 230, 30), Stroke::new(1.0, Color32::from_rgb(40, 110, 230)), egui::StrokeKind::Inside);
                }
            }
            Drag::None => {}
        }
        if resp.drag_stopped() {
            if let Drag::Box { start } = self.drag {
                if let Some(p) = pointer {
                    let r = Rect::from_two_pos(start, p);
                    let ids: Vec<u64> = self
                        .entries
                        .iter()
                        .filter(|e| self.camera.project(rect, e.mesh.bounds().center()).map(|q| r.contains(q)).unwrap_or(false))
                        .map(|e| e.id)
                        .collect();
                    self.selection = ids;
                    self.geometry_dirty = true;
                }
            }
            self.drag = Drag::None;
        }

        // clicks
        if resp.clicked() && !gizmo_active {
            let hit = pointer.and_then(|p| self.pick(rect, p));
            match self.tool {
                Tool::Select => {
                    let id = hit.map(|h| self.entries[h.entry].id);
                    self.select_click(id, mods.shift || mods.command);
                }
                Tool::Paint => {
                    if let Some(h) = hit {
                        self.paint_at(&h, false);
                    }
                }
                Tool::Fill => {
                    if let Some(h) = hit {
                        self.paint_at(&h, true);
                    }
                }
                Tool::Measure => {
                    if let Some(h) = hit {
                        if self.measure.len() >= 2 {
                            self.measure.clear();
                        }
                        self.measure.push(h.point);
                    }
                }
                Tool::LayFlat => {
                    if let Some(h) = hit {
                        self.lay_flat(&h);
                    }
                }
            }
        }

        // zoom
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 {
                self.camera.dist = (self.camera.dist * (-(scroll as f64) * 0.0018).exp()).clamp(5.0, 5000.0);
            }
            if zoom != 1.0 {
                self.camera.dist = (self.camera.dist / zoom as f64).clamp(5.0, 5000.0);
            }
        }

        self.refresh_world();

        {
            let mut r = self.renderer.lock().unwrap();
            r.view_proj = (self.camera.proj(aspect) * self.camera.view()).as_mat4();
            r.eye = self.camera.eye().as_vec3();
        }
        // overlays: gizmo draws itself; dimension labels, brush, measure
        let painter = ui.painter_at(rect);
        if self.tool == Tool::Select && !self.selection.is_empty() {
            let b = self.selection_bounds();
            if !b.is_empty() {
                let s = b.size();
                let lbl = |p: DVec3, t: String| {
                    if let Some(q) = self.camera.project(rect, p) {
                        let g = painter.layout_no_wrap(t, egui::FontId::proportional(13.0), Color32::WHITE);
                        let r = Rect::from_center_size(q, g.size() + Vec2::new(8.0, 4.0));
                        painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(20, 70, 160, 210));
                        painter.galley(r.min + Vec2::new(4.0, 2.0), g, Color32::WHITE);
                    }
                };
                lbl(DVec3::new(b.center().x, b.min.y, b.min.z), format!("{:.2}", s.x));
                lbl(DVec3::new(b.max.x, b.center().y, b.min.z), format!("{:.2}", s.y));
                lbl(DVec3::new(b.max.x, b.min.y, b.center().z), format!("{:.2}", s.z));
                if b.min.z.abs() > 0.005 {
                    lbl(DVec3::new(b.min.x, b.min.y, b.min.z), format!("↕ {:.2} above bed", b.min.z));
                }
            }
        }
        if matches!(self.tool, Tool::Paint) {
            if let (Some(h), Some(p)) = (&self.hover, pointer) {
                // approximate brush radius in pixels
                let a = self.camera.project(rect, h.point);
                let v = self.camera.view().inverse().transform_vector3(DVec3::X);
                let b2 = self.camera.project(rect, h.point + v * self.brush_mm);
                if let (Some(a), Some(b2)) = (a, b2) {
                    let col = if self.erase { Color32::BLACK } else { c32(self.doc.filaments[self.paint_slot as usize].color) };
                    painter.circle_stroke(a, a.distance(b2).max(2.0), Stroke::new(2.0, col));
                    painter.circle_stroke(a, a.distance(b2).max(2.0) + 1.5, Stroke::new(1.0, Color32::WHITE));
                }
                let _ = p;
            }
        }
        for (i, m) in self.measure.iter().enumerate() {
            if let Some(q) = self.camera.project(rect, *m) {
                painter.circle_filled(q, 5.0, Color32::from_rgb(230, 60, 20));
                if i == 1 {
                    if let Some(q0) = self.camera.project(rect, self.measure[0]) {
                        painter.line_segment([q0, q], Stroke::new(2.0, Color32::from_rgb(230, 60, 20)));
                        let d = (self.measure[1] - self.measure[0]).length();
                        painter.text(q0.lerp(q, 0.5) + Vec2::new(0.0, -14.0), egui::Align2::CENTER_CENTER, format!("{d:.2} mm"), egui::FontId::proportional(15.0), Color32::from_rgb(160, 30, 10));
                    }
                }
            }
        }
        // view cube-ish buttons
        let area = Rect::from_min_size(rect.right_top() + Vec2::new(-190.0, 8.0), Vec2::new(182.0, 26.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                for (n, y, p) in [("Home", -60.0, 32.0), ("Top", -90.0, 89.99), ("Front", -90.0, 0.0), ("Right", 0.0, 0.0)] {
                    if ui.small_button(n).clicked() {
                        self.camera.yaw = y;
                        self.camera.pitch = p;
                    }
                }
            });
        });
        if self.doc.objects.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Click a shape on the left to begin", egui::FontId::proportional(20.0), Color32::from_gray(110));
        }
    }

    /// Feed the gizmo our own pointer state. (The crate's `interact` puts an input widget
    /// under the cursor every frame, which swallows every click in the viewport.)
    fn drive_gizmo(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, t: transform_gizmo_egui::math::Transform) -> Option<GizmoResult> {
        let cursor = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
        let (pressed, down) = ui.input(|i| (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.button_down(egui::PointerButton::Primary)));
        let cfg = *self.gizmo.config();
        self.gizmo.update_config(GizmoConfig { pixels_per_point: ui.ctx().pixels_per_point(), ..cfg });
        let hovered = resp.hovered() || resp.dragged();
        let result = self.gizmo.update(
            GizmoInteraction { cursor_pos: (cursor.x, cursor.y), hovered, drag_started: pressed && hovered, dragging: down },
            &[t],
        );
        if pressed {
            // grabbed only if the press landed on a gizmo handle
            self.gizmo_grab = hovered && self.gizmo.is_focused();
        }
        if !down {
            self.gizmo_grab = false;
        }
        let dd = self.gizmo.draw();
        ui.painter_at(rect).add(egui::Mesh {
            indices: dd.indices,
            vertices: dd
                .vertices
                .into_iter()
                .zip(dd.colors)
                .map(|(pos, [r, g, b, a])| egui::epaint::Vertex {
                    pos: pos.into(),
                    uv: Pos2::default(),
                    color: egui::Rgba::from_rgba_premultiplied(r, g, b, a).into(),
                })
                .collect(),
            ..Default::default()
        });
        result.map(|r| r.0)
    }

    fn duplicate_in_place(&mut self) {
        self.copy();
        self.paste(DVec3::ZERO);
    }

    fn lay_flat(&mut self, h: &Hit) {
        let e = &self.entries[h.entry];
        let id = e.id;
        let (n, _) = e.mesh.tri_normal_area(h.tri as usize);
        let q = DQuat::from_rotation_arc(n, -DVec3::Z);
        if let Some(o) = self.doc.find_mut(id) {
            if o.locked {
                return;
            }
            let r = o.rotation();
            o.set_rotation(q * r);
        }
        self.selection = vec![id];
        self.drop_to_bed();
        self.status = "Laid flat on the chosen face.".into();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut nudge = DVec3::ZERO;
        let step = if self.snap > 0.0 { self.snap } else { 1.0 };
        ctx.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND;
            let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            if i.consume_key(cmd_shift, Key::Z) || i.consume_key(cmd, Key::Y) {
                self.do_redo();
            }
            if i.consume_key(cmd, Key::Z) {
                self.do_undo();
            }
            if i.consume_key(cmd_shift, Key::G) {
                self.ungroup();
            }
            if i.consume_key(cmd, Key::G) {
                self.group();
            }
            if i.consume_key(cmd, Key::D) {
                self.duplicate();
            }
            if i.consume_key(cmd, Key::C) {
                self.copy();
            }
            if i.consume_key(cmd, Key::V) {
                self.paste(DVec3::new(10.0, 10.0, 0.0));
            }
            if i.consume_key(cmd, Key::S) {
                self.save(false);
            }
            if i.consume_key(cmd, Key::A) {
                self.selection = self.doc.objects.iter().map(|o| o.id).collect();
                self.geometry_dirty = true;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Delete) || i.consume_key(egui::Modifiers::NONE, Key::Backspace) {
                self.delete_selection();
            }
            if i.consume_key(egui::Modifiers::NONE, Key::H) {
                self.set_hole(true);
            }
            if i.consume_key(egui::Modifiers::NONE, Key::S) {
                self.set_hole(false);
            }
            if i.consume_key(egui::Modifiers::NONE, Key::D) {
                self.drop_to_bed();
            }
            if i.consume_key(egui::Modifiers::NONE, Key::F) {
                self.fit_view(true);
            }
            if i.consume_key(egui::Modifiers::NONE, Key::P) {
                self.tool = Tool::Paint;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::B) {
                self.tool = Tool::Fill;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Num1) {
                self.tool = Tool::Select;
                self.gizmo_kind = GizmoKind::Move;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Num2) {
                self.tool = Tool::Select;
                self.gizmo_kind = GizmoKind::Rotate;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Num3) {
                self.tool = Tool::Select;
                self.gizmo_kind = GizmoKind::Scale;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                if self.tool != Tool::Select {
                    self.tool = Tool::Select;
                } else {
                    self.selection.clear();
                    self.geometry_dirty = true;
                }
            }
            let big = if i.modifiers.shift { 10.0 } else { 1.0 };
            for (k, d) in [(Key::ArrowLeft, DVec3::NEG_X), (Key::ArrowRight, DVec3::X), (Key::ArrowUp, DVec3::Y), (Key::ArrowDown, DVec3::NEG_Y)] {
                if i.key_pressed(k) {
                    nudge += if i.modifiers.command { if k == Key::ArrowUp { DVec3::Z } else if k == Key::ArrowDown { DVec3::NEG_Z } else { DVec3::ZERO } } else { d } * step * big;
                }
            }
        });
        if nudge != DVec3::ZERO && !self.selection.is_empty() {
            for id in self.selection.clone() {
                if let Some(o) = self.doc.find_mut(id) {
                    if !o.locked {
                        o.pos += nudge;
                    }
                }
            }
            self.touch();
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        if self.show_array {
            let mut open = true;
            egui::Window::new("Pattern / array").open(&mut open).resizable(false).show(ctx, |ui| {
                ui.radio_value(&mut self.array_circular, false, "Straight line");
                ui.radio_value(&mut self.array_circular, true, "Circle (around Z)");
                ui.add(egui::DragValue::new(&mut self.array_count).range(2..=200).prefix("Total copies: "));
                if self.array_circular {
                    ui.add(egui::DragValue::new(&mut self.array_radius_angle).range(1.0..=360.0).suffix("° total sweep"));
                    ui.label("Centre offset from selection (X, Y):");
                } else {
                    ui.label("Step between copies (X, Y, Z):");
                }
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.array_offset.x).speed(0.5).suffix(" mm"));
                    ui.add(egui::DragValue::new(&mut self.array_offset.y).speed(0.5).suffix(" mm"));
                    if !self.array_circular {
                        ui.add(egui::DragValue::new(&mut self.array_offset.z).speed(0.5).suffix(" mm"));
                    }
                });
                if ui.add_enabled(!self.selection.is_empty(), egui::Button::new("Create")).clicked() {
                    self.make_array();
                }
            });
            self.show_array = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Controls").open(&mut open).show(ctx, |ui| {
                ui.label("Mouse: left-drag an object to move it on the plate · left-drag empty space or right-drag to orbit · middle-drag or Shift+right-drag to pan · scroll/pinch to zoom · Shift+drag on empty space to box-select · Alt+drag to drag out a copy.");
                ui.label("Keys (Ctrl on Windows, Cmd on Mac): Z undo · Shift+Z / Y redo · C/V copy/paste · D duplicate · G group · Shift+G ungroup · A select all · S save.");
                ui.label("Plain keys: H make hole · S make solid · D drop to bed · F fit view · P paint · B fill · 1/2/3 move/rotate/scale gizmo · arrows nudge (Shift ×10, Ctrl/Cmd+↑↓ raise/lower) · Delete removes · Esc back to select.");
                ui.label("Gizmo snaps to the grid; hold Shift while dragging for free movement.");
            });
            self.show_help = open;
        }
        if let Some((k, p)) = self.export_confirm.clone() {
            egui::Window::new("Problems found").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let n = self.report.as_ref().map(|r| r.issues.iter().filter(|i| i.severity == Severity::Error).count()).unwrap_or(0);
                ui.label(format!("The check found {n} error(s) that will probably cause a failed or incorrect print. They're listed in the Print check panel."));
                ui.horizontal(|ui| {
                    if ui.button("Fix them first").clicked() {
                        self.export_confirm = None;
                    }
                    if ui.button("Export anyway").clicked() {
                        self.export_confirm = None;
                        self.do_export(k, p.clone());
                    }
                });
            });
        }
    }
}

trait KindDebug {
    fn kind_debug(&self) -> String;
}
impl KindDebug for Object {
    fn kind_debug(&self) -> String {
        match &self.kind {
            Kind::Shape(s) => format!("{s:?}"),
            Kind::Mesh(m) => format!("mesh{}", m.tris.len()),
            Kind::Group { keep_colors, children } => format!("group{}{}", keep_colors, children.len()),
        }
    }
}

fn box_lines(b: &mut Batch, bb: Aabb, col: [f32; 4]) {
    let (mn, mx) = (bb.min.as_vec3(), bb.max.as_vec3());
    let c = |x: bool, y: bool, z: bool| Vec3::new(if x { mx.x } else { mn.x }, if y { mx.y } else { mn.y }, if z { mx.z } else { mn.z });
    for &(a, bq) in &[
        ((false, false, false), (true, false, false)),
        ((true, false, false), (true, true, false)),
        ((true, true, false), (false, true, false)),
        ((false, true, false), (false, false, false)),
        ((false, false, true), (true, false, true)),
        ((true, false, true), (true, true, true)),
        ((true, true, true), (false, true, true)),
        ((false, true, true), (false, false, true)),
        ((false, false, false), (false, false, true)),
        ((true, false, false), (true, false, true)),
        ((true, true, false), (true, true, true)),
        ((false, true, false), (false, true, true)),
    ] {
        b.line(c(a.0, a.1, a.2), c(bq.0, bq.1, bq.2), col);
    }
}

fn shape_params(ui: &mut egui::Ui, s: &mut Shape) {
    let sides = |ui: &mut egui::Ui, v: &mut u32, lo: u32, hi: u32, lbl: &str| {
        ui.add(egui::Slider::new(v, lo..=hi).text(lbl));
    };
    match s {
        Shape::Box { radius, steps } => {
            ui.add(egui::Slider::new(radius, 0.0..=50.0).text("corner radius (mm)"));
            if *radius > 0.0 {
                sides(ui, steps, 1, 16, "smoothness");
            }
        }
        Shape::Cylinder { sides: n, bevel } => {
            sides(ui, n, 3, 256, "sides");
            ui.add(egui::Slider::new(bevel, 0.0..=20.0).text("bevel (mm)"));
        }
        Shape::Sphere { steps } | Shape::HalfSphere { steps } => sides(ui, steps, 4, 128, "steps"),
        Shape::Cone { sides: n, top } => {
            sides(ui, n, 3, 256, "sides");
            ui.add(egui::Slider::new(top, 0.0..=1.0).text("top size"));
        }
        Shape::Pyramid { sides: n } | Shape::Polygon { sides: n } => sides(ui, n, 3, 64, "sides"),
        Shape::RoundRoof { sides: n } | Shape::Paraboloid { sides: n } => sides(ui, n, 4, 256, "sides"),
        Shape::Torus { sides: n, tube } => {
            sides(ui, n, 3, 256, "sides");
            ui.add(egui::Slider::new(tube, 0.02..=0.25).text("tube thickness"));
        }
        Shape::Tube { sides: n, wall } => {
            sides(ui, n, 3, 256, "sides");
            ui.add(egui::Slider::new(wall, 0.4..=50.0).text("wall (mm)"));
        }
        Shape::Star { points, inner } => {
            sides(ui, points, 3, 64, "points");
            ui.add(egui::Slider::new(inner, 0.05..=0.95).text("inner radius"));
        }
        Shape::Text { text, font, spacing } => {
            ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(220.0));
            ui.horizontal(|ui| {
                ui.selectable_value(font, FontChoice::Sans, "Sans");
                ui.selectable_value(font, FontChoice::Mono, "Mono");
            });
            ui.add(egui::Slider::new(spacing, -0.2..=1.0).text("letter spacing"));
        }
        Shape::Roof | Shape::Wedge | Shape::Heart => {}
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_check();
        if self.check_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.shortcuts(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("PrintForge v{} · {} · {}", env!("CARGO_PKG_VERSION"), self.doc.printer.name, self.file.as_ref().map(|f| f.display().to_string()).unwrap_or("unsaved".into()))).weak());
                });
            });
        });
        egui::Panel::left("left").resizable(true).default_size(240.0).show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("right").resizable(true).default_size(300.0).show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
        self.windows(&ctx);
        self.commit_if_idle(&ctx);
        self.screenshot_step(&ctx);
    }
}

impl App {
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some((path, frame)) = self.screenshot.clone() else { return };
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
        });
        if let Some(img) = shot {
            let f = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.size[0] as u32, img.size[1] as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            let data: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
            w.write_image_data(&data).unwrap();
            drop(w);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
            return;
        }
        if frame == 2 {
            self.fit_view(false);
        }
        if frame == 6 {
            if std::env::var("PRINTFORGE_SHOT_SELECT").is_ok() {
                self.selection = self.doc.objects.first().map(|o| vec![o.id]).unwrap_or_default();
                self.geometry_dirty = true;
            }
            if let Ok(t) = std::env::var("PRINTFORGE_SHOT_TOOL") {
                self.tool = match t.as_str() { "paint" => Tool::Paint, "fill" => Tool::Fill, _ => Tool::Select };
            }
            if std::env::var("PRINTFORGE_SHOT_CHECK").is_ok() {
                let r = check::run(&self.build_check_input());
                self.selected_issue = r.issues.iter().position(|i| !i.tris.is_empty());
                if let Some(id) = self.selected_issue.and_then(|i| r.issues[i].object) {
                    self.selection = vec![id];
                }
                self.report = Some(r);
                self.geometry_dirty = true;
            }
        }
        if frame == 10 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        self.screenshot = Some((path, frame + 1));
        ctx.request_repaint();
    }
}
