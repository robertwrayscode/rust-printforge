//! Sample content and helpers shared by the CLI.

use crate::check::CheckInput;
use crate::io::ImportResult;
use crate::mesh::{Aabb, NO_COLOR};
use crate::scene::{Document, Filament, Kind, MeshCache, Object, normalize_mesh};
use crate::shapes::{FontChoice, Shape};
use glam::DVec3;
use std::sync::Arc;

pub fn doc_from_import(r: ImportResult) -> Document {
    let mut d = Document::default();
    let mut slot = vec![];
    for c in &r.colors {
        let i = d.filaments.iter().position(|f| f.color == *c).unwrap_or_else(|| {
            d.filaments.push(Filament { name: format!("Imported {}", d.filaments.len() + 1), color: *c, material: "PLA".into() });
            d.filaments.len() - 1
        });
        slot.push(i.min(23) as u8);
    }
    let all = r.objects.iter().fold(Aabb::empty(), |b, o| b.union(&o.mesh.bounds()));
    let shift = DVec3::new(-all.center().x, -all.center().y, -all.min.z);
    for mut ob in r.objects {
        for c in &mut ob.mesh.colors {
            if *c != NO_COLOR {
                *c = slot.get(*c as usize).copied().unwrap_or(0);
            }
        }
        ob.mesh.translate(shift);
        let (m, size, off) = normalize_mesh(ob.mesh);
        let id = d.alloc_id();
        let mut o = Object::new(id, &ob.name, Kind::Mesh(Arc::new(m)), size);
        o.pos = off;
        d.objects.push(o);
    }
    d
}

pub fn check_input(doc: &Document, cache: &mut MeshCache) -> CheckInput {
    let mut objects = vec![];
    for o in &doc.objects {
        if o.visible && !o.hole {
            let mut m = cache.world_mesh(o);
            m.fill_unset_color(o.color);
            objects.push((o.id, o.name.clone(), m));
        }
    }
    CheckInput {
        objects,
        loose_holes: doc.objects.iter().filter(|o| o.visible && o.hole).map(|o| o.name.clone()).collect(),
        hidden: doc.objects.iter().filter(|o| !o.visible).map(|o| o.name.clone()).collect(),
        csg_warnings: cache.warnings.values().flatten().cloned().collect(),
        filament_names: doc.filaments.iter().map(|f| f.name.clone()).collect(),
        printer: doc.printer.clone(),
        overhang_deg: 45.0,
    }
}

/// A multi-colour name plate + a painted cube, showing groups, holes, text and paint.
pub fn demo_doc() -> Document {
    let mut d = Document::default();
    let mut cache = MeshCache::default();
    let base_id = d.alloc_id();
    let mut base = Object::new(base_id, "Plate", Kind::Shape(Shape::Box { radius: 4.0, steps: 4 }), DVec3::new(90.0, 30.0, 4.0));
    base.color = 3;
    let text_id = d.alloc_id();
    let mut text = Object::new(
        text_id,
        "Text",
        Kind::Shape(Shape::Text { text: "RoboForge".into(), font: FontChoice::Sans, spacing: 0.0 }),
        DVec3::new(70.0, 14.0, 2.0),
    );
    text.pos = DVec3::new(0.0, 0.0, 4.0);
    text.color = 0;
    let hole_id = d.alloc_id();
    let mut hole = Object::new(hole_id, "Hanger hole", Kind::Shape(Shape::Cylinder { sides: 32, bevel: 0.0 }), DVec3::new(5.0, 5.0, 10.0));
    hole.pos = DVec3::new(-39.0, 0.0, -2.0);
    hole.hole = true;
    d.objects.extend([base, text, hole]);
    let g = d.group(&[base_id, text_id, hole_id], &mut cache).unwrap();
    if let Some(o) = d.find_mut(g) {
        o.name = "Name plate".into();
        o.pos.y = -30.0;
    }
    let cube_id = d.alloc_id();
    let mut cube = Object::new(cube_id, "Painted cube", Kind::Shape(Shape::Box { radius: 2.0, steps: 3 }), DVec3::splat(25.0));
    cube.pos = DVec3::new(-25.0, 25.0, 0.0);
    cube.color = 4;
    let m = cache.base_mesh(&cube);
    let paint: Vec<u8> = (0..m.tris.len()).map(|t| { let (n, _) = m.tri_normal_area(t); if n.z > 0.9 { 2 } else if n.x > 0.9 { 5 } else { NO_COLOR } }).collect();
    cube.paint = Some(paint);
    d.objects.push(cube);
    let star_id = d.alloc_id();
    let mut star = Object::new(star_id, "Star", Kind::Shape(Shape::Star { points: 5, inner: 0.45 }), DVec3::new(30.0, 30.0, 6.0));
    star.pos = DVec3::new(25.0, 25.0, 0.0);
    star.color = 6;
    d.objects.push(star);
    d
}
