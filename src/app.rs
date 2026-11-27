//! The RoboForge desktop application.

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
    SnapFaces,
    PushPull,
    Decal,
    SurfaceText,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GizmoKind {
    Off,
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum PendingAction {
    Close,
    New,
    Open,
    OpenPath(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportKind {
    Bambu,
    /// Export, then open in Bambu Studio ready to slice and print.
    Print,
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

#[derive(Clone)]
struct ResizeDrag {
    id: u64,
    axis: usize,
    sign: f64,
    start: Object,
    anchor: DVec3,
    origin: DVec3,
    dir: DVec3,
    s0: f64,
}

#[derive(Clone)]
struct PushPull {
    id: u64,
    region: Vec<u32>,
    n: DVec3,
    origin: DVec3,
    dist: f64,
    dragging: bool,
    s0: f64,
}

fn mods_shift(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.shift)
}

/// Parameter along the line `o + s*dir` closest to the ray `ro + t*rd`.
fn line_param(o: DVec3, dir: DVec3, ro: DVec3, rd: DVec3) -> f64 {
    let w0 = o - ro;
    let b = dir.dot(rd);
    let d = dir.dot(w0);
    let e = rd.dot(w0);
    let den = 1.0 - b * b;
    if den.abs() < 1e-9 { 0.0 } else { (b * e - d) / den }
}

struct AiPanel {
    open: bool,
    settings: crate::ai::AiSettings,
    show_settings: bool,
    /// what the user sees: (from user?, text)
    shown: Vec<(bool, String)>,
    /// what the model sees
    history: Vec<crate::ai::Msg>,
    input: String,
    rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    test_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    test_result: String,
    started: Option<std::time::Instant>,
}

impl Default for AiPanel {
    fn default() -> Self {
        let settings = crate::ai::AiSettings::load();
        let show_settings = crate::settings::get("ai_url").is_empty();
        Self { open: false, settings, show_settings, shown: vec![], history: vec![], input: String::new(), rx: None, test_rx: None, test_result: String::new(), started: None }
    }
}

struct PreviewState {
    meshes: Vec<Mesh>,
    est: crate::preview::Estimate,
    merges: Option<Vec<crate::preview::Merge>>,
    /// layers shown: 1..=est.layers
    layer: usize,
    edits: u64,
}

struct ImgDialog {
    path: PathBuf,
    pic: Arc<crate::imaging::Picture>,
    tex: Option<egui::TextureHandle>,
    opts: crate::imaging::ImageOptions,
    decal: bool,
    decal_width: f64,
    skip_bg: bool,
}

struct SvgDialog {
    path: PathBuf,
    layers: Vec<crate::imaging::SvgLayer>,
    width: f64,
    height: f64,
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
    Move { plane_z: f64, start_hit: DVec3, start: Vec<(u64, DVec3)>, sb: Aabb, others: Vec<Aabb>, riding: bool, samples: Vec<(f64, f64, f64)> },
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
    gizmo_ctx: (Aabb, Vec<Aabb>),
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
    snap_from: Option<(u64, DVec3, DVec3)>,
    last_view_rect: Rect,
    guides: Vec<(DVec3, DVec3)>,
    // check
    report: Option<Report>,
    check_rx: Option<mpsc::Receiver<Report>>,
    selected_issue: Option<usize>,
    overhang_deg: f64,
    pending_export: Option<(ExportKind, PathBuf)>,
    export_confirm: Option<(ExportKind, PathBuf)>,
    // misc
    file: Option<PathBuf>,
    opened_at_start: bool,
    img_dialog: Option<ImgDialog>,
    sketch: Option<crate::sketch::SketchEditor>,
    my_fils: Option<Vec<Filament>>,
    history: Option<Vec<crate::history::Version>>,
    ai: AiPanel,
    decal: Option<crate::decal::Decal>,
    decal_tex: Option<egui::TextureHandle>,
    svg_dialog: Option<SvgDialog>,
    last_autosave: std::time::Instant,
    recover_offer: bool,
    show_overhangs: bool,
    xray: bool,
    stext: crate::surface_text::SurfaceText,
    edge_radius: f64,
    edge_chamfer: bool,
    edge_skip_bottom: bool,
    edge_apply: Option<u64>,
    split_axis: usize,
    split_at: Option<(u64, usize, f64)>,
    split_pins: bool,
    split_pin_d: f64,
    split_preview: Option<(u64, usize, f64)>,
    split_apply: Option<u64>,
    thread_outside: bool,
    thread_d: Option<(u64, f64)>,
    thread_pitch_auto: bool,
    thread_pitch: f64,
    thread_len: f64,
    thread_top: bool,
    thread_apply: Option<u64>,
    preview: Option<PreviewState>,
    xray_edges: HashMap<u64, Arc<Vec<(Vec3, Vec3)>>>,
    bands: Vec<(f64, u8)>,
    printable_report: Option<Vec<String>>,
    resize: Option<ResizeDrag>,
    pp: Option<PushPull>,
    thumbs: Vec<egui::TextureHandle>,
    edits: u64,
    saved_edits: u64,
    confirm_discard: Option<PendingAction>,
    allow_close: bool,
    last_title: String,
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
        crate::theme::apply(&cc.egui_ctx, crate::theme::load_saved());
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
            gizmo_ctx: (Aabb::empty(), vec![]),
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
            snap_from: None,
            last_view_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)),
            guides: vec![],
            report: None,
            check_rx: None,
            selected_issue: None,
            overhang_deg: 45.0,
            pending_export: None,
            export_confirm: None,
            file,
            opened_at_start: false,
            img_dialog: None,
            sketch: None,
            my_fils: None,
            history: None,
            ai: AiPanel::default(),
            decal: None,
            decal_tex: None,
            svg_dialog: None,
            last_autosave: std::time::Instant::now(),
            recover_offer: crate::settings::autosave_path().map(|p| p.exists()).unwrap_or(false),
            show_overhangs: false,
            xray: false,
            stext: crate::surface_text::SurfaceText { text: "HELLO".into(), font: crate::shapes::FontChoice::Sans, height: 8.0, engrave: false, depth: 1.0, rotation: 0.0, color: 3, from_view: false },
            edge_radius: 2.0,
            edge_chamfer: false,
            edge_skip_bottom: true,
            edge_apply: None,
            split_axis: 0,
            split_at: None,
            split_pins: true,
            split_pin_d: 5.0,
            split_preview: None,
            split_apply: None,
            thread_outside: true,
            thread_d: None,
            thread_pitch_auto: true,
            thread_pitch: 1.5,
            thread_len: 10.0,
            thread_top: true,
            thread_apply: None,
            preview: None,
            xray_edges: HashMap::new(),
            bands: vec![(0.0, 0), (5.0, 2)],
            printable_report: None,
            resize: None,
            pp: None,
            thumbs: vec![],
            edits: 0,
            saved_edits: 0,
            confirm_discard: None,
            allow_close: false,
            last_title: String::new(),
            status: "Welcome to RoboForge — pick a shape on the left to start.".into(),
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

    // ---------------------------------------------------------- plates

    fn switch_plate(&mut self, to: usize) {
        if to == self.doc.plate {
            return;
        }
        self.doc.switch_plate(to);
        self.selection.clear();
        self.touch();
        self.status = format!("Now editing plate {}.", to + 1);
    }

    fn move_selection_to_plate(&mut self, to: usize) {
        let ids: Vec<u64> = self.selection.iter().copied().filter(|id| self.doc.find(*id).is_some_and(|o| !o.locked)).collect();
        if ids.is_empty() {
            return;
        }
        self.doc.move_to_plate(&ids, to);
        self.selection.clear();
        self.touch();
        self.status = format!("Moved {} object(s) to plate {}.", ids.len(), to + 1);
    }

    /// Plate tabs along the top-left of the viewport.
    fn plate_tabs(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let n = self.doc.plate_count();
        let area = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), Vec2::new((rect.width() - 220.0).max(120.0), 26.0));
        let mut go = None;
        let mut add = false;
        let mut remove = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                for p in 0..n {
                    let count = if p == self.doc.plate { self.doc.objects.len() } else { self.doc.plates.get(p).map_or(0, |v| v.len()) };
                    let label = if n == 1 { "Plate 1".to_string() } else { format!("Plate {} ({count})", p + 1) };
                    let r = ui.selectable_label(p == self.doc.plate, label).on_hover_text("Each plate prints separately. Bambu Studio gets every plate in one file.");
                    if r.clicked() {
                        go = Some(p);
                    }
                    r.context_menu(|ui| {
                        if count == 0 && n > 1 {
                            if ui.button("Delete this empty plate").clicked() {
                                remove = Some(p);
                                ui.close();
                            }
                        } else {
                            ui.label("Only empty plates can be deleted.");
                        }
                    });
                }
                if ui.small_button("+ Plate").on_hover_text("Add another build plate").clicked() {
                    add = true;
                }
            });
        });
        if let Some(p) = go {
            self.switch_plate(p);
        }
        if add {
            let p = self.doc.add_plate();
            self.switch_plate(p);
        }
        if let Some(p) = remove {
            if self.doc.remove_plate(p) {
                self.selection.clear();
                self.touch();
                self.status = "Plate deleted.".into();
            }
        }
    }

    // ---------------------------------------------------------- doc helpers

    fn touch(&mut self) {
        self.edits += 1;
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
        self.edits += 1;
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
        let mut shape = (info.shape)();
        if let Some(g) = crate::settings::fit_gap() {
            shape.apply_fit(g);
        }
        let mut size = DVec3::from(info.default_size);
        if let Some(a) = shape.text_aspect() {
            size.x = size.y * a;
        }
        let id = self.doc.alloc_id();
        let mut o = Object::new(id, info.name, Kind::Shape(shape), size);
        o.color = (self.doc.objects.len() % self.doc.filaments.len().max(1)) as u8;
        o.pos = self.free_spot(size);
        let is_container = matches!(o.kind, Kind::Shape(Shape::Container { .. }));
        let is_jar = matches!(o.kind, Kind::Shape(Shape::Jar { .. }));
        let (cw, cpos) = (o.size, o.pos);
        self.doc.objects.push(o);
        self.selection = vec![id];
        if is_container {
            let lid_id = self.doc.alloc_id();
            let mut l = Object::new(lid_id, "Lid", Kind::Shape({
                let mut l = Shape::Lid { wall: 2.0, lip: 4.0, clearance: 0.25, radius: 4.0 };
                if let Some(g) = crate::settings::fit_gap() {
                    l.apply_fit(g);
                }
                l
            }), DVec3::new(cw.x, cw.y, 6.0));
            l.pos = cpos + DVec3::new(0.0, cw.y + 8.0, 0.0);
            l.color = 0;
            self.doc.objects.push(l);
            self.selection.push(lid_id);
        }
        if is_jar {
            let cap_id = self.doc.alloc_id();
            let cs = crate::parts::jar_cap_size(cw.x, cw.z);
            let mut c = Object::new(cap_id, "Jar cap", Kind::Shape({
                let mut c = Shape::JarCap { clearance: 0.4 };
                if let Some(g) = crate::settings::fit_gap() {
                    c.apply_fit(g);
                }
                c
            }), cs);
            c.pos = cpos + DVec3::new(cw.x / 2.0 + cs.x / 2.0 + 8.0, 0.0, 0.0);
            c.color = 0;
            self.doc.objects.push(c);
            self.selection.push(cap_id);
        }
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
        self.edits = 0;
        self.saved_edits = 0;
    }

    fn clear_autosave(&mut self) {
        if let Some(p) = crate::settings::autosave_path() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Re-orient every free-standing part for printing: best face down, on the plate,
    /// spread out on the bed.
    /// Lids and jar caps sitting on their box/jar would print fused to it: move each one
    /// (with anything sitting on it, like raised text) onto the plate beside it.
    fn separate_lids(&mut self) -> Vec<String> {
        fn has_lid(o: &Object) -> (bool, bool) {
            match &o.kind {
                Kind::Shape(Shape::Lid { .. } | Shape::JarCap { .. }) => (true, false),
                Kind::Shape(Shape::Container { .. } | Shape::Jar { .. }) => (false, true),
                Kind::Group { children, .. } => children.iter().map(has_lid).fold((false, false), |a, b| (a.0 || b.0, a.1 || b.1)),
                _ => (false, false),
            }
        }
        let mut out = vec![];
        let lids: Vec<u64> = self.doc.objects.iter().filter(|o| o.visible && !o.hole && !o.locked).filter(|o| has_lid(o) == (true, false)).map(|o| o.id).collect();
        for id in lids {
            let b = self.world_bounds(id);
            if b.min.z < 0.5 {
                continue;
            }
            // things resting on the lid travel with it
            let riders: Vec<u64> = self
                .doc
                .objects
                .iter()
                .filter(|o| o.id != id && o.visible)
                .map(|o| o.id)
                .collect::<Vec<_>>()
                .into_iter()
                .filter(|&r| {
                    let rb = self.world_bounds(r);
                    rb.min.z >= b.max.z - 0.5 - (b.max.z - b.min.z) && rb.min.z > b.min.z + 0.1 && rb.min.x < b.max.x && rb.max.x > b.min.x && rb.min.y < b.max.y && rb.max.y > b.min.y
                })
                .collect();
            let mut all = b;
            for &r in &riders {
                let rb = self.world_bounds(r);
                all.min = all.min.min(rb.min);
                all.max = all.max.max(rb.max);
            }
            let size = all.max - all.min;
            // look for room beside it, temporarily ignoring the lid group itself
            let moving: Vec<u64> = std::iter::once(id).chain(riders.iter().copied()).collect();
            let used: Vec<Aabb> = self.doc.objects.clone().iter().filter(|o| o.visible && !moving.contains(&o.id)).map(|o| self.cache.world_mesh(o).bounds()).collect();
            let mut x = all.max.x + 10.0 + size.x / 2.0;
            let cy = (all.min.y + all.max.y) / 2.0;
            for _ in 0..60 {
                let cand = Aabb { min: DVec3::new(x - size.x / 2.0, cy - size.y / 2.0, 0.0), max: DVec3::new(x + size.x / 2.0, cy + size.y / 2.0, size.z) };
                if !used.iter().any(|u| u.overlaps(&cand, -2.0)) {
                    break;
                }
                x += 10.0;
            }
            let delta = DVec3::new(x - (all.min.x + all.max.x) / 2.0, 0.0, -b.min.z);
            for m in &moving {
                if let Some(o) = self.doc.find_mut(*m) {
                    o.pos += delta;
                }
            }
            let name = self.doc.find(id).map(|o| o.name.clone()).unwrap_or_default();
            out.push(format!("Moved '{name}' off its box onto the plate beside it, so they print as separate pieces."));
        }
        out
    }

    fn make_printable(&mut self) {
        let mut lines = self.separate_lids();
        let ids: Vec<u64> = self.doc.objects.iter().filter(|o| o.visible && !o.hole && !o.locked).map(|o| o.id).collect();
        let bounds: Vec<Aabb> = ids.iter().map(|&id| self.world_bounds(id)).collect();
        let resting = crate::check::resting_on(&bounds);
        let deg = self.overhang_deg;
        for (k, &id) in ids.iter().enumerate() {
            if resting[k].is_some() {
                continue; // stacked parts stay with what they sit on
            }
            let Some(o) = self.doc.find(id).cloned() else { continue };
            if matches!(o.kind, Kind::Shape(Shape::Lid { .. } | Shape::JarCap { .. })) {
                continue; // lids and caps are already modelled the way they print
            }
            let wm = self.cache.world_mesh(&o);
            let c = wm.bounds().center();
            // candidate "down" directions: the 6 axes + the 3 biggest flat faces
            let mut dirs: Vec<DVec3> = vec![DVec3::NEG_Z, DVec3::Z, DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y];
            let mut by_n: HashMap<(i32, i32, i32), (DVec3, f64)> = HashMap::new();
            for t in 0..wm.tris.len() {
                let (n, a) = wm.tri_normal_area(t);
                let key = ((n.x * 20.0).round() as i32, (n.y * 20.0).round() as i32, (n.z * 20.0).round() as i32);
                let e = by_n.entry(key).or_insert((DVec3::ZERO, 0.0));
                e.0 += n * a;
                e.1 += a;
            }
            let mut faces: Vec<(DVec3, f64)> = by_n.into_values().collect();
            faces.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            dirs.extend(faces.iter().take(3).map(|f| f.0.normalize_or_zero()).filter(|d| d.length() > 0.5));
            let score = |q: DQuat| -> (f64, f64, f64) {
                let pts: Vec<DVec3> = wm.verts.iter().map(|v| q * (*v - c)).collect();
                let minz = pts.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
                let maxz = pts.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
                let (mut contact, mut over) = (0.0, 0.0);
                let lim = (90.0 - deg).to_radians().cos();
                for t in &wm.tris {
                    let (a, b, cc) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
                    let nn = (b - a).cross(cc - a);
                    let area = nn.length() * 0.5;
                    if area < 1e-12 {
                        continue;
                    }
                    let n = nn / (2.0 * area);
                    let low = a.z.max(b.z).max(cc.z) < minz + 0.05;
                    if n.z < -0.995 && low {
                        contact += area;
                    } else if n.z < -lim && a.z.min(b.z).min(cc.z) > minz + 0.1 {
                        over += area;
                    }
                }
                (over - 3.0 * contact + 0.02 * (maxz - minz), contact, over)
            };
            let base = score(DQuat::IDENTITY);
            let mut best = (DQuat::IDENTITY, base);
            for d in dirs {
                let q = DQuat::from_rotation_arc(d.normalize(), DVec3::NEG_Z);
                let sc = score(q);
                if sc.0 < best.1.0 - 1.0 {
                    best = (q, sc);
                }
            }
            let rotated = best.0 != DQuat::IDENTITY;
            if let Some(ob) = self.doc.find_mut(id) {
                if rotated {
                    ob.set_rotation(best.0 * ob.rotation());
                }
            }
            if rotated {
                lines.push(format!("• '{}': turned onto a better face (overhangs {:.0} to {:.0} mm², flat contact {:.0} to {:.0} mm²).", o.name, base.2, best.1.2, base.1, best.1.1));
            }
            let h = self.world_bounds(id).size().z;
            if best.1.1 > 0.0 && h / best.1.1.sqrt() > 8.0 && h > 30.0 {
                lines.push(format!("• '{}' is tall on a small base — enable a brim in Bambu Studio (Others > Brim).", o.name));
            }
        }
        self.touch();
        self.refresh_world();
        // everything onto the plate
        let all: Vec<u64> = ids.clone();
        let bs: Vec<Aabb> = all.iter().map(|&id| self.world_bounds(id)).collect();
        let rest = crate::check::resting_on(&bs);
        for (k, &id) in all.iter().enumerate() {
            if rest[k].is_none() && bs[k].min.z.abs() > 0.005 {
                if let Some(o) = self.doc.find_mut(id) {
                    o.pos.z -= bs[k].min.z;
                }
            }
        }
        self.touch();
        self.refresh_world();
        // spread out if parts overlap or leave the plate
        let bs: Vec<Aabb> = all.iter().map(|&id| self.world_bounds(id)).collect();
        let (hx, hy) = (self.doc.printer.bed_multi[0] / 2.0, self.doc.printer.bed_multi[1] / 2.0);
        let off_bed = bs.iter().any(|b| b.min.x < -hx || b.max.x > hx || b.min.y < -hy || b.max.y > hy);
        let mut overlap = false;
        for i in 0..bs.len() {
            for j in i + 1..bs.len() {
                let (a, b) = (&bs[i], &bs[j]);
                if a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y && rest[i].is_none() && rest[j].is_none() {
                    overlap = true;
                }
            }
        }
        if off_bed || overlap {
            self.arrange();
            lines.push("• Parts were overlapping or off the plate, so they were spread out on the plate.".into());
        }
        lines.push("• Everything is sitting on the plate.".into());
        self.start_check();
        lines.push("• A print check is running — see the right panel for anything left to fix.".into());
        self.status = "Made printable.".into();
        self.printable_report = Some(lines);
    }

    /// Bake horizontal colour stripes into the selected part.
    fn apply_bands(&mut self) {
        let Some(&id) = self.selection.first() else { return };
        let Some(o) = self.doc.find(id).cloned() else { return };
        let mut wm = self.cache.world_mesh(&o);
        let bottom = wm.bounds().min.z;
        let mut bands = self.bands.clone();
        bands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for (h, _) in &bands {
            if *h > 0.0 {
                wm = wm.split_at_z(bottom + h);
            }
        }
        for t in 0..wm.tris.len() {
            let z = wm.tri_centroid(t).z - bottom;
            if let Some((_, col)) = bands.iter().rev().find(|(h, _)| z >= *h - 1e-6) {
                wm.colors[t] = *col;
            }
        }
        let local = wm.transformed(&o.transform().inverse());
        if let Some(ob) = self.doc.find_mut(id) {
            ob.kind = Kind::Mesh(Arc::new(local));
            ob.paint = None;
        }
        self.touch();
        self.status = format!("Applied {} colour bands to '{}'.", bands.len(), o.name);
    }

    fn unsaved(&self) -> bool {
        self.edits != self.saved_edits && !self.doc.objects.is_empty()
            || self.edits != self.saved_edits && self.file.is_some()
    }

    /// Run an action that would lose work, asking first if there are unsaved changes.
    fn guarded(&mut self, a: PendingAction, ctx: &egui::Context) {
        if self.unsaved() {
            self.confirm_discard = Some(a);
        } else {
            self.run_pending(a, ctx);
        }
    }

    fn run_pending(&mut self, a: PendingAction, ctx: &egui::Context) {
        match a {
            PendingAction::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingAction::New => self.new_doc(),
            PendingAction::Open => self.open(),
            PendingAction::OpenPath(p) => self.open_path(p),
        }
    }

    fn open(&mut self) {
        let Some(p) = rfd::FileDialog::new().add_filter("RoboForge project", &["pforge"]).pick_file() else { return };
        self.open_path(p);
    }

    fn open_path(&mut self, p: PathBuf) {
        match io::load_project(&p) {
            Ok(d) => {
                self.new_doc();
                self.doc = d;
                self.committed = self.doc.clone();
                self.status = format!("Opened {}", p.display());
                crate::settings::add_recent(&p);
                self.file = Some(p);
                self.edits = 0;
                self.saved_edits = 0;
            }
            Err(e) => self.status = format!("Couldn't open: {e}"),
        }
    }

    /// Returns true if the project was saved.
    fn save(&mut self, as_new: bool) -> bool {
        let path = if as_new || self.file.is_none() {
            rfd::FileDialog::new().add_filter("RoboForge project", &["pforge"]).set_file_name("design.pforge").save_file()
        } else {
            self.file.clone()
        };
        let Some(p) = path else { return false };
        let p = if p.extension().is_none() { p.with_extension("pforge") } else { p };
        match io::save_project(&p, &self.doc) {
            Ok(()) => {
                self.status = format!("Saved {}", p.display());
                crate::history::record(&p);
                crate::settings::add_recent(&p);
                self.file = Some(p);
                self.clear_autosave();
                self.saved_edits = self.edits;
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }

    fn import(&mut self) {
        let Some(paths) = rfd::FileDialog::new().add_filter("3D models, pictures, SVG", &["stl", "obj", "3mf", "png", "jpg", "jpeg", "svg"]).pick_files() else { return };
        for p in paths {
            self.import_path(&p);
        }
    }

    fn import_path(&mut self, p: &std::path::Path) {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["png", "jpg", "jpeg", "bmp", "webp", "gif"].contains(&ext.as_str()) {
            self.open_image(p.to_path_buf());
            return;
        }
        if ext == "svg" {
            self.open_svg(p.to_path_buf());
            return;
        }
        match io::import_file(p) {
            Ok(r) => {
                self.add_imported(r);
                crate::settings::add_recent(p);
            }
            Err(e) => self.status = format!("Import failed for {}: {e}", p.display()),
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

    fn open_image(&mut self, path: PathBuf) {
        match crate::imaging::load_picture(&path) {
            Ok(pic) => {
                let mut opts = crate::imaging::ImageOptions::default();
                opts.use_filaments = vec![true; self.doc.filaments.len()];
                // a part is selected: most likely they want the picture ON it
                let decal = !self.selection.is_empty();
                let has_alpha = pic.rgba.chunks(4).any(|p| p[3] < 250);
                self.img_dialog = Some(ImgDialog { path, pic: Arc::new(pic), tex: None, opts, decal, decal_width: 40.0, skip_bg: !has_alpha });
            }
            Err(e) => self.status = format!("Couldn't read that picture: {e}"),
        }
    }

    fn open_svg(&mut self, path: PathBuf) {
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| crate::imaging::load_svg(&d)) {
            Ok(layers) => self.svg_dialog = Some(SvgDialog { path, layers, width: 60.0, height: 3.0 }),
            Err(e) => self.status = format!("Couldn't read that SVG: {e}"),
        }
    }

    fn slot_for_color(&mut self, c: [u8; 3]) -> u8 {
        let near = self
            .doc
            .filaments
            .iter()
            .enumerate()
            .map(|(i, f)| (i, (0..3).map(|k| (f.color[k] as i32 - c[k] as i32).pow(2)).sum::<i32>()))
            .min_by_key(|x| x.1);
        match near {
            Some((i, d)) if d < 40 * 40 * 3 || self.doc.filaments.len() >= 24 => i as u8,
            _ => {
                self.doc.filaments.push(Filament { name: format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]), color: c, material: "PLA".into() });
                (self.doc.filaments.len() - 1) as u8
            }
        }
    }

    /// Somewhere on the plate next to existing objects, Tinkercad-style.
    fn free_spot(&mut self, size: DVec3) -> DVec3 {
        let mut x = 0.0;
        let used: Vec<Aabb> = self.doc.objects.clone().iter().map(|o| self.cache.world_mesh(o).bounds()).collect();
        for _ in 0..40 {
            let b = Aabb { min: DVec3::new(x - size.x / 2.0, -size.y / 2.0, 0.0), max: DVec3::new(x + size.x / 2.0, size.y / 2.0, size.z) };
            if !used.iter().any(|u| u.overlaps(&b, -1.0)) {
                break;
            }
            x += size.x.max(10.0) + 5.0;
        }
        DVec3::new(x, 0.0, 0.0)
    }

    fn place_new_mesh(&mut self, name: &str, mesh: Mesh) -> u64 {
        let (m, size, _off) = normalize_mesh(mesh);
        let id = self.doc.alloc_id();
        let o = Object::new(id, name, Kind::Mesh(Arc::new(m)), size);
        self.doc.objects.push(o);
        id
    }

    fn picture_windows(&mut self, ctx: &egui::Context) {
        self.sketch_window(ctx);
        self.ai_window(ctx);
        self.my_filaments_window(ctx);
        self.history_window(ctx);
        use crate::imaging::ImageMode;
        let mut close_img = false;
        let mut create_img = None;
        let mut place_decal = None;
        if let Some(d) = &mut self.img_dialog {
            if d.tex.is_none() {
                let img = egui::ColorImage::from_rgba_unmultiplied([d.pic.w as usize, d.pic.h as usize], &d.pic.rgba);
                d.tex = Some(ctx.load_texture("img-preview", img, egui::TextureOptions::LINEAR));
            }
            let mut open = true;
            egui::Window::new("Picture to 3D").open(&mut open).resizable(false).default_width(380.0).show(ctx, |ui| {
                ui.label(RichText::new(d.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).weak());
                if let Some(t) = &d.tex {
                    let w = 340.0f32;
                    let h = w * d.pic.h as f32 / d.pic.w as f32;
                    ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::new(w, h.min(260.0))));
                }
                ui.horizontal_wrapped(|ui| {
                    for (m, n) in [(ImageMode::Lithophane, "Lithophane"), (ImageMode::Relief, "Raised relief"), (ImageMode::ColourPlate, "Colour plate")] {
                        if ui.selectable_label(!d.decal && d.opts.mode == m, n).clicked() {
                            d.opts.mode = m;
                            d.decal = false;
                        }
                    }
                    if ui.selectable_label(d.decal, "On a part's surface").clicked() {
                        d.decal = true;
                    }
                });
                if d.decal {
                    ui.label(RichText::new("Paints the picture onto a part you click, using your filament colours. It prints in colour with no extra setup.").small().weak());
                    ui.add(egui::Slider::new(&mut d.decal_width, 5.0..=250.0).text("width (mm)"));
                    ui.checkbox(&mut d.skip_bg, "Leave the background unpainted").on_hover_text("Skips the colour found in the picture's corners (e.g. the white around a logo). Transparent areas are always skipped.");
                    ui.label("Use these filaments:");
                    ui.horizontal_wrapped(|ui| {
                        for (i, f) in self.doc.filaments.iter().enumerate() {
                            if d.opts.use_filaments.len() <= i {
                                d.opts.use_filaments.push(true);
                            }
                            let on = &mut d.opts.use_filaments[i];
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                            ui.painter().rect_filled(r, 3.0, c32(f.color));
                            ui.painter().rect_stroke(r, 3.0, Stroke::new(if *on { 2.5 } else { 1.0 }, if *on { crate::theme::accent() } else { Color32::from_gray(90) }), egui::StrokeKind::Outside);
                            if !*on {
                                ui.painter().line_segment([r.left_top(), r.right_bottom()], Stroke::new(2.0, Color32::from_gray(160)));
                            }
                            if resp.on_hover_text(&f.name).clicked() {
                                *on = !*on;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    if ui.button(RichText::new("Place on a part…").strong()).clicked() {
                        place_decal = Some((d.pic.clone(), d.tex.clone(), d.decal_width, d.skip_bg, d.opts.use_filaments.clone()));
                    }
                    return;
                }
                let hint = match d.opts.mode {
                    ImageMode::Lithophane => "Thin where the photo is bright, thick where dark. Print in white and hold it up to a light. Tip: print it standing up for best detail.",
                    ImageMode::Relief => "A plate with the bright parts raised (tick Invert to raise the dark parts).",
                    ImageMode::ColourPlate => "A flat plate whose top is coloured with your filaments — each spot uses the closest filament colour.",
                };
                ui.label(RichText::new(hint).small().weak());
                ui.add(egui::Slider::new(&mut d.opts.width_mm, 20.0..=300.0).text("width (mm)"));
                match d.opts.mode {
                    ImageMode::Lithophane => {
                        ui.add(egui::Slider::new(&mut d.opts.min_mm, 0.4..=3.0).text("thinnest (mm)"));
                        ui.add(egui::Slider::new(&mut d.opts.max_mm, 1.0..=8.0).text("thickest (mm)"));
                    }
                    ImageMode::Relief => {
                        ui.add(egui::Slider::new(&mut d.opts.min_mm, 0.6..=5.0).text("base (mm)"));
                        ui.add(egui::Slider::new(&mut d.opts.max_mm, 0.4..=15.0).text("relief height (mm)"));
                    }
                    ImageMode::ColourPlate => {
                        ui.add(egui::Slider::new(&mut d.opts.min_mm, 0.8..=6.0).text("plate thickness (mm)"));
                        ui.label("Use these filaments:");
                        ui.horizontal_wrapped(|ui| {
                            for (i, f) in self.doc.filaments.iter().enumerate() {
                                if d.opts.use_filaments.len() <= i {
                                    d.opts.use_filaments.push(true);
                                }
                                let on = &mut d.opts.use_filaments[i];
                                let (r, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                                ui.painter().rect_filled(r, 3.0, c32(f.color));
                                ui.painter().rect_stroke(r, 3.0, Stroke::new(if *on { 2.5 } else { 1.0 }, if *on { crate::theme::accent() } else { Color32::from_gray(90) }), egui::StrokeKind::Outside);
                                if !*on {
                                    ui.painter().line_segment([r.left_top(), r.right_bottom()], Stroke::new(2.0, Color32::from_gray(160)));
                                }
                                if resp.on_hover_text(&f.name).clicked() {
                                    *on = !*on;
                                }
                            }
                        });
                    }
                }
                ui.add(egui::Slider::new(&mut d.opts.border_mm, 0.0..=10.0).text("frame (mm)"));
                ui.add(egui::Slider::new(&mut d.opts.detail, 1.0..=8.0).text("detail (points/mm)"));
                if d.opts.mode != ImageMode::ColourPlate {
                    ui.checkbox(&mut d.opts.invert, "Invert");
                }
                ui.add_space(4.0);
                if ui.button(RichText::new("Create").strong()).clicked() {
                    create_img = Some((d.pic.clone(), d.opts.clone()));
                }
            });
            if !open {
                close_img = true;
            }
        }
        if let Some((pic, tex, width, skip_background, allowed)) = place_decal {
            self.decal = Some(crate::decal::Decal { pic, width, rotation: 0.0, allowed, skip_background, from_view: false });
            self.decal_tex = tex;
            self.tool = Tool::Decal;
            self.geometry_dirty = true;
            self.status = "Click on a part to put the picture there. Esc to cancel.".into();
            close_img = true;
        }
        if let Some((pic, opts)) = create_img {
            let mesh = crate::imaging::picture_to_mesh(&pic, &opts, &self.doc.filaments);
            let name = match opts.mode {
                crate::imaging::ImageMode::Lithophane => "Lithophane",
                crate::imaging::ImageMode::Relief => "Relief",
                crate::imaging::ImageMode::ColourPlate => "Photo plate",
            };
            let id = self.place_new_mesh(name, mesh);
            self.selection = vec![id];
            self.touch();
            self.fit_view(true);
            self.status = format!("Created {name}.");
            close_img = true;
        }
        if close_img {
            self.img_dialog = None;
        }
        // SVG
        let mut close_svg = false;
        let mut create_svg = false;
        if let Some(d) = &mut self.svg_dialog {
            let mut open = true;
            egui::Window::new("Logo from SVG").open(&mut open).resizable(false).show(ctx, |ui| {
                ui.label(RichText::new(d.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).weak());
                ui.label(format!("{} colour layer(s):", d.layers.len()));
                ui.horizontal_wrapped(|ui| {
                    for l in &d.layers {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, c32(l.color));
                        ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Inside);
                    }
                });
                ui.add(egui::Slider::new(&mut d.width, 5.0..=300.0).text("width (mm)"));
                ui.add(egui::Slider::new(&mut d.height, 0.4..=30.0).text("thickness (mm)"));
                ui.label(RichText::new("Each colour becomes its own part in the matching filament (new filaments are added for colours you don't have), grouped together.").small().weak());
                if ui.button(RichText::new("Create").strong()).clicked() {
                    create_svg = true;
                }
            });
            if !open {
                close_svg = true;
            }
        }
        if create_svg {
            let d = self.svg_dialog.take().unwrap();
            let meshes = crate::imaging::svg_meshes(&d.layers, d.width, d.height);
            let mut ids = vec![];
            for (col, mut m) in meshes {
                let slot = self.slot_for_color(col);
                m.set_color(NO_COLOR);
                let (nm, size, off) = normalize_mesh(m);
                let id = self.doc.alloc_id();
                let mut o = Object::new(id, &format!("Logo #{:02X}{:02X}{:02X}", col[0], col[1], col[2]), Kind::Mesh(Arc::new(nm)), size);
                o.pos = off;
                o.color = slot;
                self.doc.objects.push(o);
                ids.push(id);
            }
            if ids.len() > 1 {
                if let Some(g) = self.doc.group(&ids, &mut self.cache) {
                    if let Some(o) = self.doc.find_mut(g) {
                        o.name = d.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Logo".into());
                    }
                    ids = vec![g];
                }
            }
            self.selection = ids;
            self.touch();
            self.fit_view(true);
            self.status = "Logo created.".into();
            close_svg = true;
        }
        if close_svg {
            self.svg_dialog = None;
        }
    }

    /// Export to a file of our own and hand it to Bambu Studio — no save dialog.
    fn request_print(&mut self) {
        if self.doc.objects.iter().all(|o| o.hole || !o.visible) {
            self.status = "Nothing to print yet.".into();
            return;
        }
        let name = self.file.as_ref().and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "RoboForge design".into());
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
        let p = crate::bambu_studio::print_folder().join(format!("{safe}.3mf"));
        self.pending_export = Some((ExportKind::Print, p));
        self.start_check();
        self.status = "Checking, then opening in Bambu Studio…".into();
    }

    fn open_in_bambu(&mut self, file: &std::path::Path) -> String {
        let app = crate::bambu_studio::find().or_else(|| {
            let mut d = rfd::FileDialog::new().set_title("Where is Bambu Studio? (only asked once)");
            if cfg!(windows) {
                d = d.add_filter("Bambu Studio", &["exe"]).set_directory("C:\\Program Files");
            } else if cfg!(target_os = "macos") {
                d = d.set_directory("/Applications");
            }
            let p = d.pick_file()?;
            crate::bambu_studio::remember(&p);
            Some(p)
        });
        let Some(app) = app else {
            return format!("Saved {} — couldn't find Bambu Studio, so open that file in it yourself.", file.display());
        };
        match crate::bambu_studio::open(&app, file) {
            Ok(()) => "Opening in Bambu Studio… press Slice plate, then Print. Colours and filament types are already set.".into(),
            Err(e) => format!("Saved {} but couldn't start Bambu Studio ({e}). Open the file in it yourself.", file.display()),
        }
    }

    fn request_export(&mut self, kind: ExportKind) {
        let (filter, ext, def) = match kind {
            ExportKind::Bambu | ExportKind::Print => ("Bambu Studio project", "3mf", "design_bambu_H2C.3mf"),
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
        let n = self.doc.plate_count();
        let r = match kind {
            ExportKind::Bambu | ExportKind::Print if n > 1 => {
                let plates: Vec<_> = (0..n).map(|k| io::collect_export(&self.doc.plate_doc(k), &mut self.cache)).collect();
                io::write_bambu_3mf_plates(&p, &self.doc, &plates)
            }
            ExportKind::Bambu | ExportKind::Print => io::write_bambu_3mf(&p, &self.doc, &objs),
            ExportKind::Generic3mf => io::write_generic_3mf(&p, &self.doc, &objs),
            ExportKind::Stl => io::write_stl(&p, &objs),
            ExportKind::Obj => io::write_obj(&p, &self.doc, &objs),
        };
        self.status = match r {
            Ok(()) if kind == ExportKind::Print => self.open_in_bambu(&p),
            Ok(()) => match kind {
                ExportKind::Bambu if n > 1 => format!("Exported {} with {n} plates — open it in Bambu Studio and press Slice all.", p.display()),
                ExportKind::Bambu => format!("Exported {} — open it in Bambu Studio and press Slice; colours are already assigned.", p.display()),
                _ if n > 1 => format!("Exported {} (plate {} only — this format holds one plate).", p.display(), self.doc.plate + 1),
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
            filament_materials: self.doc.filaments.iter().map(|f| f.material.clone()).collect(),
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
        if self.xray_edges.len() > 256 {
            let keep: std::collections::HashSet<u64> = self.entries.iter().map(|e| e.key).collect();
            self.xray_edges.retain(|k, _| keep.contains(k));
        }
        self.rebuild_batches();
    }

    fn rebuild_batches(&mut self) {
        let mut solid = Batch::new(Pass::Opaque);
        let mut holes = Batch::new(Pass::Transparent);
        let mut lines = Batch::new(Pass::Lines);
        let mut overlay = Batch::new(Pass::Overlay);
        let fil = &self.doc.filaments;
        if let Some(pv) = &self.preview {
            let z = pv.layer as f64 * crate::preview::LAYER_H;
            for m in &pv.meshes {
                for t in 0..m.tris.len() {
                    let p = m.tri_points(t);
                    let col = fil.get(m.colors[t] as usize).map(|f| f.color).unwrap_or([200, 200, 200]);
                    for [a, b, c] in crate::preview::clip_below(p, z) {
                        solid.tri(a.as_vec3(), b.as_vec3(), c.as_vec3(), rgba(col, 1.0));
                    }
                    if let Some((a, b)) = crate::preview::section(p, z - 1e-4) {
                        let lc = [col[0] as f32 / 255.0 * 0.6 + 0.4, col[1] as f32 / 255.0 * 0.6 + 0.4, col[2] as f32 / 255.0 * 0.6 + 0.4, 1.0];
                        let lift = Vec3::new(0.0, 0.0, 0.02);
                        lines.line(a.as_vec3() + lift, b.as_vec3() + lift, lc);
                    }
                }
            }
        }
        for e in self.entries.iter().filter(|_| self.preview.is_none()) {
            let sel = self.selection.contains(&e.id);
            for t in 0..e.mesh.tris.len() {
                let [a, b, c] = e.mesh.tri_points(t);
                let (a, b, c) = (a.as_vec3(), b.as_vec3(), c.as_vec3());
                if e.hole {
                    holes.tri(a, b, c, crate::theme::hole_color(sel));
                } else {
                    let ci = e.mesh.colors[t];
                    let col = fil.get(ci as usize).map(|f| f.color).unwrap_or([200, 200, 200]);
                    if self.xray {
                        holes.tri(a, b, c, rgba(col, 0.28));
                    } else {
                        solid.tri(a, b, c, rgba(col, 1.0));
                    }
                }
            }
            if self.xray && !e.hole {
                // crisp outlines of the sharp edges so the see-through shape stays readable
                let edges = self.xray_edges.entry(e.key).or_insert_with(|| Arc::new(feature_edges(&e.mesh))).clone();
                let col = if sel { [0.35, 0.65, 1.0, 0.95] } else { [0.85, 0.88, 0.95, 0.55] };
                for &(a, b) in edges.iter() {
                    lines.line(a, b, col);
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
        // overhang view: faces that need support
        if self.show_overhangs {
            let lim = (90.0 - self.overhang_deg).to_radians().cos();
            for e in self.entries.iter().filter(|e| !e.hole) {
                let minz = e.mesh.bounds().min.z;
                for t in 0..e.mesh.tris.len() {
                    let (n, _) = e.mesh.tri_normal_area(t);
                    let [a, b, c] = e.mesh.tri_points(t);
                    if n.z < -lim && a.z.min(b.z).min(c.z) > minz.max(0.0) + 0.1 {
                        overlay.tri(a.as_vec3(), b.as_vec3(), c.as_vec3(), [1.0, 0.45, 0.1, 1.0]);
                    }
                }
            }
        }
        // push/pull: highlight face and preview the extrusion
        if let Some(pp) = &self.pp {
            if let Some(e) = self.entries.iter().find(|e| e.id == pp.id) {
                for &t in &pp.region {
                    if (t as usize) < e.mesh.tris.len() {
                        let [a, b, c] = e.mesh.tri_points(t as usize);
                        overlay.tri(a.as_vec3(), b.as_vec3(), c.as_vec3(), [0.95, 0.35, 0.75, 1.0]);
                    }
                }
                if pp.dist.abs() > 1e-6 {
                    let prism = e.mesh.region_prism(&pp.region, pp.n, 0.0, pp.dist, 0);
                    for t in 0..prism.tris.len() {
                        let [a, b, c] = prism.tri_points(t);
                        let col = if pp.dist > 0.0 { [0.2, 0.75, 0.35, 0.45] } else { [0.9, 0.2, 0.2, 0.45] };
                        holes.tri(a.as_vec3(), b.as_vec3(), c.as_vec3(), col);
                    }
                }
            }
        }
        // bed
        let p = &self.doc.printer;
        let (hx, hy) = (330.0f32 / 2.0, 320.0f32 / 2.0);
        let z = -0.05;
        let bc = crate::theme::bed_color();
        solid.tri(Vec3::new(-hx, -hy, z), Vec3::new(hx, -hy, z), Vec3::new(hx, hy, z), bc);
        solid.tri(Vec3::new(-hx, -hy, z), Vec3::new(hx, hy, z), Vec3::new(-hx, hy, z), bc);
        let gc = crate::theme::grid_color();
        let ga = crate::theme::grid_axis_color();
        let mut x = -150.0f32;
        while x <= 150.0 {
            lines.line(Vec3::new(x, -hy, 0.0), Vec3::new(x, hy, 0.0), if x == 0.0 { ga } else { gc });
            x += 10.0;
        }
        let mut y = -160.0f32;
        while y <= 160.0 {
            lines.line(Vec3::new(-hx, y, 0.0), Vec3::new(hx, y, 0.0), if y == 0.0 { ga } else { gc });
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

    /// Where a decal would go for this hover/click: (object id, local frame, world frame).
    fn decal_frame(&self, h: &Hit) -> Option<(u64, crate::decal::Frame, crate::decal::Frame)> {
        let d = self.decal.as_ref()?;
        let e = self.entries.get(h.entry)?;
        if e.hole || (h.tri as usize) >= e.mesh.tris.len() {
            return None;
        }
        let o = self.doc.find(e.id)?;
        let iv = self.camera.view().inverse();
        let (back, cam_up) = (iv.transform_vector3(DVec3::Z).normalize(), iv.transform_vector3(DVec3::Y).normalize());
        let n = if d.from_view {
            back
        } else {
            let (n, _) = e.mesh.tri_normal_area(h.tri as usize);
            if n.dot(back) < 0.0 { back } else { n }
        };
        let wf = d.frame(h.point, n, DVec3::Z, cam_up);
        let inv = o.transform().inverse();
        let lf = crate::decal::Frame {
            centre: inv.transform_point3(wf.centre),
            n: inv.transform_vector3(wf.n).normalize(),
            right: inv.transform_vector3(wf.right).normalize(),
            up: inv.transform_vector3(wf.up).normalize(),
        };
        Some((o.id, lf, wf))
    }

    fn stext_frame(&self, h: &Hit) -> Option<(u64, crate::surface_text::Frame)> {
        let e = self.entries.get(h.entry)?;
        if e.hole || (h.tri as usize) >= e.mesh.tris.len() {
            return None;
        }
        let iv = self.camera.view().inverse();
        let (back, cam_up) = (iv.transform_vector3(DVec3::Z).normalize(), iv.transform_vector3(DVec3::Y).normalize());
        let n = if self.stext.from_view {
            back
        } else {
            let (n, _) = e.mesh.tri_normal_area(h.tri as usize);
            if n.dot(back) < 0.0 { back } else { n }
        };
        Some((e.id, self.stext.frame(h.point, n, cam_up)))
    }

    fn apply_stext(&mut self, h: &Hit) {
        let Some((id, wf)) = self.stext_frame(h) else { return };
        let Some(o) = self.doc.find(id).cloned() else { return };
        if o.locked {
            self.status = "That part is locked.".into();
            return;
        }
        let mut wm = self.cache.world_mesh(&o);
        wm.fill_unset_color(o.color);
        match self.stext.apply(&wm, &wf) {
            Ok(mut m) => {
                let inv = o.transform().inverse();
                for v in &mut m.verts {
                    *v = inv.transform_point3(*v);
                }
                crate::shapes::orient_outward(&mut m);
                let (mm, size, off) = normalize_mesh(m);
                // the part may have grown (raised text): keep it where it was
                if let Some(ob) = self.doc.find_mut(id) {
                    // keep every point where it was: solve for the new position given the new size
                    let mir = DVec3::new(if ob.mirror[0] { -1.0 } else { 1.0 }, if ob.mirror[1] { -1.0 } else { 1.0 }, if ob.mirror[2] { -1.0 } else { 1.0 });
                    let rm = |v: DVec3| ob.rotation() * (v * mir);
                    let half = DVec3::new(0.0, 0.0, ob.size.z * 0.5);
                    let half2 = DVec3::new(0.0, 0.0, size.z * 0.5);
                    ob.pos = ob.pos + half - half2 + rm(off - half + half2);
                    ob.size = size;
                    ob.kind = Kind::Mesh(Arc::new(mm));
                    ob.paint = None;
                }
                self.selection = vec![id];
                self.touch();
                self.status = format!("{} '{}' on '{}'. Ctrl+Z undoes it.", if self.stext.engrave { "Cut" } else { "Raised" }, self.stext.text, o.name);
            }
            Err(e) => self.status = e,
        }
    }

    fn stext_panel(&mut self, ui: &mut egui::Ui) {
        crate::theme::section(ui, "Text on a part");
        ui.label("Move over a part to place the text, then click.");
        let st = &mut self.stext;
        ui.add(egui::TextEdit::singleline(&mut st.text).hint_text("Your text"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut st.engrave, false, "Raised");
            ui.radio_value(&mut st.engrave, true, "Cut in");
        });
        ui.add(egui::Slider::new(&mut st.height, 2.0..=80.0).text("letter height (mm)"));
        ui.add(egui::Slider::new(&mut st.depth, 0.2..=5.0).text(if st.engrave { "depth (mm)" } else { "raised by (mm)" }));
        ui.add(egui::Slider::new(&mut st.rotation, -180.0..=180.0).text("turn (°)"));
        ui.horizontal(|ui| {
            ui.label("Font");
            ui.radio_value(&mut st.font, crate::shapes::FontChoice::Sans, "Sans");
            ui.radio_value(&mut st.font, crate::shapes::FontChoice::Mono, "Mono");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Colour");
            for (i, f) in self.doc.filaments.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter().rect_filled(r, 3.0, c32(f.color));
                if st.color as usize == i {
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, crate::theme::accent()), egui::StrokeKind::Outside);
                }
                if resp.on_hover_text(&f.name).clicked() {
                    st.color = i as u8;
                }
            }
        });
        ui.checkbox(&mut st.from_view, "Project from my view").on_hover_text("Off: lies flat on the face you point at. On: straight from the screen — good for round parts.");
        ui.label(RichText::new("Letters at least 5 mm tall and 0.6 mm deep print cleanly. Cut-in letters are filled with the colour you pick.").small().weak());
        if ui.button("Done").clicked() {
            self.tool = Tool::Select;
        }
    }

    fn apply_decal(&mut self, h: &Hit) {
        let Some((id, lf, _)) = self.decal_frame(h) else { return };
        let Some(o) = self.doc.find(id).cloned() else { return };
        if o.locked {
            self.status = "That part is locked.".into();
            return;
        }
        let Some(d) = self.decal.clone() else { return };
        let mut m = (*self.cache.base_mesh(&o)).clone();
        if let Some(p) = o.paint.as_ref().filter(|p| p.len() == m.colors.len()) {
            for (c, &pc) in m.colors.iter_mut().zip(p) {
                if pc != NO_COLOR {
                    *c = pc;
                }
            }
        }
        m.fill_unset_color(o.color);
        let (out, n) = d.apply(&m, &lf, &self.doc.filaments);
        if n == 0 {
            self.status = "Nothing to paint there — try a face that points toward you.".into();
            return;
        }
        if let Some(ob) = self.doc.find_mut(id) {
            ob.kind = Kind::Mesh(Arc::new(out));
            ob.paint = None;
        }
        self.selection = vec![id];
        self.touch();
        self.tool = Tool::Select;
        self.status = "Picture painted on. Ctrl+Z (Cmd+Z on Mac) to undo; the part keeps its size and position but its shape settings no longer apply.".into();
    }

    fn decal_panel(&mut self, ui: &mut egui::Ui) {
        crate::theme::section(ui, "Picture on a part");
        let Some(d) = &mut self.decal else {
            ui.label(RichText::new("No picture loaded. Use File > Picture to 3D… and choose \"On a part's surface\".").weak());
            return;
        };
        ui.label("Move over a part to see where the picture lands, then click to paint it on.");
        if let Some(t) = &self.decal_tex {
            let w = 200.0f32;
            let h = (w * d.pic.h as f32 / d.pic.w.max(1) as f32).min(160.0);
            ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::new(w, h)));
        }
        ui.add(egui::Slider::new(&mut d.width, 5.0..=250.0).text("width (mm)"));
        ui.add(egui::Slider::new(&mut d.rotation, -180.0..=180.0).text("turn (°)"));
        ui.checkbox(&mut d.skip_background, "Leave the background unpainted");
        ui.checkbox(&mut d.from_view, "Project from my view").on_hover_text("Off: the picture lies flat on the face you point at. On: it is projected straight from the screen — best for wrapping around round parts.");
        ui.add_space(4.0);
        if ui.button("Cancel").clicked() {
            self.tool = Tool::Select;
        }
    }

    fn start_preview(&mut self) {
        let objs: Vec<Mesh> = crate::io::collect_export(&self.doc, &mut self.cache).into_iter().map(|o| o.mesh).collect();
        let est = crate::preview::estimate(&objs, &self.doc.filaments);
        let layer = match &self.preview {
            Some(p) if p.layer < p.est.layers => p.layer.min(est.layers),
            _ => est.layers,
        };
        self.preview = Some(PreviewState { meshes: objs, est, merges: None, layer, edits: self.edits });
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        if self.preview.as_ref().map(|p| p.edits != self.edits).unwrap_or(false) {
            self.start_preview();
            self.geometry_dirty = true;
        }
        let fils = self.doc.filaments.clone();
        let Some(pv) = &mut self.preview else { return };
        crate::theme::section(ui, "Print preview");
        if pv.meshes.is_empty() {
            ui.label("Nothing to print yet.");
            return;
        }
        let e = &pv.est;
        let before = pv.layer;
        ui.horizontal(|ui| {
            if ui.small_button("◀").clicked() && pv.layer > 1 {
                pv.layer -= 1;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 40.0).max(80.0);
            ui.add(egui::Slider::new(&mut pv.layer, 1..=e.layers).show_value(false));
            if ui.small_button("▶").clicked() && pv.layer < e.layers {
                pv.layer += 1;
            }
        });
        let l = pv.layer;
        ui.label(format!("Layer {} of {}  ·  {:.1} mm", l, e.layers, l as f64 * crate::preview::LAYER_H));
        let m = e.layer_colors.get(l - 1).copied().unwrap_or(0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Colours on this layer:");
            for c in 0..64u8 {
                if m & (1 << c) != 0 {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                    ui.painter().rect_filled(r, 3.0, c32(fils.get(c as usize).map(|f| f.color).unwrap_or([200, 200, 200])));
                }
            }
            let s = e.swaps_per_layer.get(l - 1).copied().unwrap_or(0);
            if s > 0 {
                ui.label(RichText::new(format!("{s} change{}", if s == 1 { "" } else { "s" })).weak());
            }
        });
        if pv.layer != before {
            self.geometry_dirty = true;
        }
        let e = &self.preview.as_ref().unwrap().est;
        ui.add_space(6.0);
        crate::theme::section(ui, "Estimate (Bambu Lab H2C, 0.2 mm layers)");
        egui::Grid::new("pv_est").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label("Print time");
            ui.label(RichText::new(format!("about {}", crate::preview::fmt_time(e.time_s))).strong());
            ui.end_row();
            ui.label("Colour changes");
            ui.label(RichText::new(e.swaps.to_string()).strong());
            ui.end_row();
            ui.label("Change waste");
            ui.label(format!("≈ {:.1} g", e.waste_g));
            ui.end_row();
            let total: f64 = e.grams.iter().map(|g| g.1).sum::<f64>() + e.waste_g;
            ui.label("Filament total");
            ui.label(RichText::new(format!("≈ {total:.0} g")).strong());
            ui.end_row();
        });
        ui.add_space(4.0);
        for (c, g) in &e.grams {
            ui.horizontal(|ui| {
                let f = fils.get(*c as usize);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, c32(f.map(|f| f.color).unwrap_or([200, 200, 200])));
                ui.label(format!("{} {}", f.map(|f| f.name.as_str()).unwrap_or("?"), f.map(|f| f.material.as_str()).unwrap_or("")));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{g:.1} g"));
                });
            });
        }
        if e.colors_used > 7 {
            ui.colored_label(Color32::from_rgb(240, 170, 60), format!("{} filaments: more than the H2C's 7 hotends, so some changes need a full purge (slower, more waste).", e.colors_used));
        } else if e.swaps > 300 {
            ui.label(RichText::new("Lots of colour changes. Fewer colours per layer (or colours that start at a layer boundary) print much faster.").small().color(Color32::from_rgb(240, 170, 60)));
        }
        ui.label(RichText::new("Rough estimate (±30%) using standard 15% infill and 2 walls. Bambu Studio shows exact numbers after slicing.").small().weak());
        if e.swaps > 0 {
            ui.add_space(6.0);
            crate::theme::section(ui, "Save time");
            let mut apply: Option<(u8, u8)> = None;
            match &self.preview.as_ref().unwrap().merges {
                None => {
                    if ui.button("Find ways to cut colour changes").clicked() {
                        let pv = self.preview.as_mut().unwrap();
                        pv.merges = Some(crate::preview::merge_suggestions(&pv.meshes, &fils, &pv.est));
                    }
                }
                Some(list) if list.is_empty() => {
                    ui.label(RichText::new("No single colour swap saves any changes — this design is already efficient.").small());
                }
                Some(list) => {
                    for m in list {
                        let (fa, fb) = (fils.get(m.from as usize), fils.get(m.to as usize));
                        ui.horizontal_wrapped(|ui| {
                            for f in [fa, fb] {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                ui.painter().rect_filled(r, 3.0, c32(f.map(|f| f.color).unwrap_or([128; 3])));
                            }
                            let look = if m.distance < 40.0 { "nearly the same colour" } else if m.distance < 120.0 { "a similar colour" } else { "a different colour" };
                            ui.label(format!(
                                "Print {} as {} ({look}): {} fewer changes, about {} faster",
                                fa.map(|f| f.name.as_str()).unwrap_or("?"),
                                fb.map(|f| f.name.as_str()).unwrap_or("?"),
                                m.swaps_saved,
                                crate::preview::fmt_time(m.time_saved_s)
                            ));
                            if ui.small_button("Apply").clicked() {
                                apply = Some((m.from, m.to));
                            }
                        });
                    }
                    ui.label(RichText::new("Apply recolours the design (Ctrl+Z undoes it).").small().weak());
                }
            }
            if let Some((from, to)) = apply {
                self.remap_color(from, to);
            }
        }
        if ui.button("Close preview").clicked() {
            self.preview = None;
            self.geometry_dirty = true;
        }
    }

    /// Filament slots actually used by visible parts (object colours and painting).
    fn used_slots(&self) -> Vec<u8> {
        let mut used = [false; 256];
        for o in &self.doc.objects {
            if o.hole || !o.visible {
                continue;
            }
            used[o.color as usize] = true;
            for &c in o.paint.iter().flatten() {
                used[c as usize] = true;
            }
            if let Kind::Mesh(m) = &o.kind {
                for &c in &m.colors {
                    used[c as usize] = true;
                }
            }
        }
        used[NO_COLOR as usize] = false;
        (0..255u8).filter(|&c| used[c as usize]).collect()
    }

    fn history_window(&mut self, ctx: &egui::Context) {
        let Some(list) = &self.history else { return };
        let mut open = true;
        let mut restore: Option<PathBuf> = None;
        let mut copy: Option<PathBuf> = None;
        let name = self.file.as_ref().and_then(|f| f.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        egui::Window::new("Version history").open(&mut open).default_width(460.0).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(format!("Every save of {name} is kept here (the last 100). Restoring loads that version; your current design stays in undo, so Ctrl+Z brings it back.")).small().weak());
            if list.is_empty() {
                ui.label("No saved versions yet — they appear from the next save on.");
            }
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                egui::Grid::new("hist").num_columns(4).striped(true).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (i, v) in list.iter().enumerate() {
                        ui.label(RichText::new(&v.when).strong());
                        ui.label(RichText::new(format!("{} part{} · {} KB{}", v.objects, if v.objects == 1 { "" } else { "s" }, (v.bytes + 1023) / 1024, if i == 0 { " · latest" } else { "" })).weak());
                        if ui.button("Restore").clicked() {
                            restore = Some(v.path.clone());
                        }
                        if ui.button("Save a copy…").clicked() {
                            copy = Some(v.path.clone());
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if let Some(p) = restore {
            match io::load_project(&p) {
                Ok(d) => {
                    self.doc = d;
                    self.selection.clear();
                    self.touch();
                    self.status = "Restored an earlier version. Save to keep it, or Ctrl+Z to go back.".into();
                    self.history = None;
                    return;
                }
                Err(e) => self.status = format!("Couldn't open that version: {e}"),
            }
        }
        if let Some(p) = copy {
            if let Some(dest) = rfd::FileDialog::new().add_filter("RoboForge project", &["pforge"]).set_file_name("earlier version.pforge").save_file() {
                let dest = if dest.extension().is_none() { dest.with_extension("pforge") } else { dest };
                self.status = match std::fs::copy(&p, &dest) {
                    Ok(_) => format!("Saved a copy as {}", dest.display()),
                    Err(e) => format!("Couldn't copy: {e}"),
                };
            }
        }
        if !open {
            self.history = None;
        }
    }

    fn my_filaments_window(&mut self, ctx: &egui::Context) {
        let Some(lib) = &mut self.my_fils else { return };
        let mut open = true;
        let mut add: Option<Filament> = None;
        let mut del: Option<usize> = None;
        let mut save_all = false;
        egui::Window::new("My Filaments").open(&mut open).collapsible(false).resizable(true).default_width(420.0).show(ctx, |ui| {
            ui.label(RichText::new("Spools you've saved with ★. They're kept on this computer for every design.").small().weak());
            if lib.is_empty() {
                ui.label("Nothing saved yet. Set up a filament in the Filaments list (colour, name, type), then click its ★.");
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, f) in lib.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 16.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, c32(f.color));
                        ui.label(RichText::new(&f.name).strong());
                        ui.label(RichText::new(format!("{} · {}", crate::materials::short(&f.material), f.hex())).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("🗑").on_hover_text("Remove from My Filaments").clicked() {
                                del = Some(i);
                            }
                            if ui.button("Add to design").clicked() {
                                add = Some(f.clone());
                            }
                        });
                    });
                }
            });
            ui.separator();
            if ui.button("★ Save all filaments in this design").clicked() {
                save_all = true;
            }
        });
        if let Some(i) = del {
            lib.remove(i);
            let _ = crate::materials::save_library(lib);
        }
        if save_all {
            for f in &self.doc.filaments {
                if !lib.iter().any(|x| x.name == f.name && x.color == f.color && x.material == f.material) {
                    lib.push(f.clone());
                }
            }
            let _ = crate::materials::save_library(lib);
            self.status = "Saved this design's filaments to My Filaments.".into();
        }
        if let Some(f) = add {
            if let Some(i) = self.doc.filaments.iter().position(|x| x.color == f.color && x.material == f.material) {
                self.status = format!("'{}' is already filament {} in this design.", f.name, i + 1);
            } else if self.doc.filaments.len() >= 24 {
                self.status = "This design already has 24 filaments (the most the H2C can use).".into();
            } else {
                self.status = format!("Added '{}' as filament {}.", f.name, self.doc.filaments.len() + 1);
                self.doc.filaments.push(f);
                self.touch();
            }
        }
        if !open {
            self.my_fils = None;
        }
    }

    fn ai_send(&mut self, ctx: &egui::Context) {
        let text = self.ai.input.trim().to_string();
        if text.is_empty() || self.ai.rx.is_some() {
            return;
        }
        self.ai.input.clear();
        self.ai.shown.push((true, text.clone()));
        self.ai.history.push(crate::ai::Msg { user: true, text });
        let keep = self.ai.history.len().saturating_sub(10);
        let history: Vec<crate::ai::Msg> = self.ai.history[keep..].to_vec();
        let system = crate::ai::system_prompt(&self.doc, &self.selection);
        let settings = self.ai.settings.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::ai::chat(&settings, &system, &history));
            ctx2.request_repaint();
        });
        self.ai.rx = Some(rx);
        self.ai.started = Some(std::time::Instant::now());
    }

    fn ai_poll(&mut self) {
        if let Some(rx) = &self.ai.test_rx {
            if let Ok(r) = rx.try_recv() {
                self.ai.test_result = match r {
                    Ok(s) => format!("✔ {s}"),
                    Err(e) => format!("✖ {e}"),
                };
                self.ai.test_rx = None;
            }
        }
        let Some(rx) = &self.ai.rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.ai.rx = None;
        match r {
            Err(e) => {
                self.ai.history.pop();
                self.ai.shown.push((false, format!("⚠ {e}")));
            }
            Ok(reply) => {
                let raw: String = reply.chars().take(6000).collect();
                match crate::ai::extract_json(&reply) {
                    Some(plan) if plan["actions"].as_array().map(|a| !a.is_empty()).unwrap_or(false) => {
                        let r = crate::ai::apply(&plan, &mut self.doc, &mut self.cache);
                        let mut msg = if r.say.is_empty() { "Done.".to_string() } else { r.say.clone() };
                        if !r.problems.is_empty() {
                            msg.push_str(&format!("\n(Skipped: {})", r.problems.join("; ")));
                        }
                        msg.push_str("\nNot right? Ctrl+Z undoes it, or tell me what to change.");
                        self.ai.shown.push((false, msg));
                        self.ai.history.push(crate::ai::Msg { user: false, text: raw });
                        if !r.changed.is_empty() {
                            self.selection = r.changed.clone();
                        }
                        self.touch();
                        self.status = "AI helper changed the design.".into();
                    }
                    Some(plan) => {
                        let say = plan["say"].as_str().unwrap_or("(no changes)").to_string();
                        self.ai.shown.push((false, say));
                        self.ai.history.push(crate::ai::Msg { user: false, text: raw });
                    }
                    None => {
                        let t: String = reply.trim().chars().take(600).collect();
                        self.ai.shown.push((false, format!("{t}\n\n(The AI didn't send a plan RoboForge can build. Try rephrasing, or a bigger model.)")));
                        self.ai.history.push(crate::ai::Msg { user: false, text: raw });
                    }
                }
            }
        }
    }

    fn ai_window(&mut self, ctx: &egui::Context) {
        self.ai_poll();
        if !self.ai.open {
            return;
        }
        let mut open = true;
        let mut send = false;
        let mut test = false;
        egui::Window::new("✨ AI helper").open(&mut open).default_width(400.0).default_pos([300.0, 120.0]).resizable(true).show(ctx, |ui| {
            let busy = self.ai.rx.is_some();
            ui.horizontal(|ui| {
                let who = match self.ai.settings.provider {
                    crate::ai::Provider::OpenAi => format!("Local / OpenAI-style · {}", if self.ai.settings.model.is_empty() { "default model" } else { &self.ai.settings.model }),
                    crate::ai::Provider::Anthropic => format!("Claude · {}", if self.ai.settings.model.is_empty() { "claude-sonnet-5-5" } else { &self.ai.settings.model }),
                };
                ui.label(RichText::new(who).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(if self.ai.show_settings { "Hide settings" } else { "⚙ Settings" }).clicked() {
                        self.ai.show_settings = !self.ai.show_settings;
                    }
                    if ui.small_button("New chat").on_hover_text("Forget the conversation (the design stays)").clicked() {
                        self.ai.shown.clear();
                        self.ai.history.clear();
                    }
                });
            });
            if self.ai.show_settings {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    let st = &mut self.ai.settings;
                    ui.horizontal(|ui| {
                        if ui.radio_value(&mut st.provider, crate::ai::Provider::OpenAi, "Local model (llama.cpp, Ollama, LM Studio…)").clicked() && st.url.contains("anthropic") {
                            st.url = "http://localhost:8080/v1".into();
                        }
                    });
                    ui.radio_value(&mut st.provider, crate::ai::Provider::Anthropic, "Claude (Anthropic API key)");
                    egui::Grid::new("ai_set").num_columns(2).show(ui, |ui| {
                        if st.provider == crate::ai::Provider::OpenAi {
                            ui.label("Address");
                            ui.add(egui::TextEdit::singleline(&mut st.url).desired_width(240.0)).on_hover_text("llama.cpp server: http://localhost:8080/v1 · Ollama: http://localhost:11434/v1 · LM Studio: http://localhost:1234/v1 · another PC: use its IP");
                            ui.end_row();
                        }
                        ui.label("Model");
                        ui.add(egui::TextEdit::singleline(&mut st.model).desired_width(240.0).hint_text(if st.provider == crate::ai::Provider::Anthropic { "claude-sonnet-5-5" } else { "blank = whatever is loaded" }));
                        ui.end_row();
                        ui.label("Key");
                        ui.add(egui::TextEdit::singleline(&mut st.key).password(true).desired_width(240.0).hint_text(if st.provider == crate::ai::Provider::Anthropic { "sk-ant-…" } else { "usually blank for local" }));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.ai.test_rx.is_none(), egui::Button::new("Save & test connection")).clicked() {
                            test = true;
                        }
                        if self.ai.test_rx.is_some() {
                            ui.spinner();
                        }
                    });
                    if !self.ai.test_result.is_empty() {
                        ui.label(RichText::new(&self.ai.test_result).small());
                    }
                    ui.label(RichText::new("Settings (including the key) are stored on this computer only.").small().weak());
                });
            }
            egui::ScrollArea::vertical().max_height(300.0).stick_to_bottom(true).show(ui, |ui| {
                if self.ai.shown.is_empty() {
                    ui.label(RichText::new("Try: \"a 60 mm box with a hinged lid and ROBERT raised on top in blue\", \"a phone stand with my name on the front\", or \"make the selected part 10 mm taller\".").weak());
                }
                for (user, t) in &self.ai.shown {
                    let (bg, al) = if *user { (crate::theme::accent().gamma_multiply(0.25), egui::Align::Max) } else { (Color32::from_gray(45), egui::Align::Min) };
                    ui.with_layout(egui::Layout::top_down(al), |ui| {
                        egui::Frame::new().fill(bg).corner_radius(6.0).inner_margin(6.0).show(ui, |ui| {
                            ui.set_max_width(320.0);
                            ui.label(t);
                        });
                    });
                    ui.add_space(4.0);
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let secs = self.ai.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                        ui.label(RichText::new(format!("Thinking… {secs}s (local models can take a minute)")).weak());
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let r = ui.add_enabled(!busy, egui::TextEdit::multiline(&mut self.ai.input).desired_rows(2).desired_width(ui.available_width() - 60.0).hint_text("Describe what to make or change… (Ctrl+V or right-click to paste)"));
                // Ctrl+V backup: if the system paste event didn't arrive, read the clipboard ourselves
                if r.has_focus() {
                    let (ctrl_v, got_paste) = ui.input(|i| {
                        (
                            i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command)),
                            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
                        )
                    });
                    if ctrl_v && !got_paste {
                        if let Some(t) = clipboard_text() {
                            self.ai.input.push_str(&t);
                        }
                    }
                }
                r.context_menu(|ui| {
                    if ui.button("Paste").clicked() {
                        if let Some(t) = clipboard_text() {
                            self.ai.input.push_str(&t);
                        }
                        ui.close();
                    }
                    if ui.button("Copy all").clicked() {
                        ui.ctx().copy_text(self.ai.input.clone());
                        ui.close();
                    }
                    if ui.button("Clear").clicked() {
                        self.ai.input.clear();
                        ui.close();
                    }
                });
                if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift) {
                    // Enter sends, Shift+Enter makes a new line
                    if self.ai.input.ends_with('\n') {
                        self.ai.input.pop();
                    }
                    send = true;
                }
                if ui.add_enabled(!busy, egui::Button::new("Send")).clicked() {
                    send = true;
                }
            });
        });
        if test {
            self.ai.settings.save();
            let st = self.ai.settings.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let c2 = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(crate::ai::test(&st));
                c2.request_repaint();
            });
            self.ai.test_rx = Some(rx);
            self.ai.test_result.clear();
        }
        if send {
            self.ai.settings.save();
            self.ai_send(ctx);
        }
        if self.ai.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !open {
            self.ai.open = false;
        }
    }

    fn apply_thread(&mut self, id: u64) {
        let Some(o) = self.doc.find(id).cloned() else { return };
        let mut wm = self.cache.world_mesh(&o);
        wm.fill_unset_color(o.color);
        let b = wm.bounds();
        let d = self.thread_d.filter(|t| t.0 == id).map(|t| t.1).unwrap_or((b.max.x - b.min.x).min(b.max.y - b.min.y));
        let opts = crate::parts::ThreadOptions {
            outside: self.thread_outside,
            diameter: d,
            pitch: self.thread_pitch,
            length: self.thread_len,
            at_top: self.thread_top,
            gap: crate::settings::fit_gap().unwrap_or(0.2),
        };
        let centre = glam::DVec2::new((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0);
        match crate::parts::add_thread(&wm, centre, &opts) {
            Ok(mut m) => {
                for c in m.colors.iter_mut() {
                    if *c == NO_COLOR {
                        *c = o.color;
                    }
                }
                let inv = o.transform().inverse();
                for v in &mut m.verts {
                    *v = inv.transform_point3(*v);
                }
                crate::shapes::orient_outward(&mut m);
                if let Some(ob) = self.doc.find_mut(id) {
                    ob.kind = Kind::Mesh(Arc::new(m));
                    ob.paint = None;
                }
                self.touch();
                self.status = format!("Added a {:.1} mm {} (pitch {:.2} mm) to '{}'.", d, if self.thread_outside { "screw thread" } else { "threaded hole" }, self.thread_pitch, o.name);
            }
            Err(e) => self.status = e,
        }
    }

    fn apply_split(&mut self, id: u64) {
        let Some(o) = self.doc.find(id).cloned() else { return };
        let Some((_, axis, at)) = self.split_at.filter(|s| s.0 == id) else { return };
        let mut wm = self.cache.world_mesh(&o);
        wm.fill_unset_color(o.color);
        let gap = crate::settings::fit_gap().unwrap_or(0.15).max(0.05);
        let opts = crate::split::SplitOptions { axis, at, pins: self.split_pins, pin_d: self.split_pin_d, pin_len: (self.split_pin_d * 2.0).max(6.0), gap };
        let r = match crate::split::split(&wm, &opts) {
            Ok(r) => r,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        self.doc.objects.retain(|x| x.id != id);
        let mut ids = vec![];
        for (k, m) in [r.low, r.high].into_iter().enumerate() {
            let (mm, size, off) = normalize_mesh(m);
            let nid = self.doc.alloc_id();
            let mut ob = Object::new(nid, &format!("{} (piece {})", o.name, k + 1), Kind::Mesh(Arc::new(mm)), size);
            ob.color = o.color;
            ob.pos = off;
            if k == 1 {
                if axis == 2 {
                    // top piece: put it on the plate, cut face down, beside the bottom piece
                    ob.pos.z = 0.0;
                    ob.pos.x += (wm.bounds().max.x - wm.bounds().min.x) + 10.0;
                } else {
                    ob.pos[axis] += 10.0;
                }
            }
            self.doc.objects.push(ob);
            ids.push(nid);
        }
        let b = wm.bounds();
        for (i, _) in r.pin_spots.iter().enumerate() {
            let nid = self.doc.alloc_id();
            let d = self.split_pin_d;
            let len = (d * 2.0).max(6.0);
            let mut ob = Object::new(nid, "Alignment pin", Kind::Shape(Shape::Cylinder { sides: 32, bevel: 0.4 }), DVec3::new(d, d, len));
            ob.color = o.color;
            ob.pos = DVec3::new(b.min.x + i as f64 * (d + 5.0) + d / 2.0, b.min.y - d - 8.0, 0.0);
            self.doc.objects.push(ob);
            ids.push(nid);
        }
        self.selection = ids;
        self.split_at = None;
        self.touch();
        self.status = if r.pin_spots.is_empty() && self.split_pins {
            "Split into 2 pieces. No room for alignment pins on this cut (needs a thicker cross-section).".into()
        } else {
            format!("Split into 2 pieces{}. Click Make it printable to lay them flat.", if r.pin_spots.is_empty() { String::new() } else { format!(" with {} alignment pin(s)", r.pin_spots.len()) })
        };
    }

    fn apply_edges(&mut self, id: u64) {
        let Some(o) = self.doc.find(id).cloned() else { return };
        let mut wm = self.cache.world_mesh(&o);
        wm.fill_unset_color(o.color);
        let opts = crate::edges::EdgeOptions { radius: self.edge_radius, chamfer: self.edge_chamfer, skip_bottom: self.edge_skip_bottom, min_angle: 30.0 };
        match crate::edges::round_edges(&wm, &opts) {
            Ok(mut m) => {
                let inv = o.transform().inverse();
                for v in &mut m.verts {
                    *v = inv.transform_point3(*v);
                }
                crate::shapes::orient_outward(&mut m);
                if let Some(ob) = self.doc.find_mut(id) {
                    ob.kind = Kind::Mesh(Arc::new(m));
                    ob.paint = None;
                }
                self.touch();
                self.status = format!("{} edges of '{}' by {:.1} mm.", if self.edge_chamfer { "Bevelled" } else { "Rounded" }, o.name, self.edge_radius);
            }
            Err(e) => self.status = e,
        }
    }

    /// Everything printed in filament `from` is printed in `to` instead.
    fn remap_color(&mut self, from: u8, to: u8) {
        fn fix(o: &mut Object, from: u8, to: u8) {
            if o.color == from {
                o.color = to;
            }
            if let Some(p) = &mut o.paint {
                for c in p.iter_mut() {
                    if *c == from {
                        *c = to;
                    }
                }
            }
            match &mut o.kind {
                Kind::Mesh(m) if m.colors.contains(&from) => {
                    let mut mm = (**m).clone();
                    for c in mm.colors.iter_mut() {
                        if *c == from {
                            *c = to;
                        }
                    }
                    *m = Arc::new(mm);
                }
                Kind::Group { children, .. } => {
                    for ch in children.iter_mut() {
                        fix(ch, from, to);
                    }
                }
                Kind::Shape(Shape::QrCode { dark, .. }) if *dark == from as u32 => *dark = to as u32,
                _ => {}
            }
        }
        for o in self.doc.objects.iter_mut() {
            fix(o, from, to);
        }
        self.touch();
        let names = |i: u8| self.doc.filaments.get(i as usize).map(|f| f.name.clone()).unwrap_or_default();
        self.status = format!("{} is now printed as {}.", names(from), names(to));
    }

    fn sketch_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.sketch else { return };
        match ed.show(ctx) {
            crate::sketch::SketchResult::None => {}
            crate::sketch::SketchResult::Cancel => self.sketch = None,
            crate::sketch::SketchResult::Done(shape) => {
                let editing = ed.editing;
                self.sketch = None;
                let size = shape.natural_size().unwrap_or(DVec3::splat(20.0));
                if let Some(o) = editing.and_then(|id| self.doc.find_mut(id)) {
                    o.kind = Kind::Shape(shape);
                    o.size = size;
                    self.selection = vec![o.id];
                    self.status = "Sketch updated.".into();
                } else {
                    let id = self.doc.alloc_id();
                    let name = shape.label().to_string();
                    let mut o = Object::new(id, &name, Kind::Shape(shape), size);
                    o.color = self.doc.objects.len() as u8 % self.doc.filaments.len().max(1) as u8;
                    o.pos = self.free_spot(size);
                    self.doc.objects.push(o);
                    self.selection = vec![id];
                    self.status = "Sketch added. Resize it like any part, or click Edit sketch… in the Inspector.".into();
                }
                self.touch();
            }
        }
    }

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
            // Real brush: refine the surface under the brush so it paints a round dab
            // instead of whole (possibly huge) triangles.
            let r = self.brush_mm.max(0.05);
            let target = (r * 0.5).max(0.2);
            let mut m = local.clone();
            for (c, &pc) in m.colors.iter_mut().zip(&paint) {
                if pc != NO_COLOR {
                    *c = pc;
                }
            }
            let refined = m.refine_near(lp, r + target, target, 2_500_000);
            let mut out = refined;
            let mut any = false;
            for t in 0..out.tris.len() {
                let [a, b, c] = out.tri_points(t);
                let cen = (a + b + c) / 3.0;
                if cen.distance(lp) <= r || crate::mesh::closest_on_tri(lp, a, b, c).distance(lp) <= r * 0.35 {
                    if out.colors[t] != value {
                        out.colors[t] = value;
                        any = true;
                    }
                }
            }
            if any || out.tris.len() != n {
                if let Some(ob) = self.doc.find_mut(id) {
                    if matches!(ob.kind, Kind::Shape(_)) && !self.status.starts_with("Painting turned") {
                        self.status = "Painting turned this shape into a mesh (size/position still editable; shape settings no longer apply).".into();
                    }
                    ob.kind = Kind::Mesh(Arc::new(out));
                    ob.paint = None;
                }
                self.touch();
            }
            return;
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
            ui.label(RichText::new("RoboForge").strong().size(15.0).color(crate::theme::accent()));
            ui.add_space(6.0);
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.guarded(PendingAction::New, &ui.ctx().clone());
                    ui.close();
                }
                if ui.button("Open project…").clicked() {
                    self.guarded(PendingAction::Open, &ui.ctx().clone());
                    ui.close();
                }
                ui.menu_button("Open recent", |ui| {
                    let rec = crate::settings::recent();
                    if rec.is_empty() {
                        ui.label(RichText::new("Nothing yet").weak());
                    }
                    for p in rec {
                        let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.button(&name).on_hover_text(p.display().to_string()).clicked() {
                            let ctx = ui.ctx().clone();
                            if p.extension().map(|e| e.eq_ignore_ascii_case("pforge")).unwrap_or(false) {
                                self.guarded(PendingAction::OpenPath(p.clone()), &ctx);
                            } else {
                                self.import_path(&p);
                            }
                            ui.close();
                        }
                    }
                });
                if ui.button("Save").clicked() {
                    self.save(false);
                    ui.close();
                }
                if ui.add_enabled(self.file.is_some(), egui::Button::new("Version history…")).on_disabled_hover_text("Save the design first — every save after that is kept").clicked() {
                    self.history = self.file.as_ref().map(|f| crate::history::list(f));
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
                if ui.button("Picture to 3D (lithophane, relief, colour plate)…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "webp", "gif"]).pick_file() {
                        self.open_image(p);
                    }
                    ui.close();
                }
                if ui.button("Sketch a shape…").clicked() {
                    self.sketch = Some(crate::sketch::SketchEditor::new());
                    ui.close();
                }
                if ui.button("Picture onto a part's surface…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "webp", "gif"]).pick_file() {
                        self.open_image(p);
                        if let Some(d) = &mut self.img_dialog {
                            d.decal = true;
                        }
                    }
                    ui.close();
                }
                if ui.button("Logo from SVG…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).pick_file() {
                        self.open_svg(p);
                    }
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
                ui.separator();
                let mut d = crate::theme::dark();
                if ui.checkbox(&mut d, "Dark theme").changed() {
                    crate::theme::apply(ui.ctx(), d);
                    crate::theme::save(d);
                    self.rebuild_batches();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Controls & shortcuts").clicked() {
                    self.show_help = true;
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("▶ Print").strong().color(crate::theme::export_green())).on_hover_text("Check the design, then open it in Bambu Studio ready to slice and print (Ctrl+P)").clicked() {
                    self.request_print();
                }
                if ui.button(RichText::new("⬆ Export to Bambu").color(crate::theme::export_green())).on_hover_text("Save a Bambu Studio file somewhere you choose").clicked() {
                    self.request_export(ExportKind::Bambu);
                }
            if ui.button("✨ Make it printable").on_hover_text("Turn parts onto their best face, put everything on the plate, spread parts out, then check").clicked() {
                self.make_printable();
            }
            if ui.selectable_label(self.preview.is_some(), "Preview").on_hover_text("Layer-by-layer preview with print time, filament and colour-change estimates").clicked() {
                if self.preview.is_some() {
                    self.preview = None;
                } else {
                    self.start_preview();
                }
                self.geometry_dirty = true;
            }
            if ui.selectable_label(self.xray, "👁 See-through").on_hover_text("Make every part see-through so you can look inside — e.g. to check a hole you cut (X)").clicked() {
                self.xray = !self.xray;
                self.geometry_dirty = true;
            }
            if ui.selectable_label(self.show_overhangs, "Overhangs").on_hover_text("Show faces that will need support (orange)").clicked() {
                self.show_overhangs = !self.show_overhangs;
                self.geometry_dirty = true;
            }
                let checking = self.check_rx.is_some();
                if ui.add_enabled(!checking, egui::Button::new(RichText::new("✔ Check printability").strong())).clicked() {
                    self.start_check();
                }
            });
        });
    }

    fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut tool = self.tool;
            ui.selectable_value(&mut tool, Tool::Select, "Select");
            ui.selectable_value(&mut tool, Tool::Paint, "Paint");
            ui.selectable_value(&mut tool, Tool::Fill, "Fill");
            ui.selectable_value(&mut tool, Tool::LayFlat, "Lay flat");
            ui.selectable_value(&mut tool, Tool::SnapFaces, "Snap faces");
            ui.selectable_value(&mut tool, Tool::PushPull, "Push/Pull");
            ui.selectable_value(&mut tool, Tool::Measure, "Measure");
            if ui.selectable_label(self.ai.open, "✨ AI helper").on_hover_text("Describe what you want and an AI builds it (your local model or Claude)").clicked() {
                self.ai.open = !self.ai.open;
            }
            if ui.selectable_label(self.tool == Tool::SurfaceText, "Aa Text on part").on_hover_text("Raised or cut-in lettering on any face of a part, flat or curved").clicked() {
                tool = if self.tool == Tool::SurfaceText { Tool::Select } else { Tool::SurfaceText };
            }
            if ui.button("✏ Sketch").on_hover_text("Draw your own outline, then push it up or spin it into a 3D part").clicked() {
                self.sketch = Some(crate::sketch::SketchEditor::new());
            }
            if tool != self.tool {
                self.tool = tool;
                self.measure.clear();
                self.snap_from = None;
                self.pp = None;
                self.geometry_dirty = true;
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
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            crate::theme::section(ui, "Shapes");
            ui.label(RichText::new("Click to add, or drag onto the plate or another object. Make any shape a Hole to cut with it.").small().weak());
            if self.thumbs.is_empty() {
                for i in 0..LIBRARY.len() {
                    let img = egui::ColorImage::from_rgba_premultiplied([crate::thumbs::THUMB, crate::thumbs::THUMB], &crate::thumbs::render(i));
                    self.thumbs.push(ui.ctx().load_texture(format!("shape-thumb-{i}"), img, egui::TextureOptions::LINEAR));
                }
            }
            let cols = ((ui.available_width() / 74.0).floor() as usize).max(2);
            let tile = Vec2::new((ui.available_width() - (cols as f32 - 1.0) * 4.0) / cols as f32, 74.0);
            for cat in ["Basic shapes", "Moving parts", "Electronics", "Hardware", "Mounting", "Home & storage"] {
            if cat != "Basic shapes" {
                ui.add_space(4.0);
                ui.label(RichText::new(cat.to_uppercase()).size(10.5).strong().color(crate::theme::header()));
            }
            egui::Grid::new(("shapes", cat)).num_columns(cols).spacing([4.0, 4.0]).show(ui, |ui| {
                let mut k = 0usize;
                for (i, info) in LIBRARY.iter().enumerate().filter(|(_, x)| x.category == cat) {
                    // one widget that both clicks (add) and drags (drop onto the plate/a part)
                    let (rect, r) = ui.allocate_exact_size(tile, Sense::click_and_drag());
                    let bg = if r.hovered() || r.dragged() { crate::theme::tile_hover() } else { crate::theme::tile_bg() };
                    ui.painter().rect_filled(rect, 6.0, bg);
                    if r.hovered() {
                        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, crate::theme::accent()), egui::StrokeKind::Inside);
                    }
                    let img_rect = Rect::from_center_size(rect.center() - Vec2::new(0.0, 8.0), Vec2::splat(50.0));
                    ui.painter().image(self.thumbs[i].id(), img_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    ui.painter().text(Pos2::new(rect.center().x, rect.bottom() - 9.0), egui::Align2::CENTER_CENTER, info.name, egui::FontId::proportional(11.5), crate::theme::tile_text());
                    let r = r.on_hover_text(format!("{} — click to add, or drag onto the plate or another part", info.name));
                    if r.clicked() {
                        self.add_shape(i);
                    }
                    if r.drag_started() || r.dragged() {
                        r.dnd_set_drag_payload(i);
                    }
                    if r.dragged() {
                        if let Some(p) = ui.ctx().pointer_hover_pos() {
                            let layer = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dnd")));
                            layer.image(self.thumbs[i].id(), Rect::from_center_size(p + Vec2::new(4.0, 4.0), Vec2::splat(44.0)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha(200));
                        }
                    }
                    k += 1;
                    if k % cols == 0 {
                        ui.end_row();
                    }
                }
            });
            }
            ui.add_space(4.0);
            if ui.add_sized([ui.available_width(), 24.0], egui::Button::new("Import STL / OBJ / 3MF…")).clicked() {
                self.import();
            }
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 6.0) / 2.0;
                if ui.add_sized([w, 24.0], egui::Button::new("Picture to 3D…")).on_hover_text("Lithophane, raised relief or colour photo plate").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "webp", "gif"]).pick_file() {
                        self.open_image(p);
                    }
                }
                if ui.add_sized([w, 24.0], egui::Button::new("Logo from SVG…")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).pick_file() {
                        self.open_svg(p);
                    }
                }
            });
            if ui.add_sized([ui.available_width(), 28.0], egui::Button::new(RichText::new("✏ Sketch a shape…").strong())).on_hover_text("Draw your own outline, then push it up or spin it into a 3D part").clicked() {
                self.sketch = Some(crate::sketch::SketchEditor::new());
            }
            if ui.add_sized([ui.available_width(), 24.0], egui::Button::new("Picture onto a part…")).on_hover_text("Paint a photo or logo onto a part's surface in filament colours").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "webp", "gif"]).pick_file() {
                    self.open_image(p);
                    if let Some(d) = &mut self.img_dialog {
                        d.decal = true;
                    }
                }
            }
            ui.separator();
            crate::theme::section(ui, "Filaments");
            ui.label(RichText::new("These become the filament list in Bambu Studio, each with Bambu's own settings for its type. Match them to your AMS spools.").small().weak());
            let mut remove = false;
            let n = self.doc.filaments.len();
            let mut changed = false;
            let mut to_lib: Option<Filament> = None;
            for i in 0..n {
                ui.horizontal(|ui| {
                    let f = &mut self.doc.filaments[i];
                    ui.label(format!("{:>2}", i + 1));
                    if ui.color_edit_button_srgb(&mut f.color).changed() {
                        changed = true;
                    }
                    if ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(78.0)).changed() {
                        changed = true;
                    }
                    if ui.small_button("★").on_hover_text("Save this filament to My Filaments").clicked() {
                        to_lib = Some(f.clone());
                    }
                    let active = self.paint_slot as usize == i;
                    if ui.radio(active, "").on_hover_text("Use for the paint brush").clicked() {
                        self.paint_slot = i as u8;
                        self.erase = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    let f = &mut self.doc.filaments[i];
                    // hex colour code, kept as typed while editing
                    let id = egui::Id::new(("fil_hex", i));
                    let mut txt = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| f.hex());
                    let r = ui.add(egui::TextEdit::singleline(&mut txt).desired_width(62.0).font(egui::TextStyle::Monospace)).on_hover_text("Colour code, e.g. #C8102E (from the spool label or Bambu's website)");
                    if r.changed() {
                        if let Some(c) = crate::materials::parse_hex(&txt) {
                            f.color = c;
                            changed = true;
                        }
                    }
                    if r.has_focus() {
                        ui.data_mut(|d| d.insert_temp(id, txt));
                    } else {
                        ui.data_mut(|d| d.remove::<String>(id));
                    }
                    let cur = crate::materials::canonical(&f.material).name.clone();
                    egui::ComboBox::from_id_salt(("fil_mat", i)).width(104.0).height(440.0).selected_text(crate::materials::short(&f.material)).show_ui(ui, |ui| {
                        let mut last_generic = false;
                        for m in crate::materials::all() {
                            let g = m.name.starts_with("Generic");
                            if g && !last_generic {
                                ui.separator();
                            }
                            last_generic = g;
                            if ui.selectable_label(m.name == cur, m.name.trim_start_matches("Bambu ")).on_hover_text(format!("{} · {} °C · {} g/cm³", m.kind, m.nozzle_temp, m.density)).clicked() {
                                f.material = m.name.clone();
                                changed = true;
                            }
                        }
                    });
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(n < 24, egui::Button::new("+ Add")).clicked() {
                    self.doc.filaments.push(Filament { name: format!("Filament {}", n + 1), color: [128, 128, 128], material: "Bambu PLA Basic".into() });
                    changed = true;
                }
                if ui.add_enabled(n > 1, egui::Button::new("− Remove last")).clicked() {
                    remove = true;
                }
                if ui.button("★ My Filaments…").on_hover_text("Your saved spools — add them to any design").clicked() {
                    self.my_fils = Some(crate::materials::library());
                }
            });
            {
                let refs: Vec<&Filament> = self.used_slots().into_iter().filter_map(|c| self.doc.filaments.get(c as usize)).collect();
                for w in crate::materials::warnings(&refs) {
                    ui.label(RichText::new(format!("⚠ {w}")).small().color(Color32::from_rgb(240, 170, 60)));
                }
            }
            if let Some(f) = to_lib {
                let mut lib = crate::materials::library();
                lib.retain(|x| !(x.name == f.name && x.color == f.color && x.material == f.material));
                lib.push(f.clone());
                self.status = match crate::materials::save_library(&lib) {
                    Ok(()) => format!("Saved '{}' to My Filaments.", f.name),
                    Err(e) => format!("Couldn't save My Filaments: {e}"),
                };
            }
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
            crate::theme::section(ui, "Objects");
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
                    ui.painter().rect_filled(r, 2.0, col); ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Inside);
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
            if self.preview.is_some() {
                self.preview_panel(ui);
                ui.separator();
            }
            match self.tool {
                Tool::Paint | Tool::Fill => self.paint_panel(ui),
                Tool::Measure => {
                    crate::theme::section(ui, "Measure");
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
                Tool::Decal => self.decal_panel(ui),
                Tool::SurfaceText => self.stext_panel(ui),
                Tool::PushPull => {
                    crate::theme::section(ui, "Push / Pull a face");
                    ui.label("Click a flat face, then drag it outward to extrude or inward to cut. Or type an exact distance below.");
                    let mut apply = None;
                    if let Some(pp) = &mut self.pp {
                        ui.horizontal(|ui| {
                            ui.label("Distance");
                            ui.add(egui::DragValue::new(&mut pp.dist).speed(0.1).suffix(" mm"));
                        });
                        ui.label(RichText::new("Positive = extrude out, negative = push in.").small().weak());
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new("Apply").strong()).clicked() {
                                apply = Some(pp.dist);
                            }
                            if ui.button("Cancel").clicked() {
                                apply = Some(f64::NAN);
                            }
                        });
                    } else {
                        ui.label(RichText::new("No face selected.").weak());
                    }
                    match apply {
                        Some(d) if d.is_nan() => {
                            self.pp = None;
                            self.rebuild_batches();
                        }
                        Some(d) => self.apply_pushpull(d),
                        None => {}
                    }
                    ui.label(RichText::new("The part becomes a mesh afterwards (size/position still editable).").small().weak());
                }
                Tool::SnapFaces => {
                    crate::theme::section(ui, "Snap faces together");
                    if self.snap_from.is_none() {
                        ui.label("1. Click the face of the part you want to MOVE.");
                    } else {
                        ui.label(RichText::new("2. Now click the face it should attach to.").strong());
                        if ui.button("Cancel").clicked() {
                            self.snap_from = None;
                        }
                    }
                    ui.label(RichText::new("The first part is turned and moved so the two faces sit flush, touching at the points you clicked.").small().weak());
                }
                Tool::LayFlat => {
                    crate::theme::section(ui, "Lay flat");
                    ui.label("Click a face of an object; it'll be rotated so that face sits on the plate. Great for reducing supports.");
                }
                Tool::Select => {}
            }
            if self.tool == Tool::Select || !self.selection.is_empty() {
                self.split_preview = None;
                self.inspector(ui);
                if let Some(id) = self.thread_apply.take() {
                    self.apply_thread(id);
                }
                if let Some(id) = self.split_apply.take() {
                    self.apply_split(id);
                }
                if let Some(id) = self.edge_apply.take() {
                    self.apply_edges(id);
                }
            }
            ui.separator();
            self.check_panel(ui);
        });
    }

    fn paint_panel(&mut self, ui: &mut egui::Ui) {
        crate::theme::section(ui, if self.tool == Tool::Paint { "Paint brush" } else { "Bucket fill" });
        ui.label("Pick a filament, then click/drag on a model. The brush paints a round dab of the radius below.");
        ui.horizontal_wrapped(|ui| {
            for (i, f) in self.doc.filaments.iter().enumerate() {
                let sel = self.paint_slot as usize == i && !self.erase;
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                ui.painter().rect_filled(r, 4.0, c32(f.color)); ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Inside);
                if sel {
                    ui.painter().rect_stroke(r.expand(2.0), 4.0, Stroke::new(2.5, crate::theme::accent()), egui::StrokeKind::Outside);
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
            crate::theme::section(ui, "Inspector");
            ui.label("Select an object to edit it.");
            return;
        }
        if sel.len() > 1 {
            crate::theme::section(ui, &format!("{} objects selected", sel.len()));
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
        let mut open_sketch = None;
        let before = format!("{:?}", (&o.name, &o.kind_debug(), o.size, o.pos, o.rot, o.hole, o.color, &o.mirror));
        ui.horizontal(|ui| {
            crate::theme::section(ui, "Inspector");
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
        // colour (kept visible for holes, greyed out, so the layout doesn't jump)
        {
            let hole = o.hole;
            ui.add_enabled_ui(!hole, |ui| {
                ui.horizontal_wrapped(|ui| {
                ui.label("Colour");
                for (i, f) in self.doc.filaments.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                    ui.painter().rect_filled(r, 3.0, c32(f.color)); ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Inside);
                    if o.color as usize == i {
                        ui.painter().rect_stroke(r.expand(1.5), 3.0, Stroke::new(2.0, crate::theme::accent()), egui::StrokeKind::Outside);
                    }
                    if resp.on_hover_text(format!("{} — {}", i + 1, f.name)).clicked() {
                        o.color = i as u8;
                    }
                }
            });
            });
        }
        // shape parameters
        if let Kind::Shape(s) = &mut o.kind {
            ui.label(RichText::new(s.label()).strong());
            let before_params = format!("{s:?}");
            shape_params(ui, s);
            if matches!(s, Shape::Sketch { .. }) && ui.button("✏ Edit sketch…").clicked() {
                open_sketch = crate::sketch::SketchEditor::from_shape(s, o.id);
            }
            if format!("{s:?}") != before_params {
                if let Some(n) = s.natural_size() {
                    o.size = n;
                }
            }
        } else if let Kind::Mesh(m) = &o.kind {
            ui.label(format!("Mesh · {} triangles", m.tris.len()));
        }
        if open_sketch.is_some() {
            self.sketch = open_sketch;
        }
        if !o.hole {
            egui::CollapsingHeader::new("Round / bevel edges").id_salt("edges_hdr").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.edge_chamfer, false, "Round");
                    ui.radio_value(&mut self.edge_chamfer, true, "Bevel");
                    ui.add(egui::DragValue::new(&mut self.edge_radius).range(0.2..=20.0).speed(0.1).suffix(" mm"));
                });
                ui.checkbox(&mut self.edge_skip_bottom, "Keep edges on the plate sharp").on_hover_text("Rounded bottom edges print as overhangs; a bevel there is fine");
                if ui.button("Apply to all sharp outside edges").clicked() {
                    self.edge_apply = Some(o.id);
                }
                ui.label(RichText::new("The part becomes a mesh, so its shape settings no longer apply. Ctrl+Z undoes it.").small().weak());
            });
            let wb = self.world_bounds(o.id);
            egui::CollapsingHeader::new("Add a thread").id_salt("thread_hdr").show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.radio(self.thread_outside, "Screw thread").clicked() {
                        self.thread_outside = true;
                        self.thread_d = None;
                    }
                    if ui.radio(!self.thread_outside, "Threaded hole").clicked() {
                        self.thread_outside = false;
                        self.thread_d = None;
                    }
                });
                let width = (wb.max.x - wb.min.x).min(wb.max.y - wb.min.y);
                let mut d = match self.thread_d {
                    Some((id, d)) if id == o.id => d,
                    _ => if self.thread_outside { width } else { (width * 0.5).round().max(3.0) },
                };
                ui.add(egui::Slider::new(&mut d, 2.0..=width.max(3.0)).text("diameter (mm)"));
                self.thread_d = Some((o.id, d));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.thread_pitch_auto, "Standard pitch");
                    if self.thread_pitch_auto {
                        self.thread_pitch = crate::parts::coarse_pitch(d).max((d / 10.0).min(3.0));
                        ui.label(format!("{:.2} mm", self.thread_pitch));
                    } else {
                        ui.add(egui::DragValue::new(&mut self.thread_pitch).range(0.4..=6.0).speed(0.05).suffix(" mm"));
                    }
                });
                let hmax = (wb.max.z - wb.min.z).max(1.0);
                self.thread_len = self.thread_len.min(hmax);
                ui.add(egui::Slider::new(&mut self.thread_len, 1.0..=hmax).text("length (mm)"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.thread_top, true, "At the top");
                    ui.radio_value(&mut self.thread_top, false, "At the bottom");
                });
                if ui.button("Add thread").clicked() {
                    self.thread_apply = Some(o.id);
                }
                ui.label(RichText::new(if self.thread_outside {
                    "That end of the part becomes a solid screw. Matching hole: same diameter and pitch (the hole gets your Fit test gap)."
                } else {
                    "Cuts a threaded hole down the middle, with a lead-in at the opening."
                }).small().weak());
            });
            egui::CollapsingHeader::new("Split into pieces").id_salt("split_hdr").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Cut across");
                    for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.radio_value(&mut self.split_axis, i, *n);
                    }
                });
                let a = self.split_axis;
                let (lo, hi) = (wb.min[a], wb.max[a]);
                let mut at = match self.split_at {
                    Some((id, ax, v)) if id == o.id && ax == a => v,
                    _ => (lo + hi) / 2.0,
                };
                ui.add(egui::Slider::new(&mut at, (lo + 1.0)..=(hi - 1.0).max(lo + 1.0)).text("cut at (mm)"));
                self.split_at = Some((o.id, a, at));
                self.split_preview = Some((o.id, a, at));
                ui.checkbox(&mut self.split_pins, "Alignment pins (printed separately, glue in)");
                if self.split_pins {
                    ui.add(egui::Slider::new(&mut self.split_pin_d, 2.0..=10.0).text("pin diameter (mm)"));
                }
                if ui.button("Split").clicked() {
                    self.split_apply = Some(o.id);
                }
                ui.label(RichText::new("The orange plane shows the cut. Then use Make it printable to lay the pieces flat.").small().weak());
            });
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
        egui::CollapsingHeader::new("Colour by height (stripes)").default_open(false).show(ui, |ui| {
            ui.label(RichText::new("From each height up (measured from the part's bottom), use that filament.").small().weak());
            let mut remove = None;
            let nfil = self.doc.filaments.len();
            for (i, (h, c)) in self.bands.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label("from");
                    ui.add(egui::DragValue::new(h).range(0.0..=1000.0).speed(0.2).suffix(" mm"));
                    for f in 0..nfil {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                        let col = self.doc.filaments[f].color;
                        ui.painter().rect_filled(r, 2.0, c32(col));
                        ui.painter().rect_stroke(r, 2.0, Stroke::new(if *c as usize == f { 2.0 } else { 1.0 }, if *c as usize == f { crate::theme::accent() } else { Color32::from_gray(120) }), egui::StrokeKind::Outside);
                        if resp.clicked() {
                            *c = f as u8;
                        }
                    }
                    if ui.small_button("✖").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.bands.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.button("+ Add band").clicked() {
                    let last = self.bands.last().map(|b| b.0).unwrap_or(0.0);
                    self.bands.push((last + 5.0, (self.bands.len() % nfil.max(1)) as u8));
                }
                if ui.add_enabled(!self.bands.is_empty() && !o.locked, egui::Button::new(RichText::new("Apply stripes").strong())).clicked() {
                    self.apply_bands();
                }
            });
        });
        self.common_buttons(ui);
    }

    fn color_picker(&mut self, ui: &mut egui::Ui, sel: &[u64]) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Colour all");
            let fl = self.doc.filaments.clone();
            for (i, f) in fl.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                ui.painter().rect_filled(r, 3.0, c32(f.color)); ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Inside);
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
        let n = self.doc.plate_count();
        if n > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label("Move to:");
                let cur = self.doc.plate;
                for p in (0..n).filter(|p| *p != cur) {
                    if ui.small_button(format!("Plate {}", p + 1)).clicked() {
                        self.move_selection_to_plate(p);
                    }
                }
            });
        }
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
        crate::theme::section(ui, "Print check");
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
        // plain-language verdict
        let errors = r.issues.iter().filter(|i| i.severity == Severity::Error).count();
        let warns = r.issues.iter().filter(|i| i.severity == Severity::Warning).count();
        let (title, sub, fg, bg) = if errors > 0 {
            (
                "✖  Not ready to print".to_string(),
                format!("Fix the {errors} red problem{} below first — the print would likely fail or come out wrong.", if errors == 1 { "" } else { "s" }),
                crate::theme::bad_banner().0,
                crate::theme::bad_banner().1,
            )
        } else if warns > 0 {
            (
                "✔  Ready to print".to_string(),
                format!("It will print. {warns} orange note{} below {} worth a look but won't stop the print.", if warns == 1 { "" } else { "s" }, if warns == 1 { "is" } else { "are" }),
                crate::theme::ok_banner().0,
                crate::theme::ok_banner().1,
            )
        } else {
            (
                "✔  Ready to print".to_string(),
                "No problems found.".to_string(),
                crate::theme::ok_banner().0,
                crate::theme::ok_banner().1,
            )
        };
        egui::Frame::new().fill(bg).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(17.0).strong().color(fg));
            ui.label(RichText::new(sub).color(fg));
        });
        ui.label(RichText::new("✖ red = must fix   ⚠ orange = worth knowing   ℹ blue = info only").small().weak());
        ui.add_space(4.0);
        for s in r.summary.iter().filter(|s| !s.starts_with("No problems found")) {
            ui.label(RichText::new(s).small());
        }
        ui.add_space(4.0);
        let mut clicked = None;
        for (i, is) in r.issues.iter().enumerate() {
            let (icon, col) = match is.severity {
                Severity::Error => ("✖", crate::theme::error()),
                Severity::Warning => ("⚠", crate::theme::warn()),
                Severity::Info => ("ℹ", crate::theme::info()),
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
        self.last_view_rect = rect;
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


        // one-sided resize handles (Tinkercad-style)
        let mut handle_active = self.resize.is_some();
        let (p_pressed, p_down, p_pos) = ui.input(|i| (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.button_down(egui::PointerButton::Primary), i.pointer.hover_pos()));
        let handles = self.resize_handles(rect);
        if p_pressed && self.tool == Tool::Select && resp.hovered() {
            if let Some(pp) = p_pos {
                if let Some(h) = handles.iter().find(|h| h.2.distance(pp) < 10.0) {
                    let o = self.doc.find(self.selection[0]).cloned().unwrap();
                    let (ro, rd) = self.camera.ray(rect, pp);
                    let s0 = line_param(h.3, h.4, ro, rd);
                    // the opposite face stays put
                    let anchor = self.face_world(&o, h.0, -h.1);
                    self.resize = Some(ResizeDrag { id: o.id, axis: h.0, sign: h.1, start: o, anchor, origin: h.3, dir: h.4, s0 });
                    handle_active = true;
                }
            }
        }
        if let Some(rd) = self.resize.clone() {
            handle_active = true;
            if !p_down {
                self.resize = None;
            } else if let Some(pp) = p_pos {
                let (ro, rdir) = self.camera.ray(rect, pp);
                let s = line_param(rd.origin, rd.dir, ro, rdir);
                let mut len = rd.start.size[rd.axis] + (s - rd.s0);
                if self.snap > 0.0 && !mods_shift(ui) {
                    len = (len / self.snap).round() * self.snap;
                }
                let len = len.max(0.1);
                let mut o = rd.start.clone();
                o.size[rd.axis] = len;
                o.pos = DVec3::ZERO;
                let at_zero = self.face_world(&o, rd.axis, -rd.sign);
                o.pos = rd.anchor - at_zero;
                if let Some(slot) = self.doc.find_mut(rd.id) {
                    if slot.size != o.size || slot.pos != o.pos {
                        *slot = o;
                        self.touch();
                    }
                }
            }
        }

        // push / pull
        let mut pp_active = false;
        if self.tool == Tool::PushPull {
            if p_pressed && resp.hovered() {
                if let Some(pp) = p_pos {
                    if let Some(h) = self.pick(rect, pp) {
                        let e = &self.entries[h.entry];
                        if !self.doc.find(e.id).map(|o| o.locked).unwrap_or(true) {
                            let region = e.mesh.flat_region(h.tri as usize, 1.0);
                            let (n, _) = e.mesh.tri_normal_area(h.tri as usize);
                            let (ro, rd) = self.camera.ray(rect, pp);
                            let s0 = line_param(h.point, n, ro, rd);
                            self.pp = Some(PushPull { id: e.id, region, n, origin: h.point, dist: 0.0, dragging: true, s0 });
                            self.selection = vec![e.id];
                            self.rebuild_batches();
                        }
                    }
                }
            }
            if let Some(mut pp) = self.pp.clone() {
                if pp.dragging {
                    pp_active = true;
                    if p_down {
                        if let Some(p) = p_pos {
                            let (ro, rd) = self.camera.ray(rect, p);
                            let mut d = line_param(pp.origin, pp.n, ro, rd) - pp.s0;
                            if self.snap > 0.0 && !mods_shift(ui) {
                                d = (d / self.snap).round() * self.snap;
                            }
                            if (d - pp.dist).abs() > 1e-9 {
                                pp.dist = d;
                                self.pp = Some(pp.clone());
                                self.rebuild_batches();
                            }
                        }
                    } else {
                        pp.dragging = false;
                        let d = pp.dist;
                        self.pp = Some(pp);
                        if d.abs() > 1e-6 {
                            self.apply_pushpull(d);
                        }
                    }
                }
            }
        }

        // gizmo
        let mut gizmo_active = handle_active || pp_active;
        if !handle_active && !pp_active && self.tool == Tool::Select && self.gizmo_kind != GizmoKind::Off && !self.selection.is_empty() && !self.selected().iter().any(|o| o.locked) {
            let b = self.selection_bounds();
            let single = if self.selection.len() == 1 { self.doc.find(self.selection[0]).cloned() } else { None };
            let rot = single.as_ref().map(|o| o.rotation()).unwrap_or(DQuat::IDENTITY);
            // No centre handles: they sit on top of the object and would steal a normal drag
            // (the view-plane move handle also changes height, which looks like sinking).
            let modes = match self.gizmo_kind {
                GizmoKind::Move => GizmoMode::TranslateX | GizmoMode::TranslateY | GizmoMode::TranslateZ | GizmoMode::TranslateXY,
                GizmoKind::Rotate => GizmoMode::RotateX | GizmoMode::RotateY | GizmoMode::RotateZ,
                _ => GizmoMode::ScaleX | GizmoMode::ScaleY | GizmoMode::ScaleZ | GizmoMode::ScaleXY,
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
                    let ids = self.selection.clone();
                    let others = self.entries.iter().filter(|e| !ids.contains(&e.id) && !e.hole).map(|e| e.mesh.bounds()).collect();
                    self.gizmo_ctx = (b, others);
                }
                let start = self.gizmo_start.clone().unwrap();
                let pivot = b.center();
                match result {
                    GizmoResult::Translation { total, .. } => {
                        let mut d = DVec3::from(total);
                        if !shift {
                            let moving = [d.x.abs() > 1e-9, d.y.abs() > 1e-9, d.z.abs() > 1e-9];
                            self.grid_snap_abs(start[0].1, &mut d, moving);
                            self.guides.clear();
                            let (gsb, gothers) = self.gizmo_ctx.clone();
                            self.snap_to_neighbours(&gsb, &gothers, &mut d, [moving[0], moving[1]]);
                        }
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
            if self.gizmo_start.is_some() {
                self.guides.clear();
            }
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
                            self.begin_move(h.point)
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
            Drag::Move { plane_z, start_hit, start, sb, others, riding, samples } => {
                let (plane_z, start_hit, start, sb, others, riding, samples) =
                    (*plane_z, *start_hit, start.clone(), *sb, others.clone(), *riding, samples.clone());
                if let Some(p) = pointer {
                    let (o, d) = self.camera.ray(rect, p);
                    let ids_now: Vec<u64> = start.iter().map(|x| x.0).collect();
                    // follow the surface under the cursor (Tinkercad-style); else the drag plane
                    let surf = if mods.shift { None } else { self.pick_excluding(o, d, &ids_now) };
                    if d.z.abs() > 1e-6 || surf.is_some() {
                        let t = if d.z.abs() > 1e-6 { (plane_z - o.z) / d.z } else { -1.0 };
                        if t > 0.0 || surf.is_some() {
                            let h = surf.unwrap_or(o + d * t);
                            let mut dd = h - start_hit;
                            dd.z = 0.0;
                            let free = mods.shift;
                            if !free {
                                self.grid_snap_abs(start[0].1, &mut dd, [true, true, false]);
                            }
                            self.guides.clear();
                            if !free {
                                self.snap_to_neighbours(&sb, &others, &mut dd, [true, true]);
                            }
                            let ids: Vec<u64> = start.iter().map(|x| x.0).collect();
                            if riding && !free {
                                // sit on whatever is under the footprint (or the plate)
                                let mut floor: f64 = 0.0;
                                for (x, y, off) in &samples {
                                    let top = self.surface_below(x + dd.x, y + dd.y, &ids);
                                    floor = floor.max(top - off);
                                }
                                dd.z = floor - sb.min.z;
                            }
                            for (id, sp) in start {
                                if let Some(ob) = self.doc.find_mut(id) {
                                    ob.pos = sp + dd;
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
            self.guides.clear();
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
                Tool::SnapFaces => {
                    if let Some(h) = hit {
                        self.snap_face_click(&h);
                    }
                }
                Tool::PushPull => {}
                Tool::Decal => {
                    if let Some(h) = hit {
                        self.apply_decal(&h);
                    }
                }
                Tool::SurfaceText => {
                    if let Some(h) = hit {
                        self.apply_stext(&h);
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
            r.clear = crate::theme::clear_color();
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
                        painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(40, 90, 190, 225));
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
        if let Some((id, a, at)) = self.split_preview {
            if self.doc.find(id).is_some() {
                let b = self.world_bounds(id);
                let (lo, hi) = (b.min - DVec3::splat(3.0), b.max + DVec3::splat(3.0));
                let (u, v) = match a {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let mut cs = vec![];
                for (pu, pv) in [(lo[u], lo[v]), (hi[u], lo[v]), (hi[u], hi[v]), (lo[u], hi[v])] {
                    let mut p = DVec3::ZERO;
                    p[a] = at;
                    p[u] = pu;
                    p[v] = pv;
                    cs.push(p);
                }
                let pts: Vec<Pos2> = cs.iter().filter_map(|c| self.camera.project(rect, *c)).collect();
                if pts.len() == 4 {
                    painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(255, 150, 40, 50), Stroke::new(2.0, Color32::from_rgb(255, 150, 40))));
                }
            }
        }
        if self.tool == Tool::SurfaceText {
            if let Some((_, wf)) = self.hover.as_ref().and_then(|h| self.stext_frame(h)) {
                let cs = self.stext.corners(&wf);
                let pts: Vec<Pos2> = cs.iter().filter_map(|c| self.camera.project(rect, *c)).collect();
                if pts.len() == 4 {
                    let col = self.doc.filaments.get(self.stext.color as usize).map(|f| f.color).unwrap_or([255, 255, 255]);
                    painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 70), Stroke::new(2.0, crate::theme::accent())));
                    let c = Pos2::new(pts.iter().map(|p| p.x).sum::<f32>() / 4.0, pts.iter().map(|p| p.y).sum::<f32>() / 4.0);
                    painter.text(c, egui::Align2::CENTER_CENTER, &self.stext.text, egui::FontId::proportional(((pts[0].distance(pts[3])) * 0.8).clamp(8.0, 60.0)), Color32::WHITE);
                }
            }
        }
        if self.tool == Tool::Decal {
            if let Some((_, _, wf)) = self.hover.as_ref().and_then(|h| self.decal_frame(h)) {
                if let Some(d) = &self.decal {
                    let cs = d.corners(&wf);
                    let pts: Vec<Pos2> = cs.iter().filter_map(|c| self.camera.project(rect, *c)).collect();
                    if pts.len() == 4 {
                        if let Some(t) = &self.decal_tex {
                            let mut mesh = egui::Mesh::with_texture(t.id());
                            let tint = Color32::from_white_alpha(190);
                            for (p, uv) in pts.iter().zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]) {
                                mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: Pos2::new(uv.0, uv.1), color: tint });
                            }
                            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                            painter.add(egui::Shape::mesh(mesh));
                        }
                        let mut ring = pts.clone();
                        ring.push(pts[0]);
                        painter.add(egui::Shape::line(ring, Stroke::new(2.0, crate::theme::accent())));
                    }
                }
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
        if self.tool == Tool::Select {
            let hp = ui.input(|i| i.pointer.hover_pos());
            for h in self.resize_handles(rect) {
                let hot = hp.map(|p| p.distance(h.2) < 10.0).unwrap_or(false) || self.resize.as_ref().map(|r| r.axis == h.0 && r.sign == h.1).unwrap_or(false);
                let r = Rect::from_center_size(h.2, Vec2::splat(if hot { 12.0 } else { 9.0 }));
                painter.rect_filled(r, 2.0, if hot { crate::theme::accent() } else { Color32::WHITE });
                painter.rect_stroke(r, 2.0, Stroke::new(1.5, crate::theme::accent()), egui::StrokeKind::Middle);
            }
            if let Some(rd) = &self.resize {
                if let (Some(o), Some(p)) = (self.doc.find(rd.id), ui.input(|i| i.pointer.hover_pos())) {
                    painter.text(p + Vec2::new(14.0, -14.0), egui::Align2::LEFT_BOTTOM, format!("{} {:.2} mm", ["W", "D", "H"][rd.axis], o.size[rd.axis]), egui::FontId::proportional(15.0), crate::theme::accent());
                }
            }
        }
        if let (Tool::PushPull, Some(pp)) = (self.tool, &self.pp) {
            if let Some(q) = self.camera.project(rect, pp.origin + pp.n * pp.dist) {
                let q0 = self.camera.project(rect, pp.origin).unwrap_or(q);
                painter.line_segment([q0, q], Stroke::new(2.0, Color32::from_rgb(230, 40, 160)));
                painter.text(q + Vec2::new(12.0, -12.0), egui::Align2::LEFT_BOTTOM, format!("{:+.2} mm", pp.dist), egui::FontId::proportional(15.0), Color32::from_rgb(160, 20, 110));
            }
        }
        for (a, b) in &self.guides {
            if let (Some(pa), Some(pb)) = (self.camera.project(rect, *a), self.camera.project(rect, *b)) {
                painter.line_segment([pa, pb], Stroke::new(1.5, Color32::from_rgb(230, 40, 160)));
            }
        }
        if let Some((_, p, _)) = self.snap_from {
            if let Some(q) = self.camera.project(rect, p) {
                painter.circle_filled(q, 6.0, Color32::from_rgb(230, 40, 160));
                painter.circle_stroke(q, 8.0, Stroke::new(1.5, Color32::WHITE));
            }
        }
        // drop new shapes dragged in from the Shapes panel
        if let Some(idx) = resp.dnd_release_payload::<usize>() {
            if let Some(p) = pointer {
                self.add_shape_at(*idx, rect, p);
            }
        }
        // view cube-ish buttons
        self.plate_tabs(ui, rect);
        let mut go_home = false;
        let area = Rect::from_min_size(rect.right_top() + Vec2::new(-190.0, 8.0), Vec2::new(182.0, 26.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                for (n, y, p) in [("Home", -60.0, 32.0), ("Top", -90.0, 89.99), ("Front", -90.0, 0.0), ("Right", 0.0, 0.0)] {
                    if ui.small_button(n).clicked() {
                        self.camera.yaw = y;
                        self.camera.pitch = p;
                        if n == "Home" {
                            go_home = true;
                        }
                    }
                }
            });
        });
        if go_home && !self.doc.objects.is_empty() {
            self.fit_view(false);
        }
        if self.doc.objects.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Click a shape on the left to begin", egui::FontId::proportional(20.0), crate::theme::hint());
        }
    }

    /// World position of the centre of an object's local box face (axis, ±1).
    fn face_world(&self, o: &Object, axis: usize, sign: f64) -> DVec3 {
        let mut p = DVec3::new(0.0, 0.0, o.size.z * 0.5);
        p[axis] += sign * o.size[axis] * 0.5;
        o.transform().transform_point3(p)
    }

    /// Side handles of the single selected object: (axis, sign, screen pos, world pos, world dir).
    fn resize_handles(&self, rect: Rect) -> Vec<(usize, f64, Pos2, DVec3, DVec3)> {
        if self.selection.len() != 1 || self.gizmo_kind == GizmoKind::Rotate {
            return vec![];
        }
        let Some(o) = self.doc.find(self.selection[0]) else { return vec![] };
        if o.locked {
            return vec![];
        }
        let mut out = vec![];
        for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0), (2, 1.0)] {
            let w = self.face_world(o, axis, sign);
            let mut step = DVec3::new(0.0, 0.0, o.size.z * 0.5);
            step[axis] += sign * (o.size[axis] * 0.5 + 1.0);
            let dir = (o.transform().transform_point3(step) - w).normalize_or_zero();
            if let Some(sp) = self.camera.project(rect, w) {
                out.push((axis, sign, sp, w, dir));
            }
        }
        out
    }

    fn apply_pushpull(&mut self, dist: f64) {
        let Some(pp) = self.pp.clone() else { return };
        let Some(e) = self.entries.iter().find(|e| e.id == pp.id) else { return };
        let Some(o) = self.doc.find(pp.id).cloned() else { return };
        let mesh = e.mesh.clone();
        let color = {
            let mut c = std::collections::HashMap::new();
            for &t in &pp.region {
                *c.entry(mesh.colors[t as usize]).or_insert(0usize) += 1;
            }
            c.into_iter().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(o.color)
        };
        let eps = 0.01;
        let r = if dist > 0.0 {
            let prism = mesh.region_prism(&pp.region, pp.n, -eps, dist, color);
            crate::csg::combine(&[mesh, prism], &[])
        } else {
            let prism = mesh.region_prism(&pp.region, pp.n, dist, eps, color);
            crate::csg::combine(&[mesh], &[prism])
        };
        if !r.warnings.is_empty() || r.mesh.tris.is_empty() {
            self.status = format!("Push/Pull couldn't be applied: {}", r.warnings.join("; "));
            return;
        }
        let (nm, size, off) = normalize_mesh(r.mesh);
        if let Some(slot) = self.doc.find_mut(pp.id) {
            slot.kind = Kind::Mesh(Arc::new(nm));
            slot.size = size;
            slot.pos = off;
            slot.rot = DVec3::ZERO;
            slot.mirror = [false; 3];
            slot.paint = None;
        }
        self.pp = None;
        self.status = format!("{} the face by {:.2} mm.", if dist > 0.0 { "Extruded" } else { "Pushed in" }, dist.abs());
        self.touch();
    }

    fn begin_move(&mut self, hit: DVec3) -> Drag {
        self.refresh_world();
        let ids: Vec<u64> = self.selection.clone();
        let start: Vec<(u64, DVec3)> = self.selected().iter().map(|o| (o.id, o.pos)).collect();
        let sb = self.selection_bounds();
        let others: Vec<Aabb> = self.entries.iter().filter(|e| !ids.contains(&e.id)).map(|e| e.mesh.bounds()).collect();
        // sample the selection's underside so it can rest on other parts' tops
        let mut samples = Vec::new();
        let n = 7;
        for i in 0..n {
            for j in 0..n {
                let x = sb.min.x + (sb.max.x - sb.min.x) * (i as f64 + 0.5) / n as f64;
                let y = sb.min.y + (sb.max.y - sb.min.y) * (j as f64 + 0.5) / n as f64;
                if let Some(z) = self.underside(x, y, &ids) {
                    samples.push((x, y, z - sb.min.z));
                }
            }
        }
        for c in [sb.min, DVec3::new(sb.max.x, sb.min.y, 0.0), DVec3::new(sb.min.x, sb.max.y, 0.0), sb.max] {
            let (x, y) = (c.x * 0.98 + sb.center().x * 0.02, c.y * 0.98 + sb.center().y * 0.02);
            if let Some(z) = self.underside(x, y, &ids) {
                samples.push((x, y, z - sb.min.z));
            }
        }
        // "riding" only if it currently rests on the plate or on another part
        let mut floor: f64 = 0.0;
        for (x, y, off) in &samples {
            floor = floor.max(self.surface_below(*x, *y, &ids) - off);
        }
        let riding = (sb.min.z - floor).abs() < 0.05 || sb.min.z.abs() < 0.05;
        Drag::Move { plane_z: hit.z, start_hit: hit, start, sb, others, riding, samples }
    }

    /// Closest hit of a ray against solid objects not in `exclude`.
    fn pick_excluding(&mut self, o: DVec3, d: DVec3, exclude: &[u64]) -> Option<DVec3> {
        let mut best: Option<(f64, f64)> = None;
        for e in self.entries.iter_mut() {
            if e.hole || exclude.contains(&e.id) {
                continue;
            }
            let bvh = e.bvh.get_or_insert_with(|| Arc::new(Bvh::build(&e.mesh))).clone();
            if let Some((t, tri)) = bvh.raycast(&e.mesh, o, d, None) {
                if best.map_or(true, |b| t < b.0) {
                    best = Some((t, e.mesh.tri_normal_area(tri as usize).0.z));
                }
            }
        }
        // only follow upward-facing surfaces (tops); over a side wall keep the current height
        best.filter(|b| b.1 > 0.5).map(|b| o + d * b.0)
    }

    /// Height of the highest upward-facing surface under (x, y), ignoring `exclude`. 0 = plate.
    fn surface_below(&mut self, x: f64, y: f64, exclude: &[u64]) -> f64 {
        let o = DVec3::new(x, y, 1.0e5);
        let d = DVec3::NEG_Z;
        let mut best: f64 = 0.0;
        for e in self.entries.iter_mut() {
            if e.hole || exclude.contains(&e.id) {
                continue;
            }
            let b = e.mesh.bounds();
            if x < b.min.x || x > b.max.x || y < b.min.y || y > b.max.y {
                continue;
            }
            let bvh = e.bvh.get_or_insert_with(|| Arc::new(Bvh::build(&e.mesh))).clone();
            if let Some((t, _)) = bvh.raycast(&e.mesh, o, d, None) {
                best = best.max(o.z - t);
            }
        }
        best
    }

    /// Lowest point of the selection directly above/below (x, y).
    fn underside(&mut self, x: f64, y: f64, ids: &[u64]) -> Option<f64> {
        let o = DVec3::new(x, y, -1.0e5);
        let mut best: Option<f64> = None;
        for e in self.entries.iter_mut() {
            if !ids.contains(&e.id) || e.hole {
                continue;
            }
            let bvh = e.bvh.get_or_insert_with(|| Arc::new(Bvh::build(&e.mesh))).clone();
            if let Some((t, _)) = bvh.raycast(&e.mesh, o, DVec3::Z, None) {
                let z = o.z + t;
                best = Some(best.map_or(z, |b: f64| b.min(z)));
            }
        }
        best
    }

    /// Snap the moving selection's edges/centres to nearby objects' edges/centres.
    /// Snap the object's absolute position (not the distance moved) to the grid.
    fn grid_snap_abs(&self, start_pos: DVec3, dd: &mut DVec3, axes: [bool; 3]) {
        if self.snap <= 0.0 {
            return;
        }
        for a in 0..3 {
            if axes[a] {
                let target = ((start_pos[a] + dd[a]) / self.snap).round() * self.snap;
                dd[a] = target - start_pos[a];
            }
        }
    }

    fn snap_to_neighbours(&mut self, sb: &Aabb, others: &[Aabb], dd: &mut DVec3, axes: [bool; 2]) {
        // ~12 screen pixels, but never less than a grid step so the grid can't jump past it
        let c = sb.center() + *dd;
        let right = self.camera.view().inverse().transform_vector3(DVec3::X);
        let rect = self.last_view_rect;
        let px_per_mm = match (self.camera.project(rect, c), self.camera.project(rect, c + right)) {
            (Some(a), Some(b)) => (a.distance(b) as f64).max(1e-3),
            _ => 1.0,
        };
        let thr = (12.0 / px_per_mm).max(self.snap * 0.75).clamp(0.3, 20.0);
        for axis in 0..2 {
            if !axes[axis] {
                continue;
            }
            let moved_min = sb.min[axis] + dd[axis];
            let moved_max = sb.max[axis] + dd[axis];
            let mine = [moved_min, (moved_min + moved_max) * 0.5, moved_max];
            let mut best: Option<(f64, f64, &Aabb)> = None; // (correction, target value, other)
            for ob in others {
                // only consider neighbours that are reasonably close on the other axis
                let oa = 1 - axis;
                let gap = (ob.min[oa] - (sb.max[oa] + dd[oa])).max((sb.min[oa] + dd[oa]) - ob.max[oa]).max(0.0);
                if gap > 60.0 {
                    continue;
                }
                for t in [ob.min[axis], (ob.min[axis] + ob.max[axis]) * 0.5, ob.max[axis]] {
                    for m in mine {
                        let c = t - m;
                        if c.abs() < thr && best.map_or(true, |b| c.abs() < b.0.abs()) {
                            best = Some((c, t, ob));
                        }
                    }
                }
            }
            if let Some((c, t, ob)) = best {
                dd[axis] += c;
                let oa = 1 - axis;
                let lo = ob.min[oa].min(sb.min[oa] + dd[oa]) - 5.0;
                let hi = ob.max[oa].max(sb.max[oa] + dd[oa]) + 5.0;
                let z = 0.05;
                let (a, b) = if axis == 0 {
                    (DVec3::new(t, lo, z), DVec3::new(t, hi, z))
                } else {
                    (DVec3::new(lo, t, z), DVec3::new(hi, t, z))
                };
                self.guides.push((a, b));
            }
        }
    }

    fn snap_face_click(&mut self, h: &Hit) {
        let e = &self.entries[h.entry];
        let (id, (n, _)) = (e.id, e.mesh.tri_normal_area(h.tri as usize));
        match self.snap_from {
            None => {
                if self.doc.find(id).map(|o| o.locked).unwrap_or(true) {
                    self.status = "That object is locked.".into();
                    return;
                }
                self.snap_from = Some((id, h.point, n));
                self.selection = vec![id];
                self.geometry_dirty = true;
                self.status = "Now click the face to attach it to.".into();
            }
            Some((src, pa, na)) => {
                if src == id {
                    self.status = "Pick a face on a different object.".into();
                    return;
                }
                let Some(o) = self.doc.find(src).cloned() else { return };
                let q = DQuat::from_rotation_arc(na, -n);
                let local = o.transform().inverse().transform_point3(pa);
                let mut moved = o.clone();
                moved.set_rotation(q * o.rotation());
                // place so the clicked point lands on the target point
                let now = moved.transform().transform_point3(local);
                moved.pos += h.point - now;
                if let Some(slot) = self.doc.find_mut(src) {
                    *slot = moved;
                }
                self.snap_from = None;
                self.selection = vec![src, id];
                self.touch();
                self.status = "Snapped face to face. Press Group (Ctrl/Cmd+G) to join them.".into();
            }
        }
    }

    fn add_shape_at(&mut self, idx: usize, rect: Rect, p: Pos2) {
        let hit = self.pick(rect, p);
        let target = match &hit {
            Some(h) if !self.entries[h.entry].hole => Some(h.point),
            _ => {
                let (o, d) = self.camera.ray(rect, p);
                if d.z.abs() > 1e-6 && (-o.z / d.z) > 0.0 { Some(o + d * (-o.z / d.z)) } else { None }
            }
        };
        self.add_shape(idx);
        let Some(t) = target else { return };
        let Some(&id) = self.selection.first() else { return };
        let size = self.doc.find(id).map(|o| o.size).unwrap_or(DVec3::ONE);
        let (mut x, mut y) = (t.x, t.y);
        if self.snap > 0.0 {
            x = (x / self.snap).round() * self.snap;
            y = (y / self.snap).round() * self.snap;
        }
        let mut floor: f64 = 0.0;
        for (dx, dy) in [(0.0, 0.0), (-0.45, -0.45), (0.45, -0.45), (-0.45, 0.45), (0.45, 0.45)] {
            floor = floor.max(self.surface_below(x + dx * size.x, y + dy * size.y, &[id]));
        }
        if let Some(o) = self.doc.find_mut(id) {
            o.pos = DVec3::new(x, y, floor);
        }
        self.touch();
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
        let mut print_now = false;
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
            if i.consume_key(egui::Modifiers::COMMAND, Key::P) {
                print_now = true;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::X) {
                self.xray = !self.xray;
                self.geometry_dirty = true;
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
        if print_now {
            self.request_print();
        }
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
                ui.label("Side handles: drag a white square on a selected part to stretch just that side. Push/Pull tool: click a flat face and drag it out or in.");
                ui.label("Dragged parts ride on top of other parts and snap to their edges and centres (pink guides). Snap faces tool: click a face to move, then the face to attach to. Drag shapes from the panel onto the plate or a part. Hold Shift while dragging for free movement with no snapping.");
            });
            self.show_help = open;
        }
        if self.recover_offer {
            egui::Window::new("Recover unsaved design?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label("RoboForge closed last time without saving. An automatic backup of that design was kept.");
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Recover it").strong()).clicked() {
                        if let Some(p) = crate::settings::autosave_path() {
                            if let Ok(d) = io::load_project(&p) {
                                self.doc = d;
                                self.committed = self.doc.clone();
                                self.edits = 1;
                                self.saved_edits = 0;
                                self.geometry_dirty = true;
                                self.status = "Recovered your unsaved design — remember to Save it.".into();
                            }
                        }
                        self.recover_offer = false;
                    }
                    if ui.button("Discard").clicked() {
                        self.clear_autosave();
                        self.recover_offer = false;
                    }
                });
            });
        }
        if let Some(lines) = self.printable_report.clone() {
            let mut open = true;
            egui::Window::new("Make it printable").open(&mut open).resizable(false).show(ctx, |ui| {
                for l in &lines {
                    ui.label(l);
                }
            });
            if !open {
                self.printable_report = None;
            }
        }
        if let Some(action) = self.confirm_discard.clone() {
            let what = match action {
                PendingAction::Close => "closing",
                PendingAction::New => "starting a new design",
                PendingAction::Open | PendingAction::OpenPath(_) => "opening another project",
            };
            egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(RichText::new("Your design has changes that haven't been saved.").strong());
                ui.label(format!("Save before {what}? Exported 3MF/STL files don't count as saving the project."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save").strong()).clicked() {
                        self.confirm_discard = None;
                        if self.save(false) {
                            self.run_pending(action.clone(), ctx);
                        }
                    }
                    if ui.button("Don't save").clicked() {
                        self.confirm_discard = None;
                        self.saved_edits = self.edits;
                        self.run_pending(action.clone(), ctx);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_discard = None;
                    }
                });
            });
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
        Shape::Container { wall, floor, radius } => {
            ui.add(egui::Slider::new(wall, 0.8..=10.0).text("wall (mm)"));
            ui.add(egui::Slider::new(floor, 0.6..=10.0).text("floor (mm)"));
            ui.add(egui::Slider::new(radius, 0.0..=20.0).text("corner radius (mm)"));
            ui.label(RichText::new("Size is the outside of the box. Pair it with a Lid of the same W and D.").small().weak());
        }
        Shape::Lid { wall, lip, clearance, radius } => {
            ui.add(egui::Slider::new(wall, 0.8..=10.0).text("box wall (mm)"));
            ui.add(egui::Slider::new(lip, 0.0..=15.0).text("lip depth (mm)"));
            ui.add(egui::Slider::new(clearance, 0.0..=1.0).text("fit clearance (mm)"));
            ui.add(egui::Slider::new(radius, 0.0..=20.0).text("corner radius (mm)"));
            ui.label(RichText::new("Prints face-down; flip it onto the box. 0.2–0.3 mm clearance = snug fit.").small().weak());
        }
        Shape::Bracket { thickness, hole } => {
            ui.add(egui::Slider::new(thickness, 1.0..=10.0).text("thickness (mm)"));
            ui.add(egui::Slider::new(hole, 0.0..=10.0).text("screw hole (mm)"));
        }
        Shape::Gear { teeth, bore } => {
            sides(ui, teeth, 6, 120, "teeth");
            ui.add(egui::Slider::new(bore, 0.0..=40.0).text("bore (mm)"));
            ui.label(RichText::new("Gears mesh when they share the same tooth size: keep diameter / (teeth + 2) equal.").small().weak());
        }
        Shape::Knob { grips, bore } => {
            sides(ui, grips, 3, 40, "grips");
            ui.add(egui::Slider::new(bore, 0.0..=20.0).text("D-shaft hole (mm)"));
        }
        Shape::CableClip { cable } => {
            ui.add(egui::Slider::new(cable, 1.0..=30.0).text("cable diameter (mm)"));
        }
        Shape::Bolt { diameter, length } => {
            ui.add(egui::Slider::new(diameter, 3.0..=20.0).text("M size (mm)"));
            ui.add(egui::Slider::new(length, 4.0..=120.0).text("thread length (mm)"));
            ui.label(RichText::new(format!("Coarse pitch {:.2} mm. Prints head-down.", crate::parts::coarse_pitch(*diameter))).small().weak());
        }
        Shape::Nut { diameter, clearance } => {
            ui.add(egui::Slider::new(diameter, 3.0..=20.0).text("M size (mm)"));
            ui.add(egui::Slider::new(clearance, 0.0..=0.8).text("thread clearance (mm)"));
            ui.label(RichText::new("Matches a printed Bolt of the same size. 0.2–0.3 mm clearance for PLA.").small().weak());
        }
        Shape::Hinge { knuckles, clearance } => {
            sides(ui, knuckles, 3, 15, "knuckles");
            ui.add(egui::Slider::new(clearance, 0.15..=1.0).text("gap (mm)"));
            ui.label(RichText::new("Prints already assembled — print flat, then snap it loose.").small().weak());
        }
        Shape::SnapHook { arm, catch } => {
            ui.add(egui::Slider::new(arm, 0.6..=4.0).text("arm thickness (mm)"));
            ui.add(egui::Slider::new(catch, 0.3..=4.0).text("catch depth (mm)"));
        }
        Shape::Washer { hole } | Shape::Standoff { hole } => {
            ui.add(egui::Slider::new(hole, 0.5..=60.0).text("hole (mm)"));
        }
        Shape::Pulley { bore, groove } => {
            ui.add(egui::Slider::new(bore, 1.0..=40.0).text("bore (mm)"));
            ui.add(egui::Slider::new(groove, 0.5..=15.0).text("groove depth (mm)"));
        }
        Shape::Wheel { bore, spokes } => {
            ui.add(egui::Slider::new(bore, 1.0..=40.0).text("bore (mm)"));
            sides(ui, spokes, 3, 16, "spokes");
        }
        Shape::Spring { coils, wire } => {
            ui.add(egui::Slider::new(coils, 1.0..=30.0).text("coils"));
            ui.add(egui::Slider::new(wire, 0.8..=8.0).text("wire thickness (mm)"));
            ui.label(RichText::new("Print standing up. PETG or TPU springs back best.").small().weak());
        }
        Shape::Rack { module } => {
            ui.add(egui::Slider::new(module, 0.5..=5.0).text("tooth size (module)"));
            ui.label(RichText::new("Meshes with a Gear whose diameter / (teeth + 2) equals this number.").small().weak());
        }
        Shape::Dovetail { clearance } => {
            ui.add(egui::Slider::new(clearance, 0.05..=1.0).text("fit clearance (mm)"));
            ui.label(RichText::new("Two pieces that slide together.").small().weak());
        }
        Shape::LivingHinge { film, gap } => {
            ui.add(egui::Slider::new(film, 0.3..=1.5).text("hinge film (mm)"));
            ui.add(egui::Slider::new(gap, 1.0..=20.0).text("gap (mm)"));
            ui.label(RichText::new("Flexes best in PETG or PP.").small().weak());
        }
        Shape::WallHook { hole } | Shape::CornerBrace { hole } | Shape::Handle { hole } => {
            ui.add(egui::Slider::new(hole, 0.0..=10.0).text("screw hole (mm)"));
        }
        Shape::PipeClamp { pipe } => {
            ui.add(egui::Slider::new(pipe, 3.0..=80.0).text("pipe diameter (mm)"));
        }
        Shape::CableComb { slots, cable } => {
            sides(ui, slots, 1, 30, "slots");
            ui.add(egui::Slider::new(cable, 1.0..=20.0).text("cable diameter (mm)"));
        }
        Shape::Grommet { hole } => {
            ui.add(egui::Slider::new(hole, 5.0..=120.0).text("hole (mm)"));
        }
        Shape::PhoneStand { angle } => {
            ui.add(egui::Slider::new(angle, 45.0..=85.0).text("lean angle (°)"));
        }
        Shape::DividedBin { cols, rows, wall } => {
            sides(ui, cols, 1, 12, "columns");
            sides(ui, rows, 1, 12, "rows");
            ui.add(egui::Slider::new(wall, 0.8..=5.0).text("wall (mm)"));
        }
        Shape::Planter { wall, hole } => {
            ui.add(egui::Slider::new(wall, 1.0..=6.0).text("wall (mm)"));
            ui.add(egui::Slider::new(hole, 0.0..=30.0).text("drain hole (mm)"));
        }
        Shape::Funnel { spout } => {
            ui.add(egui::Slider::new(spout, 3.0..=40.0).text("spout (mm)"));
        }
        Shape::KeyTag { hole } => {
            ui.add(egui::Slider::new(hole, 1.0..=10.0).text("ring hole (mm)"));
            ui.label(RichText::new("Add Text on top and Group for a name tag.").small().weak());
        }
        Shape::Jar { wall } => {
            ui.add(egui::Slider::new(wall, 1.2..=4.0).text("wall (mm)"));
            ui.label(RichText::new("Pair with a Jar cap that is 5 mm wider than the jar.").small().weak());
        }
        Shape::JarCap { clearance } => {
            ui.add(egui::Slider::new(clearance, 0.1..=1.0).text("thread clearance (mm)"));
        }
        Shape::PegHook | Shape::KeyholePlate | Shape::Saucer => {}
        Shape::GridBin { gx, gy, hu, div_x, div_y, magnets, lip } => {
            ui.label(RichText::new("Size is set in Gridfinity units: 42 mm squares, 7 mm height steps.").small().weak());
            sides(ui, gx, 1, 7, "wide (units)");
            sides(ui, gy, 1, 7, "deep (units)");
            sides(ui, hu, 2, 20, "tall (×7 mm)");
            sides(ui, div_x, 1, 12, "compartments across");
            sides(ui, div_y, 1, 12, "compartments front-to-back");
            ui.checkbox(magnets, "Magnet holes (6×2 mm)");
            ui.checkbox(lip, "Stacking lip");
        }
        Shape::GridPlate { gx, gy } => {
            ui.label(RichText::new("Bins snap into the pockets. Glue or screw it into a drawer.").small().weak());
            sides(ui, gx, 1, 7, "wide (units)");
            sides(ui, gy, 1, 7, "deep (units)");
        }
        Shape::QrCode { wifi, text, ssid, password, open, raise, nfc, dark } => {
            ui.horizontal(|ui| {
                ui.radio_value(wifi, true, "Wi-Fi login");
                ui.radio_value(wifi, false, "Link or text");
            });
            if *wifi {
                ui.horizontal(|ui| {
                    ui.label("Network");
                    ui.text_edit_singleline(ssid);
                });
                ui.checkbox(open, "Open network (no password)");
                if !*open {
                    ui.horizontal(|ui| {
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(password).password(true));
                    });
                }
                ui.label(RichText::new("Guests point their phone camera at it to join. The password is printed into the code, so anyone who scans it can read it.").small().weak());
            } else {
                ui.add(egui::TextEdit::multiline(text).desired_rows(2).hint_text("https://… or any text"));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Code colour");
                for c in 0..8u32 {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                    ui.painter().rect_filled(r, 3.0, c32(crate::scene::default_filaments().get(c as usize).map(|f| f.color).unwrap_or([128, 128, 128])));
                    if *dark == c {
                        ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, crate::theme::accent()), egui::StrokeKind::Outside);
                    }
                    if resp.clicked() {
                        *dark = c;
                    }
                }
            });
            ui.add(egui::Slider::new(raise, 0.4..=3.0).text("raised (mm)"));
            ui.checkbox(nfc, "Pocket underneath for a 25 mm NFC sticker");
            let payload = if *wifi { crate::parts::wifi_payload(ssid, password, *open) } else { text.clone() };
            let cells = crate::parts::qr_modules(&payload);
            if cells > 0 {
                ui.label(RichText::new(format!("{cells}×{cells} squares — make the plaque at least {} mm wide so each square is 1 mm or more.", cells)).small().weak());
            }
        }
        Shape::Enclosure { board, custom_w, custom_d, extra_h, vents, wall_mount, gap } => {
            let boards = crate::parts::BOARDS;
            egui::ComboBox::from_id_salt("enc_board").selected_text(boards[(*board as usize).min(boards.len() - 1)].name).show_ui(ui, |ui| {
                for (i, b) in boards.iter().enumerate() {
                    ui.selectable_value(board, i as u32, b.name);
                }
            });
            let b = &boards[(*board as usize).min(boards.len() - 1)];
            if b.name == "Custom board" {
                ui.add(egui::Slider::new(custom_w, 10.0..=250.0).text("board width (mm)"));
                ui.add(egui::Slider::new(custom_d, 10.0..=250.0).text("board depth (mm)"));
                ui.label(RichText::new("Screw posts go 3.5 mm in from each corner. Cut extra openings with Hole shapes.").small().weak());
            } else {
                let sides = ["front", "right", "back", "left"];
                let list: Vec<String> = b.ports.iter().map(|p| sides[p.0 as usize].to_string()).collect();
                ui.label(RichText::new(format!("{} openings ({}), {} screw posts. Check them against your board before printing — clones can differ.", b.ports.len(), list.join(", "), b.holes.len())).small().weak());
            }
            ui.add(egui::Slider::new(extra_h, 0.0..=60.0).text("extra height (mm)")).on_hover_text("Room for a HAT, fan or wires");
            ui.checkbox(vents, "Vent slots in the lid");
            ui.checkbox(wall_mount, "Wall-mount tabs");
            ui.add(egui::Slider::new(gap, 0.05..=0.8).text("lid fit gap (mm)"));
        }
        Shape::FlexiChain { segments, width, gap, snake } => {
            ui.label(RichText::new("Prints in one piece and comes off the plate already moving. Flex it gently the first time.").small().weak());
            ui.checkbox(snake, "Snake (head and tail)");
            sides(ui, segments, 2, 60, "segments");
            ui.add(egui::Slider::new(width, 6.0..=40.0).text("width (mm)"));
            ui.add(egui::Slider::new(gap, 0.15..=0.8).text("joint gap (mm)"));
        }
        Shape::Spinner { rings, gap } => {
            ui.label(RichText::new("Rings locked together by a V ridge — print flat, then spin. W sets the diameter, H the height.").small().weak());
            sides(ui, rings, 2, 8, "rings");
            ui.add(egui::Slider::new(gap, 0.15..=0.8).text("gap (mm)"));
        }
        Shape::GearPair { teeth1, teeth2, module, gap } => {
            ui.label(RichText::new("Base with two posts set exactly the right distance apart, plus the two gears. Push the gears onto the posts.").small().weak());
            sides(ui, teeth1, 8, 80, "teeth (gear 1)");
            sides(ui, teeth2, 8, 80, "teeth (gear 2)");
            ui.add(egui::Slider::new(module, 0.8..=4.0).text("tooth size (module)"));
            ui.add(egui::Slider::new(gap, 0.1..=0.8).text("post gap (mm)"));
            ui.label(RichText::new(format!("Ratio {}:{} = {:.2}", teeth1, teeth2, *teeth2 as f64 / *teeth1 as f64)).small());
        }
        Shape::FitTest => {
            ui.label(RichText::new("1. Print this (any PLA). 2. Push the peg into each hole. 3. Click the number of the TIGHTEST hole the peg still slides into by hand:").small());
            ui.horizontal_wrapped(|ui| {
                let cur = crate::settings::fit_gap();
                for (i, g) in crate::parts::FIT_GAPS.iter().enumerate() {
                    let on = cur.map(|c| (c - g).abs() < 0.01).unwrap_or(false);
                    if ui.selectable_label(on, format!(" {} ", i + 1)).on_hover_text(format!("{g:.1} mm gap")).clicked() {
                        crate::settings::set_fit_gap(*g);
                    }
                }
            });
            match crate::settings::fit_gap() {
                Some(g) => ui.label(RichText::new(format!("Saved: {g:.1} mm. New lids, caps, nuts, hinges and dovetails use it automatically.")).small().color(Color32::from_rgb(110, 200, 130))),
                None => ui.label(RichText::new("Not set yet — parts use RoboForge's defaults (0.2 mm).").small().weak()),
            };
        }
        Shape::Sketch { spin, height, sides: n, wall, floor, .. } => {
            if *spin {
                sides(ui, n, 8, 256, "smoothness");
                let mut hollow = *wall > 0.0;
                if ui.checkbox(&mut hollow, "Hollow (open top)").changed() {
                    *wall = if hollow { 2.0 } else { 0.0 };
                    *floor = if hollow { 2.0 } else { 0.0 };
                }
                if hollow {
                    ui.add(egui::Slider::new(wall, 0.4..=10.0).text("wall (mm)"));
                    ui.add(egui::Slider::new(floor, 0.4..=20.0).text("floor (mm)"));
                }
            } else {
                ui.add(egui::Slider::new(height, 0.2..=300.0).text("height (mm)"));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(p) = std::env::var_os("ROBOFORGE_OPEN_AT_START").filter(|_| !self.opened_at_start) {
            self.opened_at_start = true;
            self.import_path(std::path::Path::new(&p));
        }
        // files dropped onto the window
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for p in dropped {
            if p.extension().map(|e| e.eq_ignore_ascii_case("pforge")).unwrap_or(false) {
                self.guarded(PendingAction::OpenPath(p), &ctx);
            } else {
                self.import_path(&p);
            }
        }
        // auto-save every minute while there are unsaved changes
        if self.unsaved() && self.last_autosave.elapsed().as_secs() >= std::env::var("ROBOFORGE_AUTOSAVE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(60) {
            if let Some(p) = crate::settings::autosave_path() {
                let _ = io::save_project(&p, &self.doc);
            }
            self.last_autosave = std::time::Instant::now();
        }
        if self.unsaved() {
            ctx.request_repaint_after(std::time::Duration::from_secs(std::env::var("ROBOFORGE_AUTOSAVE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(60) + 1));
        }
        if ctx.input(|i| i.viewport().close_requested()) && (self.allow_close || !self.unsaved()) {
            self.clear_autosave();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_discard = Some(PendingAction::Close);
        }
        self.poll_check();
        if self.check_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.shortcuts(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("tools").show(ui, |ui| self.tool_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("RoboForge v{} · {} · {}", env!("CARGO_PKG_VERSION"), self.doc.printer.name, if self.unsaved() { "unsaved changes".to_string() } else { self.file.as_ref().map(|f| f.display().to_string()).unwrap_or("not saved yet".into()) })).weak());
                });
            });
        });
        egui::Panel::left("left").resizable(true).default_size(240.0).show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("right").resizable(true).default_size(300.0).show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
        self.windows(&ctx);
        self.picture_windows(&ctx);
        // drop-target hint
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let r = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            p.rect_filled(r, 0.0, Color32::from_black_alpha(140));
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Drop to open / import", egui::FontId::proportional(28.0), Color32::WHITE);
        }
        self.commit_if_idle(&ctx);
        let title = format!(
            "{}{} — RoboForge v{}",
            if self.unsaved() { "● " } else { "" },
            self.file.as_ref().and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or("Untitled".into()),
            env!("CARGO_PKG_VERSION")
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
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
        if frame == 1 {
            if let Ok(p) = std::env::var("ROBOFORGE_SHOT_OPEN") {
                self.import_path(std::path::Path::new(&p));
            }
        }
        if frame == 2 {
            self.fit_view(false);
            if let Ok(v) = std::env::var("ROBOFORGE_SHOT_VIEW") {
                let p: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if p.len() >= 2 {
                    self.camera.yaw = p[0];
                    self.camera.pitch = p[1];
                }
                if p.len() >= 3 {
                    self.camera.dist *= p[2];
                }
            }
        }
        if frame == 6 {
            if std::env::var("ROBOFORGE_SHOT_SELECT").is_ok() {
                self.selection = self.doc.objects.first().map(|o| vec![o.id]).unwrap_or_default();
                self.geometry_dirty = true;
            }
            if let Ok(t) = std::env::var("ROBOFORGE_SHOT_TOOL") {
                self.tool = match t.as_str() { "paint" => Tool::Paint, "fill" => Tool::Fill, _ => Tool::Select };
            }
            if std::env::var("ROBOFORGE_SHOT_CHECK").is_ok() {
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

/// Edges where the surface bends sharply (> ~25°), plus open edges.
fn feature_edges(m: &Mesh) -> Vec<(Vec3, Vec3)> {
    if m.tris.len() > 400_000 {
        return vec![];
    }
    let nb = m.tri_neighbors();
    let normals: Vec<DVec3> = (0..m.tris.len()).map(|t| m.tri_normal_area(t).0).collect();
    let mut out = vec![];
    for (i, t) in m.tris.iter().enumerate() {
        for k in 0..3 {
            let j = nb[i][k];
            let sharp = j == u32::MAX || ((j as usize) > i && normals[i].dot(normals[j as usize]) < 0.9);
            if sharp && (j == u32::MAX || (j as usize) > i) {
                let (a, b) = (m.verts[t[k] as usize], m.verts[t[(k + 1) % 3] as usize]);
                out.push((a.as_vec3(), b.as_vec3()));
            }
        }
    }
    out
}

/// Text on the system clipboard (used by the right-click Paste and as a Ctrl+V backup).
fn clipboard_text() -> Option<String> {
    let t = arboard::Clipboard::new().ok()?.get_text().ok()?;
    let t = t.replace("\r\n", "\n");
    (!t.is_empty()).then_some(t)
}
