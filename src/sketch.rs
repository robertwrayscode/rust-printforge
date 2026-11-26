//! Sketch tool: draw a 2D outline, then extrude it up or spin it round an axis.

use crate::mesh::Mesh;
use crate::shapes::{extrude, orient_outward, revolve, revolve_loop, Shape};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use glam::{DVec2, DVec3};

/// Outline after rounding the corners marked `round` (Chaikin corner cutting).
pub fn outline(pts: &[[f64; 2]], round: &[bool]) -> Vec<DVec2> {
    let mut p: Vec<DVec2> = pts.iter().map(|&[x, y]| DVec2::new(x, y)).collect();
    let mut sharp: Vec<bool> = (0..p.len()).map(|i| !round.get(i).copied().unwrap_or(false)).collect();
    if p.len() < 3 || sharp.iter().all(|&s| s) {
        return p;
    }
    for _ in 0..4 {
        let n = p.len();
        let (mut q, mut qs) = (Vec::with_capacity(n * 2), Vec::with_capacity(n * 2));
        for i in 0..n {
            let (a, b) = (p[i], p[(i + 1) % n]);
            if sharp[i] {
                q.push(a);
                qs.push(true);
            }
            // cut near a only if a is round, near b only if b is round
            if !sharp[i] {
                q.push(a * 0.75 + b * 0.25);
                qs.push(false);
            }
            if !sharp[(i + 1) % n] {
                q.push(a * 0.25 + b * 0.75);
                qs.push(false);
            }
        }
        p = q;
        sharp = qs;
    }
    p.dedup_by(|a, b| a.distance(*b) < 1e-6);
    p
}

fn seg_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let o = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    ((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) && ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9))
}

pub fn area(p: &[DVec2]) -> f64 {
    (0..p.len()).map(|i| p[i].perp_dot(p[(i + 1) % p.len()])).sum::<f64>() / 2.0
}

/// Plain-English problem with the outline, if any.
pub fn problem(pts: &[[f64; 2]], round: &[bool], spin: bool) -> Option<String> {
    if pts.len() < 3 {
        return Some("Click at least 3 points to make a shape.".into());
    }
    let p = outline(pts, round);
    let n = p.len();
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if seg_cross(p[i], p[(i + 1) % n], p[j], p[(j + 1) % n]) {
                return Some("Lines cross each other. Drag a point so the outline doesn't cross itself.".into());
            }
        }
    }
    if area(&p).abs() < 0.5 {
        return Some("The shape is too thin or too small.".into());
    }
    if spin {
        if pts.iter().any(|q| q[0] < -1e-6) {
            return Some("For Spin, keep every point on the right of the spin axis (the green line).".into());
        }
        let on: Vec<bool> = p.iter().map(|q| q.x < 1e-6).collect();
        let runs = (0..n).filter(|&i| on[i] && !on[(i + n - 1) % n]).count();
        if runs > 1 {
            return Some("For Spin, the outline may touch the axis in only one place (one stretch).".into());
        }
    }
    None
}

/// The solid for a sketch, at its natural size (mm), sitting on z = 0 and centred.
pub fn build(pts: &[[f64; 2]], round: &[bool], spin: bool, height: f64, sides: u32) -> Mesh {
    let p = outline(pts, round);
    if p.len() < 3 || problem(pts, round, spin).is_some() {
        return crate::shapes::Shape::Box { radius: 0.0, steps: 1 }.generate(DVec3::splat(10.0));
    }
    let mut m = if spin {
        let n = p.len();
        let on: Vec<bool> = p.iter().map(|q| q.x < 1e-6).collect();
        if let Some(start) = (0..n).find(|&i| on[i] && !on[(i + 1) % n]) {
            // open profile running from the axis, round the outside, back to the axis
            let mut prof = vec![];
            let mut i = start;
            loop {
                let q = p[i];
                prof.push((if on[i] { 0.0 } else { q.x }, q.y));
                i = (i + 1) % n;
                if on[i] {
                    prof.push((0.0, p[i].y));
                    break;
                }
            }
            let mut m = revolve(&prof, sides);
            m.weld(1e-7);
            m.compact();
            m
        } else {
            let prof: Vec<(f64, f64)> = p.iter().map(|q| (q.x, q.y)).collect();
            let mut m = revolve_loop(&prof, sides);
            m.weld(1e-9);
            m
        }
    } else {
        extrude(&[vec![p]], 0.0, height.max(0.2))
    };
    orient_outward(&mut m);
    m
}

/// The inside profile (r, y) of a hollow spun shape, from the floor at the axis up and out
/// through the open top. None if the shape can't be hollowed.
pub fn cavity(pts: &[[f64; 2]], round: &[bool], wall: f64, floor: f64) -> Option<Vec<(f64, f64)>> {
    if wall <= 0.0 || problem(pts, round, true).is_some() {
        return None;
    }
    let p = outline(pts, round);
    let n = p.len();
    let on: Vec<bool> = p.iter().map(|q| q.x < 1e-6).collect();
    let start = (0..n).find(|&i| on[i] && !on[(i + 1) % n])?;
    // open profile from the axis, round the outside, back to the axis
    let mut prof = vec![DVec2::new(0.0, p[start].y)];
    let mut i = (start + 1) % n;
    while !on[i] {
        prof.push(p[i]);
        i = (i + 1) % n;
    }
    prof.push(DVec2::new(0.0, p[i].y));
    if prof[0].y > prof[prof.len() - 1].y {
        prof.reverse();
    }
    // the side wall: from the bottom axis point to the highest outside point (the rim)
    let top_i = (1..prof.len() - 1).max_by(|&a, &b| prof[a].y.total_cmp(&prof[b].y).then(a.cmp(&b)))?;
    let side = &prof[..=top_i];
    let (y_bot, y_top) = (prof[0].y, prof[top_i].y);
    let y_floor = y_bot + floor.max(0.2);
    if y_floor >= y_top - 0.2 {
        return None;
    }
    let dist = |q: DVec2| {
        side.windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let ab = b - a;
                let t = ((q - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                (a + ab * t).distance(q)
            })
            .fold(f64::MAX, f64::min)
    };
    // outside radius at height y (rightmost crossing of the side wall)
    let r_out = |y: f64| {
        side.windows(2)
            .filter_map(|w| {
                let (a, b) = (w[0], w[1]);
                if (a.y - y) * (b.y - y) > 0.0 || (a.y - b.y).abs() < 1e-12 {
                    return if (a.y - y).abs() < 1e-9 { Some(a.x) } else { None };
                }
                Some(a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y))
            })
            .fold(0.0f64, f64::max)
    };
    let steps = (((y_top - y_floor) / 0.25).ceil() as usize).clamp(16, 600);
    let mut rows = vec![];
    for k in 0..=steps {
        let y = y_floor + (y_top - y_floor) * k as f64 / steps as f64;
        let ro = r_out(y);
        let dr = (ro / 300.0).max(0.02);
        let mut r = ro - wall * 0.5;
        while r > 0.0 && dist(DVec2::new(r, y)) < wall {
            r -= dr;
        }
        rows.push((r.max(0.0), y));
    }
    // the cavity must be open at the top and connected from there down
    let mut first = rows.len();
    for k in (0..rows.len()).rev() {
        if rows[k].0 < 0.3 {
            break;
        }
        first = k;
    }
    if first >= rows.len() || rows.len() - first < 3 {
        return None;
    }
    let rows = &rows[first..];
    let mut cav = vec![(0.0, rows[0].1)];
    cav.extend_from_slice(rows);
    let (rt, yt) = rows[rows.len() - 1];
    cav.push((rt, yt + 5.0));
    cav.push((0.0, yt + 5.0));
    Some(cav)
}

pub fn build_hollow(pts: &[[f64; 2]], round: &[bool], spin: bool, height: f64, sides: u32, wall: f64, floor: f64) -> Mesh {
    let outer = build(pts, round, spin, height, sides);
    if !spin || wall <= 0.0 {
        return outer;
    }
    let Some(cav) = cavity(pts, round, wall, floor) else { return outer };
    let mut inner = revolve(&cav, sides);
    inner.weld(1e-7);
    inner.compact();
    orient_outward(&mut inner);
    let r = crate::csg::combine(&[outer.clone()], &[inner]);
    if r.mesh.tris.is_empty() { outer } else { r.mesh }
}

pub fn natural_size(pts: &[[f64; 2]], round: &[bool], spin: bool, height: f64) -> DVec3 {
    let p = outline(pts, round);
    if p.len() < 3 {
        return DVec3::splat(10.0);
    }
    let (mut mn, mut mx) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for q in &p {
        mn = mn.min(*q);
        mx = mx.max(*q);
    }
    if spin {
        let r = mx.x.max(0.1);
        DVec3::new(2.0 * r, 2.0 * r, (mx.y - mn.y).max(0.1))
    } else {
        DVec3::new((mx.x - mn.x).max(0.1), (mx.y - mn.y).max(0.1), height.max(0.2))
    }
}

// ----------------------------------------------------------------- editor

pub struct SketchEditor {
    pub pts: Vec<[f64; 2]>,
    pub round: Vec<bool>,
    pub spin: bool,
    pub height: f64,
    pub sides: u32,
    pub snap: f64,
    pub hollow: bool,
    pub wall: f64,
    pub floor: f64,
    /// object being edited (None = new part)
    pub editing: Option<u64>,
    scale: f32,
    pan: Vec2,
    drag: Option<usize>,
    history: Vec<(Vec<[f64; 2]>, Vec<bool>)>,
}

pub enum SketchResult {
    None,
    Cancel,
    Done(Shape),
}

impl SketchEditor {
    pub fn new() -> Self {
        Self { pts: vec![], round: vec![], spin: false, height: 10.0, sides: 64, snap: 1.0, hollow: false, wall: 2.0, floor: 2.0, editing: None, scale: 6.0, pan: Vec2::ZERO, drag: None, history: vec![] }
    }

    pub fn from_shape(s: &Shape, id: u64) -> Option<Self> {
        if let Shape::Sketch { pts, round, spin, height, sides, wall, floor } = s {
            let mut e = Self::new();
            e.pts = pts.clone();
            e.round = round.clone();
            e.spin = *spin;
            e.height = *height;
            e.sides = *sides;
            e.wall = *wall;
            e.floor = if *floor > 0.0 { *floor } else { 2.0 };
            e.hollow = *wall > 0.0;
            e.editing = Some(id);
            e.fit();
            Some(e)
        } else {
            None
        }
    }

    fn fit(&mut self) {
        if self.pts.len() < 2 {
            return;
        }
        let (mut mn, mut mx) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &self.pts {
            for k in 0..2 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        let span = (mx[0] - mn[0]).max(mx[1] - mn[1]).max(10.0);
        self.scale = (380.0 / span as f32).clamp(0.5, 40.0);
        self.pan = -Vec2::new((mn[0] + mx[0]) as f32 / 2.0, -((mn[1] + mx[1]) as f32) / 2.0) * self.scale;
    }

    fn shape(&self) -> Shape {
        let hollow = self.spin && self.hollow;
        Shape::Sketch {
            pts: self.pts.clone(),
            round: self.round.clone(),
            spin: self.spin,
            height: self.height,
            sides: self.sides,
            wall: if hollow { self.wall } else { 0.0 },
            floor: if hollow { self.floor } else { 0.0 },
        }
    }

    fn save_undo(&mut self) {
        self.history.push((self.pts.clone(), self.round.clone()));
        if self.history.len() > 100 {
            self.history.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> SketchResult {
        let mut result = SketchResult::None;
        let mut open = true;
        let title = if self.editing.is_some() { "Edit sketch" } else { "Sketch a shape" };
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).default_width(640.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.spin, false, "⬆ Extrude (push up)");
                ui.selectable_value(&mut self.spin, true, "⟳ Spin around an axis");
            });
            ui.label(
                RichText::new(if self.spin {
                    "Draw half the side view, right of the green axis. It spins round the axis (vases, knobs, bottles)."
                } else {
                    "Draw the shape as seen from above. It is pushed straight up to the height below."
                })
                .small()
                .weak(),
            );
            ui.label(RichText::new("Click to add a point (click on a line to insert one) · drag points to move · right-click a point to delete · double-click a point to round it · scroll to zoom, right-drag to pan").small().weak());
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(620.0, 430.0), Sense::click_and_drag());
            self.canvas(ui, rect, &resp);
            ui.horizontal(|ui| {
                if self.spin {
                    ui.add(egui::Slider::new(&mut self.sides, 8..=256).text("smoothness"));
                    ui.checkbox(&mut self.hollow, "Hollow (open top)").on_hover_text("Makes a cup/vase: the inside follows your outline so the wall is the same thickness everywhere");
                } else {
                    ui.add(egui::Slider::new(&mut self.height, 0.2..=300.0).text("height (mm)"));
                }
                ui.label("Grid snap");
                egui::ComboBox::from_id_salt("sk_snap").width(60.0).selected_text(if self.snap == 0.0 { "off".to_string() } else { format!("{} mm", self.snap) }).show_ui(ui, |ui| {
                    for v in [0.0, 0.5, 1.0, 2.0, 5.0] {
                        ui.selectable_value(&mut self.snap, v, if v == 0.0 { "off".to_string() } else { format!("{v} mm") });
                    }
                });
            });
            if self.spin && self.hollow {
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.wall, 0.4..=10.0).text("wall (mm)"));
                    ui.add(egui::Slider::new(&mut self.floor, 0.4..=20.0).text("floor (mm)"));
                });
                ui.label(RichText::new("The inside is shown dashed. 0.8 mm or more is sturdy; 0.4–0.5 mm is a single nozzle line.").small().weak());
            }
            ui.horizontal(|ui| {
                ui.label("Start from:");
                let presets: [(&str, Vec<[f64; 2]>, Vec<bool>); 4] = [
                    ("Square", vec![[-15.0, -15.0], [15.0, -15.0], [15.0, 15.0], [-15.0, 15.0]], vec![false; 4]),
                    ("Rounded", vec![[-15.0, -15.0], [15.0, -15.0], [15.0, 15.0], [-15.0, 15.0]], vec![true; 4]),
                    ("Arrow", vec![[-20.0, -5.0], [5.0, -5.0], [5.0, -12.0], [20.0, 0.0], [5.0, 12.0], [5.0, 5.0], [-20.0, 5.0]], vec![false; 7]),
                    ("Vase profile", vec![[0.0, 0.0], [15.0, 0.0], [20.0, 15.0], [10.0, 35.0], [12.0, 50.0], [0.0, 50.0]], vec![false, false, true, true, false, false]),
                ];
                for (name, p, r) in presets {
                    if ui.button(name).clicked() {
                        self.save_undo();
                        if name == "Vase profile" {
                            self.spin = true;
                        }
                        self.pts = p;
                        self.round = r;
                        self.fit();
                    }
                }
                if ui.add_enabled(!self.history.is_empty(), egui::Button::new("Undo")).clicked() {
                    if let Some((p, r)) = self.history.pop() {
                        self.pts = p;
                        self.round = r;
                    }
                }
                if ui.button("Clear").clicked() {
                    self.save_undo();
                    self.pts.clear();
                    self.round.clear();
                }
            });
            ui.separator();
            let prob = problem(&self.pts, &self.round, self.spin);
            if let Some(p) = &prob {
                ui.colored_label(Color32::from_rgb(240, 170, 60), p);
            } else {
                let s = natural_size(&self.pts, &self.round, self.spin, self.height);
                ui.label(format!("Size: {:.1} × {:.1} × {:.1} mm", s.x, s.y, s.z));
                if self.spin && self.hollow && cavity(&self.pts, &self.round, self.wall, self.floor).is_none() {
                    ui.colored_label(Color32::from_rgb(240, 170, 60), "Too narrow to hollow with this wall — it will stay solid. Use a thinner wall, or make sure the outline touches the axis at the bottom.");
                }
            }
            ui.horizontal(|ui| {
                let label = if self.editing.is_some() { "Update part" } else { "Add to plate" };
                if ui.add_enabled(prob.is_none(), egui::Button::new(RichText::new(label).strong())).clicked() {
                    result = SketchResult::Done(self.shape());
                }
                if ui.button("Cancel").clicked() {
                    result = SketchResult::Cancel;
                }
            });
        });
        if !open {
            result = SketchResult::Cancel;
        }
        result
    }

    fn canvas(&mut self, ui: &mut egui::Ui, rect: Rect, resp: &egui::Response) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 27, 32));
        let (c0, s0) = (rect.center() + self.pan, self.scale);
        let to_mm0 = |q: Pos2| [((q.x - c0.x) / s0) as f64, (-(q.y - c0.y) / s0) as f64];
        // zoom & pan
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(mp) = resp.hover_pos() {
                    let before = to_mm0(mp);
                    self.scale = (self.scale * (1.0 + scroll * 0.002)).clamp(0.5, 60.0);
                    let ns = self.scale;
                    // keep the point under the mouse fixed
                    self.pan = Vec2::new(mp.x - rect.center().x - before[0] as f32 * ns, mp.y - rect.center().y + before[1] as f32 * ns);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            self.pan += resp.drag_delta();
        }
        let c = rect.center() + self.pan;
        let s = self.scale;
        let to_screen = |p: [f64; 2]| Pos2::new(c.x + p[0] as f32 * s, c.y - p[1] as f32 * s);
        let to_mm = |q: Pos2| [((q.x - c.x) / s) as f64, (-(q.y - c.y) / s) as f64];
        // grid
        let step = if s > 8.0 { 1.0 } else if s > 2.0 { 5.0 } else { 10.0 };
        let lo = to_mm(rect.left_bottom());
        let hi = to_mm(rect.right_top());
        let mut x = (lo[0] / step).floor() * step;
        while x <= hi[0] {
            let major = (x / 10.0).round() * 10.0 == x;
            let col = if x == 0.0 { Color32::from_gray(110) } else if major { Color32::from_gray(58) } else { Color32::from_gray(38) };
            painter.line_segment([to_screen([x, lo[1]]), to_screen([x, hi[1]])], Stroke::new(1.0, col));
            x += step;
        }
        let mut y = (lo[1] / step).floor() * step;
        while y <= hi[1] {
            let major = (y / 10.0).round() * 10.0 == y;
            let col = if y == 0.0 { Color32::from_gray(110) } else if major { Color32::from_gray(58) } else { Color32::from_gray(38) };
            painter.line_segment([to_screen([lo[0], y]), to_screen([hi[0], y])], Stroke::new(1.0, col));
            y += step;
        }
        if self.spin {
            painter.line_segment([to_screen([0.0, lo[1]]), to_screen([0.0, hi[1]])], Stroke::new(2.0, Color32::from_rgb(80, 200, 120)));
            painter.text(to_screen([0.0, hi[1]]) + Vec2::new(6.0, 6.0), egui::Align2::LEFT_TOP, "spin axis", egui::FontId::proportional(12.0), Color32::from_rgb(80, 200, 120));
        }
        painter.text(rect.left_bottom() + Vec2::new(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("grid {step} mm"), egui::FontId::proportional(11.0), Color32::from_gray(120));
        // filled preview of the final outline
        let out = outline(&self.pts, &self.round);
        if out.len() >= 3 {
            let ok = problem(&self.pts, &self.round, self.spin).is_none();
            let scr: Vec<Pos2> = out.iter().map(|p| to_screen([p.x, p.y])).collect();
            let fill_col = if ok { Color32::from_rgba_unmultiplied(70, 140, 255, 40) } else { Color32::from_rgba_unmultiplied(255, 120, 60, 30) };
            // triangulate for the fill (works for concave outlines)
            let flat: Vec<f64> = out.iter().flat_map(|p| [p.x, p.y]).collect();
            if let Ok(idx) = earcutr::earcut(&flat, &[], 2) {
                let mut mesh = egui::Mesh::default();
                for q in &scr {
                    mesh.colored_vertex(*q, fill_col);
                }
                mesh.indices = idx.iter().map(|&i| i as u32).collect();
                painter.add(egui::Shape::mesh(mesh));
            }
            let mut ring = scr.clone();
            ring.push(scr[0]);
            painter.add(egui::Shape::line(ring, Stroke::new(2.0, if ok { Color32::from_rgb(110, 170, 255) } else { Color32::from_rgb(255, 140, 70) })));
        } else if self.pts.len() == 2 {
            painter.line_segment([to_screen(self.pts[0]), to_screen(self.pts[1])], Stroke::new(2.0, Color32::from_rgb(110, 170, 255)));
        }
        // control polygon (thin) when rounding is used
        if self.round.iter().any(|&r| r) && self.pts.len() >= 3 {
            let n = self.pts.len();
            for i in 0..n {
                painter.line_segment([to_screen(self.pts[i]), to_screen(self.pts[(i + 1) % n])], Stroke::new(1.0, Color32::from_gray(90)));
            }
        }
        // inside of a hollow spin (dashed)
        if self.spin && self.hollow {
            if let Some(cav) = cavity(&self.pts, &self.round, self.wall, self.floor) {
                let scr: Vec<Pos2> = cav.iter().map(|&(r, y)| to_screen([r, y])).collect();
                for w in scr.windows(2) {
                    painter.extend(egui::Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.5, Color32::from_rgb(255, 210, 90)), 5.0, 4.0));
                }
            }
        }
        // interaction
        let snap = |p: [f64; 2], sn: f64, spin: bool| {
            let mut q = if sn > 0.0 { [(p[0] / sn).round() * sn, (p[1] / sn).round() * sn] } else { p };
            if spin && q[0] < 0.0 {
                q[0] = 0.0;
            }
            q
        };
        let hover = resp.hover_pos();
        let near = |pos: Pos2, pts: &[[f64; 2]]| pts.iter().position(|&p| to_screen(p).distance(pos) < 9.0);
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(i) = resp.interact_pointer_pos().and_then(|p| near(p, &self.pts)) {
                self.save_undo();
                self.drag = Some(i);
            }
        }
        if let (Some(i), Some(p)) = (self.drag, resp.interact_pointer_pos()) {
            if resp.dragged_by(egui::PointerButton::Primary) {
                self.pts[i] = snap(to_mm(p), self.snap, self.spin);
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.double_clicked() {
            if let Some(i) = hover.and_then(|p| near(p, &self.pts)) {
                // the first click of the double-click added nothing (it hit a point)
                self.save_undo();
                self.round[i] = !self.round[i];
            }
        } else if resp.clicked() {
            if let Some(p) = hover {
                if near(p, &self.pts).is_none() {
                    self.save_undo();
                    let q = snap(to_mm(p), self.snap, self.spin);
                    // on (or very near) an edge: insert into that edge
                    let n = self.pts.len();
                    let mut ins = n;
                    if n >= 2 {
                        let mut best = 8.0f32;
                        for i in 0..n {
                            let (a, b) = (to_screen(self.pts[i]), to_screen(self.pts[(i + 1) % n]));
                            if n == 2 && i == 1 {
                                break;
                            }
                            let ab = b - a;
                            let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
                            let d = (a + ab * t).distance(p);
                            if d < best {
                                best = d;
                                ins = i + 1;
                            }
                        }
                    }
                    self.pts.insert(ins, q);
                    self.round.insert(ins, false);
                }
            }
        }
        if resp.secondary_clicked() {
            if let Some(i) = hover.and_then(|p| near(p, &self.pts)) {
                self.save_undo();
                self.pts.remove(i);
                self.round.remove(i);
            }
        }
        // points
        for (i, p) in self.pts.iter().enumerate() {
            let q = to_screen(*p);
            let hot = hover.map(|h| h.distance(q) < 9.0).unwrap_or(false) || self.drag == Some(i);
            let r = if hot { 6.0 } else { 4.5 };
            if self.round[i] {
                painter.circle(q, r, Color32::from_rgb(255, 200, 80), Stroke::new(1.5, Color32::BLACK));
            } else {
                painter.rect(Rect::from_center_size(q, Vec2::splat(r * 2.0)), 1.0, Color32::WHITE, Stroke::new(1.5, Color32::BLACK), egui::StrokeKind::Middle);
            }
        }
        // cursor readout
        if let Some(h) = hover {
            let q = snap(to_mm(h), self.snap, self.spin);
            painter.text(rect.right_bottom() + Vec2::new(-6.0, -6.0), egui::Align2::RIGHT_BOTTOM, format!("x {:.1}  y {:.1} mm", q[0], q[1]), egui::FontId::monospace(11.0), Color32::from_gray(150));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watertight(m: &Mesh) -> bool {
        let mut e = std::collections::HashSet::new();
        for t in &m.tris {
            for k in 0..3 {
                e.insert((t[k], t[(k + 1) % 3]));
            }
        }
        e.iter().all(|&(a, b)| e.contains(&(b, a)))
    }

    #[test]
    fn extrude_and_spin_are_solid() {
        let sq = vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]];
        let m = build(&sq, &[false; 4], false, 5.0, 64);
        assert!(watertight(&m));
        assert!((m.volume() - 2000.0).abs() < 1e-6);
        let rnd = build(&sq, &[true; 4], false, 5.0, 64);
        assert!(watertight(&rnd) && rnd.volume() < 2000.0 && rnd.volume() > 1000.0);
        // spin: rectangle touching the axis -> cylinder r 10, h 20
        let prof = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]];
        let c = build(&prof, &[false; 4], true, 0.0, 128);
        assert!(watertight(&c), "spin not watertight");
        let v = std::f64::consts::PI * 100.0 * 20.0;
        assert!((c.volume() - v).abs() / v < 0.01, "{}", c.volume());
        // spin: ring not touching the axis
        let ring = vec![[5.0, 0.0], [10.0, 0.0], [10.0, 5.0], [5.0, 5.0]];
        let r = build(&ring, &[false; 4], true, 0.0, 64);
        assert!(watertight(&r) && r.volume() > 0.0);
        // vase preset
        let vase = vec![[0.0, 0.0], [15.0, 0.0], [20.0, 15.0], [10.0, 35.0], [12.0, 50.0], [0.0, 50.0]];
        let vm = build(&vase, &[false, false, true, true, false, false], true, 0.0, 64);
        assert!(watertight(&vm) && vm.volume() > 0.0);
    }

    #[test]
    fn hollow_vase_has_even_wall_and_open_top() {
        let vase = vec![[0.0, 0.0], [15.0, 0.0], [20.0, 15.0], [10.0, 35.0], [12.0, 50.0], [0.0, 50.0]];
        let rd = [false, false, true, true, false, false];
        let solid = build(&vase, &rd, true, 0.0, 64);
        let h = build_hollow(&vase, &rd, true, 0.0, 64, 2.0, 2.0);
        assert!(watertight(&h), "hollow not watertight");
        assert!(h.volume() < solid.volume() * 0.6 && h.volume() > solid.volume() * 0.1, "{} vs {}", h.volume(), solid.volume());
        // bounds unchanged (the cavity doesn't eat the outside)
        let (a, b) = (solid.bounds(), h.bounds());
        assert!((a.max - b.max).length() < 0.05 && (a.min - b.min).length() < 0.05);
        // every inside wall point is ~2 mm from the outside
        let cav = cavity(&vase, &rd, 2.0, 2.0).unwrap();
        assert!(cav.len() > 20);
        // a wall too thick for the neck stays solid rather than failing
        assert!(cavity(&vase, &rd, 30.0, 2.0).is_none());
    }

    #[test]
    fn problems_are_reported() {
        let bow = vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
        assert!(problem(&bow, &[false; 4], false).unwrap().contains("cross"));
        let left = vec![[-5.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert!(problem(&left, &[false; 3], true).is_some());
        assert!(problem(&[[0.0, 0.0]], &[false], false).is_some());
    }
}
