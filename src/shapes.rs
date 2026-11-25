//! Parametric shape library (Tinkercad's "Basic Shapes" plus extras).
//! Every generator returns a closed, welded mesh sitting on z=0, centred on X/Y,
//! filling exactly the requested size (W x D x H in mm).

use crate::mesh::Mesh;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Shape {
    Box { radius: f64, steps: u32 },
    Cylinder { sides: u32, bevel: f64 },
    Sphere { steps: u32 },
    HalfSphere { steps: u32 },
    Cone { sides: u32, top: f64 },
    Pyramid { sides: u32 },
    Roof,
    RoundRoof { sides: u32 },
    Wedge,
    Torus { sides: u32, tube: f64 },
    Tube { sides: u32, wall: f64 },
    Paraboloid { sides: u32 },
    Polygon { sides: u32 },
    Star { points: u32, inner: f64 },
    Heart,
    Text { text: String, font: FontChoice, spacing: f64 },
    // ---- functional parts
    Container { wall: f64, floor: f64, radius: f64 },
    Lid { wall: f64, lip: f64, clearance: f64, radius: f64 },
    Bracket { thickness: f64, hole: f64 },
    Gear { teeth: u32, bore: f64 },
    Knob { grips: u32, bore: f64 },
    CableClip { cable: f64 },
    Bolt { diameter: f64, length: f64 },
    Nut { diameter: f64, clearance: f64 },
    Hinge { knuckles: u32, clearance: f64 },
    SnapHook { arm: f64, catch: f64 },
    Washer { hole: f64 },
    Standoff { hole: f64 },
    Pulley { bore: f64, groove: f64 },
    Wheel { bore: f64, spokes: u32 },
    Spring { coils: f64, wire: f64 },
    Rack { module: f64 },
    Dovetail { clearance: f64 },
    LivingHinge { film: f64, gap: f64 },
    WallHook { hole: f64 },
    PegHook,
    PipeClamp { pipe: f64 },
    CornerBrace { hole: f64 },
    KeyholePlate,
    CableComb { slots: u32, cable: f64 },
    Grommet { hole: f64 },
    Handle { hole: f64 },
    PhoneStand { angle: f64 },
    DividedBin { cols: u32, rows: u32, wall: f64 },
    Planter { wall: f64, hole: f64 },
    Saucer,
    Funnel { spout: f64 },
    KeyTag { hole: f64 },
    Jar { wall: f64 },
    JarCap { clearance: f64 },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FontChoice {
    Sans,
    Mono,
}

pub struct ShapeInfo {
    pub category: &'static str,
    pub name: &'static str,
    pub shape: fn() -> Shape,
    pub default_size: [f64; 3],
}

pub const LIBRARY: &[ShapeInfo] = &[
    ShapeInfo { category: "Basic shapes", name: "Box", shape: || Shape::Box { radius: 0.0, steps: 4 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Rounded box", shape: || Shape::Box { radius: 3.0, steps: 4 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Cylinder", shape: || Shape::Cylinder { sides: 48, bevel: 0.0 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Sphere", shape: || Shape::Sphere { steps: 24 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Half sphere", shape: || Shape::HalfSphere { steps: 24 }, default_size: [20.0, 20.0, 10.0] },
    ShapeInfo { category: "Basic shapes", name: "Cone", shape: || Shape::Cone { sides: 48, top: 0.0 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Pyramid", shape: || Shape::Pyramid { sides: 4 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Roof", shape: || Shape::Roof, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Round roof", shape: || Shape::RoundRoof { sides: 32 }, default_size: [20.0, 20.0, 10.0] },
    ShapeInfo { category: "Basic shapes", name: "Wedge", shape: || Shape::Wedge, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Torus", shape: || Shape::Torus { sides: 48, tube: 0.15 }, default_size: [20.0, 20.0, 6.0] },
    ShapeInfo { category: "Basic shapes", name: "Tube", shape: || Shape::Tube { sides: 48, wall: 2.0 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Paraboloid", shape: || Shape::Paraboloid { sides: 48 }, default_size: [20.0, 20.0, 20.0] },
    ShapeInfo { category: "Basic shapes", name: "Polygon", shape: || Shape::Polygon { sides: 6 }, default_size: [20.0, 20.0, 10.0] },
    ShapeInfo { category: "Basic shapes", name: "Star", shape: || Shape::Star { points: 5, inner: 0.45 }, default_size: [20.0, 20.0, 5.0] },
    ShapeInfo { category: "Basic shapes", name: "Heart", shape: || Shape::Heart, default_size: [20.0, 18.0, 5.0] },
    ShapeInfo {
        category: "Basic shapes",
        name: "Text",
        shape: || Shape::Text { text: "TEXT".into(), font: FontChoice::Sans, spacing: 0.0 },
        default_size: [40.0, 12.0, 4.0],
    },
    ShapeInfo { category: "Home & storage", name: "Box + lid", shape: || Shape::Container { wall: 2.0, floor: 2.0, radius: 4.0 }, default_size: [60.0, 40.0, 30.0] },
    ShapeInfo { category: "Home & storage", name: "Lid", shape: || Shape::Lid { wall: 2.0, lip: 4.0, clearance: 0.25, radius: 4.0 }, default_size: [60.0, 40.0, 6.0] },
    ShapeInfo { category: "Mounting", name: "Bracket", shape: || Shape::Bracket { thickness: 3.0, hole: 4.2 }, default_size: [30.0, 25.0, 25.0] },
    ShapeInfo { category: "Hardware", name: "Gear", shape: || Shape::Gear { teeth: 20, bore: 5.0 }, default_size: [40.0, 40.0, 6.0] },
    ShapeInfo { category: "Home & storage", name: "Knob", shape: || Shape::Knob { grips: 12, bore: 6.0 }, default_size: [30.0, 30.0, 18.0] },
    ShapeInfo { category: "Mounting", name: "Cable clip", shape: || Shape::CableClip { cable: 6.0 }, default_size: [10.0, 23.6, 11.6] },
    ShapeInfo { category: "Hardware", name: "Bolt", shape: || Shape::Bolt { diameter: 8.0, length: 20.0 }, default_size: [15.70393, 13.6, 25.6] },
    ShapeInfo { category: "Hardware", name: "Nut", shape: || Shape::Nut { diameter: 8.0, clearance: 0.25 }, default_size: [15.70393, 13.6, 6.4] },
    ShapeInfo { category: "Hardware", name: "Hinge", shape: || Shape::Hinge { knuckles: 5, clearance: 0.3 }, default_size: [40.0, 30.0, 6.0] },
    ShapeInfo { category: "Hardware", name: "Snap hook", shape: || Shape::SnapHook { arm: 1.4, catch: 1.0 }, default_size: [6.0, 8.0, 15.0] },
    ShapeInfo { category: "Hardware", name: "Washer", shape: || Shape::Washer { hole: 8.4 }, default_size: [16.0, 16.0, 1.6] },
    ShapeInfo { category: "Hardware", name: "Standoff", shape: || Shape::Standoff { hole: 3.2 }, default_size: [7.0, 7.0, 10.0] },
    ShapeInfo { category: "Hardware", name: "Pulley", shape: || Shape::Pulley { bore: 5.0, groove: 3.0 }, default_size: [30.0, 30.0, 10.0] },
    ShapeInfo { category: "Hardware", name: "Wheel", shape: || Shape::Wheel { bore: 5.0, spokes: 6 }, default_size: [50.0, 50.0, 12.0] },
    ShapeInfo { category: "Hardware", name: "Spring", shape: || Shape::Spring { coils: 6.0, wire: 2.0 }, default_size: [20.0, 20.0, 30.0] },
    ShapeInfo { category: "Hardware", name: "Gear rack", shape: || Shape::Rack { module: 1.818 }, default_size: [80.0, 6.0, 10.0] },
    ShapeInfo { category: "Hardware", name: "Dovetail", shape: || Shape::Dovetail { clearance: 0.2 }, default_size: [60.0, 30.0, 15.0] },
    ShapeInfo { category: "Hardware", name: "Living hinge", shape: || Shape::LivingHinge { film: 0.6, gap: 4.0 }, default_size: [60.0, 30.0, 3.0] },
    ShapeInfo { category: "Mounting", name: "Wall hook", shape: || Shape::WallHook { hole: 4.2 }, default_size: [15.0, 30.0, 50.0] },
    ShapeInfo { category: "Mounting", name: "Pegboard hook", shape: || Shape::PegHook, default_size: [12.0, 60.0, 40.0] },
    ShapeInfo { category: "Mounting", name: "Pipe clamp", shape: || Shape::PipeClamp { pipe: 20.0 }, default_size: [12.0, 43.0, 12.5] },
    ShapeInfo { category: "Mounting", name: "Corner brace", shape: || Shape::CornerBrace { hole: 4.2 }, default_size: [40.0, 40.0, 3.0] },
    ShapeInfo { category: "Mounting", name: "Keyhole plate", shape: || Shape::KeyholePlate, default_size: [60.0, 30.0, 4.0] },
    ShapeInfo { category: "Mounting", name: "Cable comb", shape: || Shape::CableComb { slots: 5, cable: 5.0 }, default_size: [45.0, 10.0, 12.0] },
    ShapeInfo { category: "Mounting", name: "Desk grommet", shape: || Shape::Grommet { hole: 50.0 }, default_size: [60.0, 60.0, 20.0] },
    ShapeInfo { category: "Home & storage", name: "Divided bin", shape: || Shape::DividedBin { cols: 3, rows: 2, wall: 1.6 }, default_size: [100.0, 60.0, 40.0] },
    ShapeInfo { category: "Home & storage", name: "Screw jar", shape: || Shape::Jar { wall: 2.0 }, default_size: [60.0, 60.0, 70.0] },
    ShapeInfo { category: "Home & storage", name: "Jar cap", shape: || Shape::JarCap { clearance: 0.4 }, default_size: [65.0, 65.0, 17.5] },
    ShapeInfo { category: "Home & storage", name: "Planter", shape: || Shape::Planter { wall: 2.0, hole: 8.0 }, default_size: [80.0, 80.0, 70.0] },
    ShapeInfo { category: "Home & storage", name: "Saucer", shape: || Shape::Saucer, default_size: [80.0, 80.0, 10.0] },
    ShapeInfo { category: "Home & storage", name: "Funnel", shape: || Shape::Funnel { spout: 10.0 }, default_size: [80.0, 80.0, 70.0] },
    ShapeInfo { category: "Home & storage", name: "Handle", shape: || Shape::Handle { hole: 4.2 }, default_size: [100.0, 15.0, 30.0] },
    ShapeInfo { category: "Home & storage", name: "Phone stand", shape: || Shape::PhoneStand { angle: 65.0 }, default_size: [70.0, 80.0, 90.0] },
    ShapeInfo { category: "Home & storage", name: "Key tag", shape: || Shape::KeyTag { hole: 5.0 }, default_size: [50.0, 20.0, 3.0] },
];

impl Shape {
    pub fn label(&self) -> &'static str {
        match self {
            Shape::Box { radius, .. } if *radius > 0.0 => "Rounded box",
            Shape::Box { .. } => "Box",
            Shape::Cylinder { .. } => "Cylinder",
            Shape::Sphere { .. } => "Sphere",
            Shape::HalfSphere { .. } => "Half sphere",
            Shape::Cone { .. } => "Cone",
            Shape::Pyramid { .. } => "Pyramid",
            Shape::Roof => "Roof",
            Shape::RoundRoof { .. } => "Round roof",
            Shape::Wedge => "Wedge",
            Shape::Torus { .. } => "Torus",
            Shape::Tube { .. } => "Tube",
            Shape::Paraboloid { .. } => "Paraboloid",
            Shape::Polygon { .. } => "Polygon",
            Shape::Star { .. } => "Star",
            Shape::Heart => "Heart",
            Shape::Text { .. } => "Text",
            Shape::Container { .. } => "Container",
            Shape::Lid { .. } => "Lid",
            Shape::Bracket { .. } => "Bracket",
            Shape::Gear { .. } => "Gear",
            Shape::Knob { .. } => "Knob",
            Shape::CableClip { .. } => "Cable clip",
            Shape::Bolt { .. } => "Bolt",
            Shape::Nut { .. } => "Nut",
            Shape::Hinge { .. } => "Hinge",
            Shape::SnapHook { .. } => "Snap hook",
            Shape::Washer { .. } => "Washer",
            Shape::Standoff { .. } => "Standoff",
            Shape::Pulley { .. } => "Pulley",
            Shape::Wheel { .. } => "Wheel",
            Shape::Spring { .. } => "Spring",
            Shape::Rack { .. } => "Gear rack",
            Shape::Dovetail { .. } => "Dovetail",
            Shape::LivingHinge { .. } => "Living hinge",
            Shape::WallHook { .. } => "Wall hook",
            Shape::PegHook => "Pegboard hook",
            Shape::PipeClamp { .. } => "Pipe clamp",
            Shape::CornerBrace { .. } => "Corner brace",
            Shape::KeyholePlate => "Keyhole plate",
            Shape::CableComb { .. } => "Cable comb",
            Shape::Grommet { .. } => "Desk grommet",
            Shape::Handle { .. } => "Handle",
            Shape::PhoneStand { .. } => "Phone stand",
            Shape::DividedBin { .. } => "Divided bin",
            Shape::Planter { .. } => "Planter",
            Shape::Saucer => "Saucer",
            Shape::Funnel { .. } => "Funnel",
            Shape::KeyTag { .. } => "Key tag",
            Shape::Jar { .. } => "Screw jar",
            Shape::JarCap { .. } => "Jar cap",
        }
    }

    /// Parts whose size follows from their settings (threads etc. shouldn't be stretched).
    pub fn natural_size(&self) -> Option<DVec3> {
        match self {
            Shape::Bolt { diameter, length } => Some(crate::parts::bolt_size(*diameter, *length)),
            Shape::Nut { diameter, .. } => Some(crate::parts::nut_size(*diameter)),
            Shape::CableClip { cable } => Some(crate::parts::cable_clip_size(*cable)),
            Shape::PipeClamp { pipe } => Some(crate::parts::pipe_clamp_size(*pipe)),
            _ => None,
        }
    }

    pub fn generate(&self, size: DVec3) -> Mesh {
        let size = size.max(DVec3::splat(0.01));
        if let Some(m) = self.generate_part(size) {
            // centred on X/Y, sitting on z = 0, exactly the requested size
            return crate::scene::fit_to_size(&m, size);
        }
        let mut m = match self {
            Shape::Box { radius, steps } => rounded_box(size, *radius, (*steps).clamp(1, 16)),
            Shape::Cylinder { sides, bevel } => {
                if *bevel > 0.0 {
                    bevel_cylinder(size, *bevel, (*sides).clamp(3, 256))
                } else {
                    unit(revolve(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)], *sides), size)
                }
            }
            Shape::Sphere { steps } => {
                let n = (*steps).clamp(4, 128);
                let prof: Vec<(f64, f64)> = (0..=n)
                    .map(|i| {
                        let a = PI * i as f64 / n as f64;
                        (0.5 * a.sin(), 0.5 - 0.5 * a.cos())
                    })
                    .collect();
                unit(revolve(&prof, n * 2), size)
            }
            Shape::HalfSphere { steps } => {
                let n = (*steps).clamp(4, 128);
                let mut prof = vec![(0.0, 0.0)];
                prof.extend((0..=n / 2).map(|i| {
                    let a = 0.5 * PI * i as f64 / (n / 2) as f64;
                    (0.5 * a.cos(), a.sin())
                }));
                unit(revolve(&prof, n * 2), size)
            }
            Shape::Cone { sides, top } => {
                let t = top.clamp(0.0, 1.0) * 0.5;
                unit(revolve(&[(0.0, 0.0), (0.5, 0.0), (t, 1.0), (0.0, 1.0)], *sides), size)
            }
            Shape::Paraboloid { sides } => {
                let n = 24;
                let mut prof = vec![(0.0, 0.0)];
                prof.extend((0..=n).map(|i| {
                    let z = i as f64 / n as f64;
                    (0.5 * (1.0 - z).max(0.0).sqrt(), z)
                }));
                unit(revolve(&prof, *sides), size)
            }
            Shape::Torus { sides, tube } => {
                let t = tube.clamp(0.02, 0.25);
                let n = 24;
                let prof: Vec<(f64, f64)> = (0..n)
                    .map(|i| {
                        let a = TAU * i as f64 / n as f64;
                        (0.5 - t + t * a.cos(), 0.5 + 0.5 * a.sin())
                    })
                    .collect();
                unit(revolve_loop(&prof, *sides), size)
            }
            Shape::Tube { sides, wall } => tube(size, *wall, (*sides).clamp(3, 256)),
            Shape::Pyramid { sides } => {
                let n = (*sides).clamp(3, 64);
                let base = fit_unit(ngon(n));
                let mut m = Mesh::new();
                let apex = m.add_vert(DVec3::new(0.0, 0.0, 1.0));
                let c = m.add_vert(DVec3::ZERO);
                let ids: Vec<u32> = base.iter().map(|p| m.add_vert(DVec3::new(p.x, p.y, 0.0))).collect();
                for i in 0..ids.len() {
                    let j = (i + 1) % ids.len();
                    m.add_tri(ids[i], ids[j], apex);
                    m.add_tri(ids[j], ids[i], c);
                }
                unit(m, size)
            }
            Shape::Polygon { sides } => unit(extrude(&[vec![fit_unit(ngon((*sides).clamp(3, 64)))]], 0.0, 1.0), size),
            Shape::Star { points, inner } => {
                let n = (*points).clamp(3, 64) as usize;
                let poly: Vec<DVec2> = (0..2 * n)
                    .map(|i| {
                        let r = if i % 2 == 0 { 0.5 } else { 0.5 * inner.clamp(0.05, 0.99) };
                        let a = PI / 2.0 + PI * i as f64 / n as f64;
                        DVec2::new(r * a.cos(), r * a.sin())
                    })
                    .collect();
                unit(extrude(&[vec![fit_unit(poly)]], 0.0, 1.0), size)
            }
            Shape::Heart => {
                let poly: Vec<DVec2> = (0..96)
                    .map(|i| {
                        let t = TAU * i as f64 / 96.0;
                        let x = 16.0 * t.sin().powi(3);
                        let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                        DVec2::new(x, y)
                    })
                    .collect();
                let mut p = fit_unit(poly);
                p.dedup_by(|a, b| a.distance(*b) < 1e-9);
                unit(extrude(&[vec![p]], 0.0, 1.0), size)
            }
            Shape::Roof => side_profile(&[DVec2::new(-0.5, 0.0), DVec2::new(0.5, 0.0), DVec2::new(0.0, 1.0)], size),
            Shape::Wedge => side_profile(&[DVec2::new(-0.5, 0.0), DVec2::new(0.5, 0.0), DVec2::new(-0.5, 1.0)], size),
            Shape::RoundRoof { sides } => {
                let n = (*sides).clamp(4, 256);
                let p: Vec<DVec2> = (0..=n)
                    .map(|i| {
                        let a = PI * i as f64 / n as f64;
                        DVec2::new(0.5 * a.cos(), a.sin())
                    })
                    .collect();
                side_profile(&p, size)
            }
            Shape::Text { text, font, spacing } => {
                let polys = text_polygons(text, *font, *spacing);
                if polys.is_empty() {
                    rounded_box(DVec3::new(size.x, size.y, size.z), 0.0, 1)
                } else {
                    let all: Vec<DVec2> = polys.iter().flatten().flatten().copied().collect();
                    let (mn, mx) = bounds2(&all);
                    let ext = (mx - mn).max(DVec2::splat(1e-9));
                    let norm: Vec<Vec<Vec<DVec2>>> = polys
                        .iter()
                        .map(|g| g.iter().map(|c| c.iter().map(|p| (*p - mn) / ext - DVec2::splat(0.5)).collect()).collect())
                        .collect();
                    unit(extrude(&norm, 0.0, 1.0), size)
                }
            }
            _ => rounded_box(size, 0.0, 1),
        };
        m.weld(1e-7);
        orient_outward(&mut m);
        m
    }

    fn generate_part(&self, size: DVec3) -> Option<Mesh> {
        use crate::parts::*;
        Some(match self {
            Shape::Container { wall, floor, radius } => container(size, *wall, *floor, *radius),
            Shape::Lid { wall, lip, clearance, radius } => lid(size, *wall, *lip, *clearance, *radius),
            Shape::Bracket { thickness, hole } => bracket(size, *thickness, *hole),
            Shape::Gear { teeth, bore } => gear(size, *teeth, *bore),
            Shape::Knob { grips, bore } => knob(size, *grips, *bore),
            Shape::CableClip { cable } => cable_clip(size, cable.clamp(1.0, 40.0)),
            Shape::Bolt { diameter, length } => {
                let d = diameter.clamp(2.0, 30.0);
                crate::scene::fit_to_size(&bolt(d, length.clamp(2.0, 300.0), coarse_pitch(d)), size)
            }
            Shape::Nut { diameter, clearance } => {
                let d = diameter.clamp(2.0, 30.0);
                crate::scene::fit_to_size(&nut(d, coarse_pitch(d), *clearance), size)
            }
            Shape::Hinge { knuckles, clearance } => hinge(size, *knuckles, *clearance),
            Shape::SnapHook { arm, catch } => snap_hook(size, *arm, *catch),
            Shape::Washer { hole } => washer(size, *hole),
            Shape::Standoff { hole } => standoff(size, *hole),
            Shape::Pulley { bore, groove } => pulley(size, *bore, *groove),
            Shape::Wheel { bore, spokes } => wheel(size, *bore, *spokes),
            Shape::Spring { coils, wire } => spring(size, *coils, *wire),
            Shape::Rack { module } => rack(size, *module),
            Shape::Dovetail { clearance } => dovetail(size, *clearance),
            Shape::LivingHinge { film, gap } => living_hinge(size, *film, *gap),
            Shape::WallHook { hole } => wall_hook(size, *hole),
            Shape::PegHook => peg_hook(size),
            Shape::PipeClamp { pipe } => crate::scene::fit_to_size(&pipe_clamp(pipe_clamp_size(*pipe), *pipe), size),
            Shape::CornerBrace { hole } => corner_brace(size, *hole),
            Shape::KeyholePlate => keyhole_plate(size),
            Shape::CableComb { slots, cable } => cable_comb(size, *slots, *cable),
            Shape::Grommet { hole } => grommet(size, *hole),
            Shape::Handle { hole } => handle(size, *hole),
            Shape::PhoneStand { angle } => phone_stand(size, *angle),
            Shape::DividedBin { cols, rows, wall } => divided_bin(size, *cols, *rows, *wall),
            Shape::Planter { wall, hole } => planter(size, *wall, *hole),
            Shape::Saucer => saucer(size),
            Shape::Funnel { spout } => funnel(size, *spout),
            Shape::KeyTag { hole } => key_tag(size, *hole),
            Shape::Jar { wall } => jar(size, *wall),
            Shape::JarCap { clearance } => jar_cap(size, *clearance),
            _ => return None,
        })
    }

    /// Natural aspect for text so new text objects aren't squashed.
    pub fn text_aspect(&self) -> Option<f64> {
        if let Shape::Text { text, font, spacing } = self {
            let polys = text_polygons(text, *font, *spacing);
            let all: Vec<DVec2> = polys.iter().flatten().flatten().copied().collect();
            if all.is_empty() {
                return None;
            }
            let (mn, mx) = bounds2(&all);
            Some((mx.x - mn.x) / (mx.y - mn.y).max(1e-9))
        } else {
            None
        }
    }
}

fn unit(mut m: Mesh, size: DVec3) -> Mesh {
    m.scale_to(size);
    m
}

pub fn orient_outward(m: &mut Mesh) {
    if m.volume() < 0.0 {
        m.flip();
    }
}

fn bounds2(p: &[DVec2]) -> (DVec2, DVec2) {
    let mut mn = DVec2::splat(f64::INFINITY);
    let mut mx = DVec2::splat(f64::NEG_INFINITY);
    for q in p {
        mn = mn.min(*q);
        mx = mx.max(*q);
    }
    (mn, mx)
}

fn ngon(n: u32) -> Vec<DVec2> {
    let off = if n % 2 == 0 { PI / n as f64 } else { PI / 2.0 };
    (0..n)
        .map(|i| {
            let a = off + TAU * i as f64 / n as f64;
            DVec2::new(a.cos(), a.sin())
        })
        .collect()
}

/// Scale a polygon so its bounding box is exactly [-0.5, 0.5]^2.
fn fit_unit(p: Vec<DVec2>) -> Vec<DVec2> {
    let (mn, mx) = bounds2(&p);
    let ext = (mx - mn).max(DVec2::splat(1e-12));
    p.into_iter().map(|q| (q - mn) / ext - DVec2::splat(0.5)).collect()
}

fn signed_area(p: &[DVec2]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let j = (i + 1) % p.len();
        a += p[i].x * p[j].y - p[j].x * p[i].y;
    }
    a * 0.5
}

/// Surface of revolution around Z. `prof` is (radius, z) from the axis back to the axis.
pub fn revolve(prof: &[(f64, f64)], sides: u32) -> Mesh {
    let n = sides.clamp(3, 512) as usize;
    let mut m = Mesh::new();
    let rows: Vec<Vec<u32>> = prof
        .iter()
        .map(|&(r, z)| {
            (0..n)
                .map(|i| {
                    let a = TAU * i as f64 / n as f64;
                    m.add_vert(DVec3::new(r * a.cos(), r * a.sin(), z))
                })
                .collect()
        })
        .collect();
    for k in 0..rows.len() - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            m.add_quad(rows[k][i], rows[k][j], rows[k + 1][j], rows[k + 1][i]);
        }
    }
    m
}

/// Revolve a closed profile loop (torus-like).
pub fn revolve_loop(prof: &[(f64, f64)], sides: u32) -> Mesh {
    let mut p = prof.to_vec();
    p.push(prof[0]);
    revolve(&p, sides)
}

/// Extrude polygons (each group = outer contour followed by its holes) between z0 and z1.
pub fn extrude(groups: &[Vec<Vec<DVec2>>], z0: f64, z1: f64) -> Mesh {
    let mut m = Mesh::new();
    for g in groups {
        if g.is_empty() || g[0].len() < 3 {
            continue;
        }
        // normalise orientation: outer CCW, holes CW
        let mut contours: Vec<Vec<DVec2>> = Vec::new();
        for (ci, c) in g.iter().enumerate() {
            let mut c = c.clone();
            if c.len() > 1 && c[0].distance(*c.last().unwrap()) < 1e-12 {
                c.pop();
            }
            if c.len() < 3 {
                continue;
            }
            let a = signed_area(&c);
            if (ci == 0 && a < 0.0) || (ci > 0 && a > 0.0) {
                c.reverse();
            }
            contours.push(c);
        }
        if contours.is_empty() {
            continue;
        }
        let mut flat = Vec::new();
        let mut holes = Vec::new();
        let mut all: Vec<DVec2> = Vec::new();
        for (ci, c) in contours.iter().enumerate() {
            if ci > 0 {
                holes.push(all.len());
            }
            for p in c {
                flat.push(p.x);
                flat.push(p.y);
                all.push(*p);
            }
        }
        let bot: Vec<u32> = all.iter().map(|p| m.add_vert(DVec3::new(p.x, p.y, z0))).collect();
        let top: Vec<u32> = all.iter().map(|p| m.add_vert(DVec3::new(p.x, p.y, z1))).collect();
        if let Ok(idx) = earcutr::earcut(&flat, &holes, 2) {
            for t in idx.chunks(3) {
                let (a, b, c) = (t[0], t[1], t[2]);
                let ccw = (all[b] - all[a]).perp_dot(all[c] - all[a]) > 0.0;
                if ccw {
                    m.add_tri(top[a], top[b], top[c]);
                    m.add_tri(bot[a], bot[c], bot[b]);
                } else {
                    m.add_tri(top[a], top[c], top[b]);
                    m.add_tri(bot[a], bot[b], bot[c]);
                }
            }
        }
        let mut start = 0;
        for c in &contours {
            let n = c.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (a, b) = (start + i, start + j);
                m.add_quad(bot[a], bot[b], top[b], top[a]);
            }
            start += n;
        }
    }
    m
}

/// Profile drawn in the XZ plane (x across, y = height in [0,1]) extruded along Y.
fn side_profile(p: &[DVec2], size: DVec3) -> Mesh {
    let mut m = extrude(&[vec![p.to_vec()]], -0.5, 0.5);
    for v in &mut m.verts {
        *v = DVec3::new(v.x, v.z, v.y);
    }
    m.flip();
    unit(m, size)
}

pub fn rounded_box(size: DVec3, radius: f64, steps: u32) -> Mesh {
    let h = size * 0.5;
    let r = radius.clamp(0.0, h.min_element() * 0.999);
    let coords = |half: f64| -> Vec<f64> {
        if r <= 1e-9 {
            return vec![-half, half];
        }
        let mut c = Vec::new();
        for i in 0..=steps {
            let a = 0.5 * PI * i as f64 / steps as f64;
            c.push(-(half - r) - r * a.cos());
        }
        for i in (0..=steps).rev() {
            let a = 0.5 * PI * i as f64 / steps as f64;
            let v = (half - r) + r * a.cos();
            if (v - c.last().unwrap()).abs() > 1e-9 {
                c.push(v);
            }
        }
        c
    };
    let cx = coords(h.x);
    let cy = coords(h.y);
    let cz = coords(h.z);
    let mut m = Mesh::new();
    // 6 faces: axis, sign
    for axis in 0..3 {
        for &s in &[-1.0f64, 1.0] {
            let (u_ax, v_ax) = match axis { 0 => (1, 2), 1 => (2, 0), _ => (0, 1) };
            let cu = [&cx, &cy, &cz][u_ax];
            let cv = [&cx, &cy, &cz][v_ax];
            let fixed = s * h[axis];
            let mut ids = vec![vec![0u32; cv.len()]; cu.len()];
            for (i, &u) in cu.iter().enumerate() {
                for (j, &v) in cv.iter().enumerate() {
                    let mut p = DVec3::ZERO;
                    p[axis] = fixed;
                    p[u_ax] = u;
                    p[v_ax] = v;
                    ids[i][j] = m.add_vert(p);
                }
            }
            for i in 0..cu.len() - 1 {
                for j in 0..cv.len() - 1 {
                    if s > 0.0 {
                        m.add_quad(ids[i][j], ids[i + 1][j], ids[i + 1][j + 1], ids[i][j + 1]);
                    } else {
                        m.add_quad(ids[i][j], ids[i][j + 1], ids[i + 1][j + 1], ids[i + 1][j]);
                    }
                }
            }
        }
    }
    m.weld(1e-9);
    if r > 1e-9 {
        let inner = h - DVec3::splat(r);
        for v in &mut m.verts {
            let c = v.clamp(-inner, inner);
            let d = *v - c;
            if d.length() > 1e-12 {
                *v = c + d.normalize() * r;
            }
        }
    }
    m.translate(DVec3::new(0.0, 0.0, h.z));
    m
}

fn bevel_cylinder(size: DVec3, bevel: f64, sides: u32) -> Mesh {
    let r = 0.5;
    let b = (bevel / size.z.max(1e-9)).min(0.5);
    let bx = (bevel / size.x.min(size.y).max(1e-9)).min(0.5);
    let prof = [(0.0, 0.0), (r - bx, 0.0), (r, b), (r, 1.0 - b), (r - bx, 1.0), (0.0, 1.0)];
    unit(revolve(&prof, sides), size)
}

fn tube(size: DVec3, wall: f64, sides: u32) -> Mesh {
    let n = sides as usize;
    let (rx, ry) = (size.x * 0.5, size.y * 0.5);
    let w = wall.clamp(0.01, rx.min(ry) * 0.99);
    let mut m = Mesh::new();
    let ring = |sx: f64, sy: f64, z: f64, m: &mut Mesh| -> Vec<u32> {
        (0..n)
            .map(|i| {
                let a = TAU * i as f64 / n as f64;
                m.add_vert(DVec3::new(sx * a.cos(), sy * a.sin(), z))
            })
            .collect()
    };
    let ob = ring(rx, ry, 0.0, &mut m);
    let ot = ring(rx, ry, size.z, &mut m);
    let ib = ring(rx - w, ry - w, 0.0, &mut m);
    let it = ring(rx - w, ry - w, size.z, &mut m);
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_quad(ob[i], ob[j], ot[j], ot[i]);
        m.add_quad(ib[j], ib[i], it[i], it[j]);
        m.add_quad(ot[i], ot[j], it[j], it[i]);
        m.add_quad(ob[j], ob[i], ib[i], ib[j]);
    }
    m
}

// ---------------------------------------------------------------- text

struct Outline {
    contours: Vec<Vec<DVec2>>,
    cur: Vec<DVec2>,
    off: DVec2,
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.cur.len() > 2 {
            self.contours.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
        self.cur.push(DVec2::new(x as f64, y as f64) + self.off);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cur.push(DVec2::new(x as f64, y as f64) + self.off);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = *self.cur.last().unwrap();
        let p1 = DVec2::new(x1 as f64, y1 as f64) + self.off;
        let p2 = DVec2::new(x as f64, y as f64) + self.off;
        for i in 1..=6 {
            let t = i as f64 / 6.0;
            self.cur.push(p0 * (1.0 - t) * (1.0 - t) + p1 * 2.0 * t * (1.0 - t) + p2 * t * t);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = *self.cur.last().unwrap();
        let p1 = DVec2::new(x1 as f64, y1 as f64) + self.off;
        let p2 = DVec2::new(x2 as f64, y2 as f64) + self.off;
        let p3 = DVec2::new(x as f64, y as f64) + self.off;
        for i in 1..=8 {
            let t = i as f64 / 8.0;
            let u = 1.0 - t;
            self.cur.push(p0 * u * u * u + p1 * 3.0 * u * u * t + p2 * 3.0 * u * t * t + p3 * t * t * t);
        }
    }
    fn close(&mut self) {
        if self.cur.len() > 2 {
            self.contours.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
    }
}

pub fn point_in_poly(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Returns polygon groups (outer + holes) for the text, in font units.
pub fn text_polygons(text: &str, font: FontChoice, spacing: f64) -> Vec<Vec<Vec<DVec2>>> {
    let data = match font {
        FontChoice::Sans => epaint_default_fonts::UBUNTU_LIGHT,
        FontChoice::Mono => epaint_default_fonts::HACK_REGULAR,
    };
    let Ok(face) = ttf_parser::Face::parse(data, 0) else { return vec![] };
    let upm = face.units_per_em() as f64;
    let line_h = (face.ascender() - face.descender() + face.line_gap()) as f64;
    let mut contours: Vec<Vec<DVec2>> = Vec::new();
    for (li, line) in text.lines().enumerate() {
        let mut x = 0.0;
        for ch in line.chars() {
            let Some(gid) = face.glyph_index(ch) else { continue };
            let mut ob = Outline { contours: vec![], cur: vec![], off: DVec2::new(x, -(li as f64) * line_h) };
            face.outline_glyph(gid, &mut ob);
            ttf_parser::OutlineBuilder::close(&mut ob);
            for mut c in ob.contours {
                c.dedup_by(|a, b| a.distance(*b) < 1e-6);
                if c.len() >= 3 && signed_area(&c).abs() > 1e-6 {
                    contours.push(c);
                }
            }
            x += face.glyph_hor_advance(gid).unwrap_or(0) as f64 + spacing * upm;
        }
    }
    nest_contours(contours)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(m: &Mesh) -> bool {
        let mut e = std::collections::HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                *e.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        e.iter().all(|(&(a, b), &n)| n == 1 && e.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn all_shapes_watertight_and_sized() {
        for info in LIBRARY {
            let s = (info.shape)();
            let size = DVec3::from(info.default_size);
            let m = s.generate(size);
            assert!(closed(&m), "{} not closed", info.name);
            assert!(m.volume() > 0.0, "{} volume", info.name);
            let b = m.bounds();
            assert!((b.size() - size).abs().max_element() < 1e-6, "{} size {:?}", info.name, b.size());
            assert!(b.min.z.abs() < 1e-9, "{} not on bed", info.name);
        }
    }
}

/// Group closed contours into (outer, holes...) by nesting depth (even-odd).
pub fn nest_contours(contours: Vec<Vec<DVec2>>) -> Vec<Vec<Vec<DVec2>>> {
    // nesting depth decides outer vs hole
    let depth: Vec<usize> = contours
        .iter()
        .enumerate()
        .map(|(i, c)| contours.iter().enumerate().filter(|(j, o)| *j != i && point_in_poly(c[0], o)).count())
        .collect();
    let mut groups: Vec<Vec<Vec<DVec2>>> = Vec::new();
    let mut outer_idx: Vec<usize> = Vec::new();
    for (i, c) in contours.iter().enumerate() {
        if depth[i] % 2 == 0 {
            outer_idx.push(i);
            groups.push(vec![c.clone()]);
        }
    }
    for (i, c) in contours.iter().enumerate() {
        if depth[i] % 2 == 1 {
            // innermost containing outer: the one with depth == depth[i]-1 that contains it
            if let Some(g) = outer_idx
                .iter()
                .position(|&o| depth[o] + 1 == depth[i] && point_in_poly(c[0], &contours[o]))
            {
                groups[g].push(c.clone());
            }
        }
    }
    groups
}
