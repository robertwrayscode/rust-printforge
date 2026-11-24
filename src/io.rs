//! File formats: Bambu Studio project 3MF (colour-ready for the H2C), generic colour 3MF,
//! STL, OBJ (+MTL), and PrintForge project files.

use crate::mesh::{Mesh, NO_COLOR};
use crate::scene::{Document, Filament, MeshCache};
use glam::{DMat4, DVec3};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Read, Seek, Write};
use std::path::Path;

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub struct ExportObject {
    pub name: String,
    /// World-space mesh of all parts; colours are filament slots (0-based into doc.filaments).
    pub mesh: Mesh,
    /// The separate parts (more than one when parts are stacked on each other).
    pub parts: Vec<(String, Mesh)>,
}

pub fn collect_export(doc: &Document, cache: &mut MeshCache) -> Vec<ExportObject> {
    let items: Vec<(String, Mesh)> = doc
        .printable_meshes(cache)
        .into_iter()
        .map(|(i, mut m)| {
            m.fill_unset_color(doc.objects[i].color);
            (doc.objects[i].name.clone(), m)
        })
        .collect();
    // Slicers drop every object onto the plate, so parts stacked on each other are
    // exported as one object with several parts.
    let bounds: Vec<crate::mesh::Aabb> = items.iter().map(|x| x.1.bounds()).collect();
    let groups = crate::check::stack_groups(&bounds);
    let mut order: Vec<usize> = Vec::new();
    let mut by_group: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (i, &g) in groups.iter().enumerate() {
        if !by_group.contains_key(&g) {
            order.push(g);
        }
        by_group.entry(g).or_default().push(i);
    }
    order
        .into_iter()
        .map(|g| {
            let mut members = by_group[&g].clone();
            members.sort_by(|&a, &b| bounds[a].min.z.partial_cmp(&bounds[b].min.z).unwrap());
            let mut mesh = Mesh::new();
            let mut parts = Vec::new();
            for &i in &members {
                mesh.append(&items[i].1);
                parts.push(items[i].clone());
            }
            let name = if members.len() > 1 {
                members.iter().map(|&i| items[i].0.as_str()).collect::<Vec<_>>().join(" + ")
            } else {
                items[members[0]].0.clone()
            };
            ExportObject { name, mesh, parts }
        })
        .collect()
}

/// Bambu's per-triangle paint encoding (TriangleSelector::serialize, leaf only).
pub fn bambu_paint_code(extruder: u32) -> String {
    let mut n = extruder as i64;
    let mut nibbles: Vec<u8> = Vec::new();
    if n < 3 {
        nibbles.push((n as u8) << 2);
    } else {
        nibbles.push(0b1100);
        n -= 3;
        while n >= 15 {
            nibbles.push(0b1111);
            n -= 15;
        }
        nibbles.push(n as u8);
    }
    nibbles.iter().rev().map(|d| format!("{:X}", d)).collect()
}

pub fn decode_bambu_paint(s: &str) -> Option<u32> {
    let digits: Vec<u8> = s.chars().rev().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let first = *digits.first()?;
    if first & 0b11 != 0 {
        return None; // split triangle: not a whole-face colour
    }
    let st = first >> 2;
    if st < 3 {
        return Some(st as u32);
    }
    let mut n = 3u32;
    for d in &digits[1..] {
        n += *d as u32;
        if *d != 15 {
            break;
        }
    }
    Some(n)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fmt_f(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// Remap used filament slots to 1-based extruder numbers.
fn filament_map(objs: &[ExportObject]) -> BTreeMap<u8, u32> {
    let mut used: Vec<u8> = objs.iter().flat_map(|o| o.mesh.used_colors()).collect();
    used.sort_unstable();
    used.dedup();
    if used.is_empty() {
        used.push(0);
    }
    used.into_iter().enumerate().map(|(i, s)| (s, i as u32 + 1)).collect()
}

fn dominant(m: &Mesh) -> u8 {
    let mut area: HashMap<u8, f64> = HashMap::new();
    for t in 0..m.tris.len() {
        *area.entry(m.colors[t]).or_default() += m.tri_normal_area(t).1;
    }
    area.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|x| x.0).unwrap_or(0)
}

const BAMBU_TEMPLATE: &str = include_str!("../assets/bambu_h2c_template.json");
const BED_CENTER: [f64; 2] = [165.0, 160.0];

/// Very rough flush estimate (mm³) from colour lightness difference; Bambu recalculates
/// on demand, this just gives sensible starting values.
fn flush_volume(from: [u8; 3], to: [u8; 3]) -> f64 {
    let lum = |c: [u8; 3]| 0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64;
    let d = (lum(from) - lum(to)).max(0.0) / 255.0;
    let hue = ((from[0] as f64 - to[0] as f64).abs() + (from[1] as f64 - to[1] as f64).abs() + (from[2] as f64 - to[2] as f64).abs()) / 765.0;
    (107.0 + 450.0 * d + 250.0 * hue).round()
}

pub fn bambu_project_config(fils: &[&Filament]) -> Value {
    let t: Value = serde_json::from_str(BAMBU_TEMPLATE).expect("template");
    let n = fils.len().max(1);
    let variants: Vec<Value> = t["variants"].as_array().unwrap().clone();
    let nv = variants.len();
    let mut out: Map<String, Value> = t["global"].as_object().unwrap().clone();
    for (k, v) in t["per_filament"].as_object().unwrap() {
        let unit = v.as_array().cloned().unwrap_or_default();
        let first = unit.first().cloned().unwrap_or(Value::String(String::new()));
        out.insert(k.clone(), Value::Array(vec![first; n]));
    }
    for (k, v) in t["per_variant"].as_object().unwrap() {
        let mut unit = v.as_array().cloned().unwrap_or_default();
        unit.resize(nv, unit.first().cloned().unwrap_or(Value::String("nil".into())));
        let mut arr = Vec::with_capacity(n * nv);
        for _ in 0..n {
            arr.extend(unit.iter().cloned());
        }
        out.insert(k.clone(), Value::Array(arr));
    }
    let s = |x: String| Value::String(x);
    out.insert("filament_colour".into(), Value::Array(fils.iter().map(|f| s(f.hex())).collect()));
    out.insert("default_filament_colour".into(), Value::Array(vec![s(String::new()); n]));
    out.insert("filament_map".into(), Value::Array(vec![s("1".into()); n]));
    out.insert("filament_map_mode".into(), s("Auto For Flush".into()));
    out.insert(
        "filament_extruder_variant".into(),
        Value::Array((0..n).flat_map(|_| variants.iter().cloned()).collect()),
    );
    out.insert(
        "filament_self_index".into(),
        Value::Array((0..n).flat_map(|i| std::iter::repeat_n(s((i + 1).to_string()), nv)).collect()),
    );
    out.insert("different_settings_to_system".into(), Value::Array(vec![s(String::new()); n + 2]));
    let mut matrix = Vec::new();
    for _ext in 0..2 {
        for a in fils {
            for b in fils {
                let v = if std::ptr::eq(*a, *b) { 0.0 } else { flush_volume(a.color, b.color) };
                matrix.push(s(fmt_f(v)));
            }
        }
    }
    if fils.is_empty() {
        matrix = vec![s("0".into()); 2];
    }
    out.insert("flush_volumes_matrix".into(), Value::Array(matrix));
    out.insert("flush_volumes_vector".into(), Value::Array(vec![s("140".into()); 2 * n]));
    out.insert("name".into(), s("project_settings".into()));
    out.insert("from".into(), s("project".into()));
    Value::Object(out)
}

fn zip_opts() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

fn mesh_xml(m: &Mesh, offset: DVec3, colors: impl Fn(usize) -> String) -> String {
    let mut s = String::with_capacity(m.tris.len() * 64);
    s.push_str("   <mesh>\n    <vertices>\n");
    for v in &m.verts {
        let p = *v - offset;
        let _ = writeln!(s, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", fmt_f(p.x), fmt_f(p.y), fmt_f(p.z));
    }
    s.push_str("    </vertices>\n    <triangles>\n");
    for (i, t) in m.tris.iter().enumerate() {
        let _ = writeln!(s, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"{}/>", t[0], t[1], t[2], colors(i));
    }
    s.push_str("    </triangles>\n   </mesh>\n");
    s
}

/// Write a Bambu Studio project for the H2C with filament colours and per-face colour
/// assignment, so it opens ready to slice and print.
pub fn write_bambu_3mf(path: &Path, doc: &Document, objs: &[ExportObject]) -> Res<()> {
    let fmap = filament_map(objs);
    let fils: Vec<&Filament> = fmap.keys().map(|&s| &doc.filaments[(s as usize).min(doc.filaments.len() - 1)]).collect();
    let cfg = bambu_project_config(&fils);
    let version = cfg["version"].as_str().unwrap_or("02.00.02.01").to_string();
    let f = std::fs::File::create(path).map_err(err)?;
    let mut z = zip::ZipWriter::new(f);
    let o = zip_opts();
    let w = |z: &mut zip::ZipWriter<std::fs::File>, name: &str, data: &[u8]| -> Res<()> {
        z.start_file(name, o).map_err(err)?;
        z.write_all(data).map_err(err)
    };
    w(&mut z, "[Content_Types].xml", br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="png" ContentType="image/png"/>
 <Default Extension="gcode" ContentType="text/x.gcode"/>
</Types>"#)?;
    w(&mut z, "_rels/.rels", br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#)?;

    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n");
    let mut main_objs = String::new();
    let mut build = String::new();
    let mut settings = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    let mut plate = String::from("  <plate>\n    <metadata key=\"plater_id\" value=\"1\"/>\n    <metadata key=\"plater_name\" value=\"\"/>\n    <metadata key=\"locked\" value=\"false\"/>\n    <metadata key=\"filament_map_mode\" value=\"Auto For Flush\"/>\n");
    // volume ids: each part is its own mesh object inside the object's sub-model file
    let mut next_vol = 1usize;
    let vol_ids: Vec<Vec<usize>> = objs
        .iter()
        .map(|ob| {
            ob.parts
                .iter()
                .map(|_| {
                    next_vol += 1;
                    next_vol - 1
                })
                .collect()
        })
        .collect();
    let first_main = next_vol;
    for (k, ob) in objs.iter().enumerate() {
        let sub_file = k + 1;
        let main_id = first_main + k;
        let b = ob.mesh.bounds();
        let off = DVec3::new(b.center().x, b.center().y, b.min.z);
        let obj_dom_ext = fmap[&dominant(&ob.mesh)];
        let mut sub = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\" xmlns:p=\"http://schemas.microsoft.com/3dmanufacturing/production/2015/06\" requiredextensions=\"p\">\n <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources>\n");
        let mut comps = String::new();
        let mut part_cfg = String::new();
        for (pi, (pname, pm)) in ob.parts.iter().enumerate() {
            let vid = vol_ids[k][pi];
            let dom_ext = fmap[&dominant(pm)];
            let colors = |i: usize| {
                let e = fmap[&pm.colors[i]];
                if e == dom_ext { String::new() } else { format!(" paint_color=\"{}\"", bambu_paint_code(e)) }
            };
            let _ = writeln!(sub, "  <object id=\"{vid}\" p:UUID=\"{:08x}-81cb-4c03-9d28-80fed5dfa1dc\" type=\"model\">", 0x10000 * vid);
            sub.push_str(&mesh_xml(pm, off, colors));
            sub.push_str("  </object>\n");
            let _ = writeln!(
                comps,
                "    <component p:path=\"/3D/Objects/object_{sub_file}.model\" objectid=\"{vid}\" p:UUID=\"{:08x}-b206-40ff-9872-83e8017abed1\" transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>",
                (main_id << 16) + pi
            );
            let pn = xml_escape(pname);
            let _ = write!(
                part_cfg,
                "    <part id=\"{vid}\" subtype=\"normal_part\">\n      <metadata key=\"name\" value=\"{pn}\"/>\n      <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n      <metadata key=\"source_object_id\" value=\"{k}\"/>\n      <metadata key=\"source_volume_id\" value=\"{pi}\"/>\n      <metadata key=\"extruder\" value=\"{dom_ext}\"/>\n      <mesh_stat face_count=\"{fc}\" edges_fixed=\"0\" degenerate_facets=\"0\" facets_removed=\"0\" facets_reversed=\"0\" backwards_edges=\"0\"/>\n    </part>\n",
                fc = pm.tris.len()
            );
        }
        sub.push_str(" </resources>\n <build/>\n</model>\n");
        w(&mut z, &format!("3D/Objects/object_{sub_file}.model"), sub.as_bytes())?;
        let _ = writeln!(rels, " <Relationship Target=\"/3D/Objects/object_{sub_file}.model\" Id=\"rel-{sub_file}\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>");
        let _ = writeln!(
            main_objs,
            "  <object id=\"{main_id}\" p:UUID=\"{:08x}-61cb-4c03-9d28-80fed5dfa1dc\" type=\"model\">\n   <components>\n{comps}   </components>\n  </object>",
            main_id
        );
        let _ = writeln!(
            build,
            "  <item objectid=\"{main_id}\" p:UUID=\"{:08x}-b1ec-4553-aec9-835e5b724bb4\" transform=\"1 0 0 0 1 0 0 0 1 {} {} {}\" printable=\"1\"/>",
            main_id,
            fmt_f(off.x + BED_CENTER[0]),
            fmt_f(off.y + BED_CENTER[1]),
            fmt_f(off.z)
        );
        let name = xml_escape(&ob.name);
        let fc: usize = ob.parts.iter().map(|p| p.1.tris.len()).sum();
        let _ = write!(
            settings,
            "  <object id=\"{main_id}\">\n    <metadata key=\"name\" value=\"{name}\"/>\n    <metadata key=\"extruder\" value=\"{obj_dom_ext}\"/>\n    <metadata face_count=\"{fc}\"/>\n{part_cfg}  </object>\n"
        );
        let _ = write!(
            plate,
            "    <model_instance>\n      <metadata key=\"object_id\" value=\"{main_id}\"/>\n      <metadata key=\"instance_id\" value=\"0\"/>\n      <metadata key=\"identify_id\" value=\"{}\"/>\n    </model_instance>\n",
            100 + k
        );
    }
    rels.push_str("</Relationships>\n");
    plate.push_str("  </plate>\n");
    settings.push_str(&plate);
    settings.push_str("</config>\n");
    let date = "2026-01-01";
    let main = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\" xmlns:p=\"http://schemas.microsoft.com/3dmanufacturing/production/2015/06\" requiredextensions=\"p\">\n <metadata name=\"Application\">BambuStudio-{version}</metadata>\n <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <metadata name=\"CreationDate\">{date}</metadata>\n <metadata name=\"Designer\">PrintForge</metadata>\n <metadata name=\"Title\"></metadata>\n <resources>\n{main_objs} </resources>\n <build p:UUID=\"2c7c17d8-22b5-4d84-8835-1976022ea369\">\n{build} </build>\n</model>\n"
    );
    w(&mut z, "3D/3dmodel.model", main.as_bytes())?;
    w(&mut z, "3D/_rels/3dmodel.model.rels", rels.as_bytes())?;
    w(&mut z, "Metadata/model_settings.config", settings.as_bytes())?;
    w(&mut z, "Metadata/project_settings.config", serde_json::to_string_pretty(&cfg).map_err(err)?.as_bytes())?;
    w(&mut z, "Metadata/slice_info.config", format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n  <header>\n    <header_item key=\"X-BBL-Client-Type\" value=\"slicer\"/>\n    <header_item key=\"X-BBL-Client-Version\" value=\"{version}\"/>\n  </header>\n</config>\n").as_bytes())?;
    z.finish().map_err(err)?;
    Ok(())
}

/// Standard 3MF with the Materials extension colours (PrusaSlicer, OrcaSlicer, Cura,
/// Windows 3D Viewer, and Bambu Studio's colour-import dialog all read this).
pub fn write_generic_3mf(path: &Path, doc: &Document, objs: &[ExportObject]) -> Res<()> {
    let fmap = filament_map(objs);
    let f = std::fs::File::create(path).map_err(err)?;
    let mut z = zip::ZipWriter::new(f);
    let o = zip_opts();
    z.start_file("[Content_Types].xml", o).map_err(err)?;
    z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>"#).map_err(err)?;
    z.start_file("_rels/.rels", o).map_err(err)?;
    z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#).map_err(err)?;
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n <metadata name=\"Application\">PrintForge</metadata>\n <resources>\n  <basematerials id=\"1\">\n");
    let order: Vec<u8> = fmap.keys().copied().collect();
    for &slot in &order {
        let fl = &doc.filaments[(slot as usize).min(doc.filaments.len() - 1)];
        let _ = writeln!(s, "   <base name=\"{}\" displaycolor=\"{}FF\"/>", xml_escape(&fl.name), fl.hex());
    }
    s.push_str("  </basematerials>\n");
    let idx_of: HashMap<u8, usize> = order.iter().enumerate().map(|(i, &c)| (c, i)).collect();
    for (k, ob) in objs.iter().enumerate() {
        let dom = idx_of[&dominant(&ob.mesh)];
        let _ = writeln!(s, "  <object id=\"{}\" name=\"{}\" type=\"model\" pid=\"1\" pindex=\"{}\">", k + 2, xml_escape(&ob.name), dom);
        let b = ob.mesh.bounds();
        let off = DVec3::new(b.center().x, b.center().y, b.min.z);
        s.push_str(&mesh_xml(&ob.mesh, off, |i| format!(" pid=\"1\" p1=\"{}\"", idx_of[&ob.mesh.colors[i]])));
        s.push_str("  </object>\n");
    }
    s.push_str(" </resources>\n <build>\n");
    for (k, ob) in objs.iter().enumerate() {
        let b = ob.mesh.bounds();
        let _ = writeln!(
            s,
            "  <item objectid=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 {} {} {}\"/>",
            k + 2,
            fmt_f(b.center().x + BED_CENTER[0]),
            fmt_f(b.center().y + BED_CENTER[1]),
            fmt_f(b.min.z)
        );
    }
    s.push_str(" </build>\n</model>\n");
    z.start_file("3D/3dmodel.model", o).map_err(err)?;
    z.write_all(s.as_bytes()).map_err(err)?;
    z.finish().map_err(err)?;
    Ok(())
}

pub fn write_stl(path: &Path, objs: &[ExportObject]) -> Res<()> {
    let total: usize = objs.iter().map(|o| o.mesh.tris.len()).sum();
    let mut buf = Vec::with_capacity(84 + total * 50);
    let mut hdr = [0u8; 80];
    let h = b"PrintForge binary STL";
    hdr[..h.len()].copy_from_slice(h);
    buf.extend_from_slice(&hdr);
    buf.extend_from_slice(&(total as u32).to_le_bytes());
    for o in objs {
        for t in 0..o.mesh.tris.len() {
            let (n, _) = o.mesh.tri_normal_area(t);
            for v in std::iter::once(n).chain(o.mesh.tri_points(t)) {
                for c in v.to_array() {
                    buf.extend_from_slice(&(c as f32).to_le_bytes());
                }
            }
            buf.extend_from_slice(&[0, 0]);
        }
    }
    std::fs::write(path, buf).map_err(err)
}

pub fn write_obj(path: &Path, doc: &Document, objs: &[ExportObject]) -> Res<()> {
    let mtl_path = path.with_extension("mtl");
    let mtl_name = mtl_path.file_name().unwrap().to_string_lossy().to_string();
    let mut mtl = String::new();
    for (i, f) in doc.filaments.iter().enumerate() {
        let _ = writeln!(
            mtl,
            "newmtl filament_{}\nKd {:.4} {:.4} {:.4}\n",
            i + 1,
            f.color[0] as f64 / 255.0,
            f.color[1] as f64 / 255.0,
            f.color[2] as f64 / 255.0
        );
    }
    let mut s = format!("# PrintForge\nmtllib {mtl_name}\n");
    let mut base = 1;
    for o in objs {
        let _ = writeln!(s, "o {}", o.name.replace(' ', "_"));
        for v in &o.mesh.verts {
            let _ = writeln!(s, "v {} {} {}", fmt_f(v.x), fmt_f(v.y), fmt_f(v.z));
        }
        let mut cur = NO_COLOR;
        for (t, &c) in o.mesh.tris.iter().zip(&o.mesh.colors) {
            if c != cur {
                let _ = writeln!(s, "usemtl filament_{}", c as usize + 1);
                cur = c;
            }
            let _ = writeln!(s, "f {} {} {}", t[0] as usize + base, t[1] as usize + base, t[2] as usize + base);
        }
        base += o.mesh.verts.len();
    }
    std::fs::write(&mtl_path, mtl).map_err(err)?;
    std::fs::write(path, s).map_err(err)
}

// ------------------------------------------------------------------ import

/// An imported object: mesh with colours as indices into `colors` (RGB) or NO_COLOR.
pub struct Imported {
    pub name: String,
    pub mesh: Mesh,
}

pub struct ImportResult {
    pub objects: Vec<Imported>,
    pub colors: Vec<[u8; 3]>,
}

pub fn read_stl(data: &[u8]) -> Res<Mesh> {
    let mut m = Mesh::new();
    let is_ascii = data.starts_with(b"solid") && {
        let head = String::from_utf8_lossy(&data[..data.len().min(1024)]);
        head.contains("facet")
    };
    if is_ascii {
        let text = String::from_utf8_lossy(data);
        let mut cur: Vec<DVec3> = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next() == Some("vertex") {
                let v: Vec<f64> = it.filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    cur.push(DVec3::new(v[0], v[1], v[2]));
                }
                if cur.len() == 3 {
                    let a = m.add_vert(cur[0]);
                    let b = m.add_vert(cur[1]);
                    let c = m.add_vert(cur[2]);
                    m.add_tri(a, b, c);
                    cur.clear();
                }
            }
        }
    } else {
        if data.len() < 84 {
            return Err("STL file is too short".into());
        }
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        if data.len() < 84 + n * 50 {
            return Err("STL file is truncated".into());
        }
        for i in 0..n {
            let o = 84 + i * 50 + 12;
            let f = |k: usize| f32::from_le_bytes(data[o + k * 4..o + k * 4 + 4].try_into().unwrap()) as f64;
            let a = m.add_vert(DVec3::new(f(0), f(1), f(2)));
            let b = m.add_vert(DVec3::new(f(3), f(4), f(5)));
            let c = m.add_vert(DVec3::new(f(6), f(7), f(8)));
            m.add_tri(a, b, c);
        }
    }
    m.weld(1e-5);
    Ok(m)
}

pub fn read_obj(path: &Path) -> Res<ImportResult> {
    let text = std::fs::read_to_string(path).map_err(err)?;
    let mut mtl_colors: HashMap<String, [u8; 3]> = HashMap::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("mtllib ") {
            if let Ok(mt) = std::fs::read_to_string(path.with_file_name(rest.trim())) {
                let mut cur = String::new();
                for l in mt.lines() {
                    let p: Vec<&str> = l.split_whitespace().collect();
                    match p.as_slice() {
                        ["newmtl", name, ..] => cur = name.to_string(),
                        ["Kd", r, g, b, ..] => {
                            let c = |x: &str| (x.parse::<f64>().unwrap_or(0.8).clamp(0.0, 1.0) * 255.0).round() as u8;
                            mtl_colors.insert(cur.clone(), [c(r), c(g), c(b)]);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let mut m = Mesh::new();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut cur = NO_COLOR;
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p.first().copied() {
            Some("v") if p.len() >= 4 => {
                let f = |i: usize| p[i].parse::<f64>().unwrap_or(0.0);
                m.add_vert(DVec3::new(f(1), f(2), f(3)));
            }
            Some("usemtl") if p.len() >= 2 => {
                cur = match mtl_colors.get(p[1]) {
                    Some(c) => {
                        let idx = colors.iter().position(|x| x == c).unwrap_or_else(|| {
                            colors.push(*c);
                            colors.len() - 1
                        });
                        idx.min(250) as u8
                    }
                    None => NO_COLOR,
                };
            }
            Some("f") if p.len() >= 4 => {
                let nv = m.verts.len() as i64;
                let idx: Vec<u32> = p[1..]
                    .iter()
                    .filter_map(|s| s.split('/').next()?.parse::<i64>().ok())
                    .map(|i| if i < 0 { (nv + i) as u32 } else { (i - 1) as u32 })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    m.add_tri(idx[0], idx[k], idx[k + 1]);
                    *m.colors.last_mut().unwrap() = cur;
                }
            }
            _ => {}
        }
    }
    m.weld(1e-5);
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok(ImportResult { objects: vec![Imported { name, mesh: m }], colors })
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

fn parse_transform(s: &str) -> DMat4 {
    let v: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if v.len() != 12 {
        return DMat4::IDENTITY;
    }
    DMat4::from_cols_array(&[v[0], v[1], v[2], 0.0, v[3], v[4], v[5], 0.0, v[6], v[7], v[8], 0.0, v[9], v[10], v[11], 1.0])
}

#[derive(Default, Clone)]
struct ModelObj {
    name: String,
    mesh: Option<Mesh>,
    tri_color: Vec<Option<[u8; 3]>>,
    tri_paint: Vec<Option<u32>>,
    obj_color: Option<[u8; 3]>,
    components: Vec<(String, String, DMat4)>, // (path, id, transform)
}

#[derive(Default)]
struct ModelFile {
    objects: HashMap<String, ModelObj>,
    groups: HashMap<String, Vec<[u8; 3]>>,
    build: Vec<(String, DMat4)>,
}

fn parse_model(xml: &str, cur_path: &str) -> Res<ModelFile> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut mf = ModelFile::default();
    let mut cur_obj: Option<(String, ModelObj)> = None;
    let mut cur_group: Option<String> = None;
    let attrs = |e: &quick_xml::events::BytesStart| -> HashMap<String, String> {
        e.attributes()
            .flatten()
            .map(|a| {
                let k = a.key.as_ref().to_string();
                let k = k.rsplit(':').next().unwrap().to_string();
                (k, a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.to_string()).unwrap_or_default())
            })
            .collect()
    };
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                let a = attrs(&e);
                match name.as_str() {
                    "object" => {
                        let mut o = ModelObj { name: a.get("name").cloned().unwrap_or_default(), ..Default::default() };
                        if let (Some(pid), Some(pi)) = (a.get("pid"), a.get("pindex")) {
                            o.obj_color = mf.groups.get(pid).and_then(|g| g.get(pi.parse::<usize>().unwrap_or(0)).copied());
                        } else if let Some(pid) = a.get("pid") {
                            o.obj_color = mf.groups.get(pid).and_then(|g| g.first().copied());
                        }
                        cur_obj = Some((a.get("id").cloned().unwrap_or_default(), o));
                    }
                    "basematerials" | "colorgroup" => {
                        let id = a.get("id").cloned().unwrap_or_default();
                        mf.groups.insert(id.clone(), vec![]);
                        cur_group = Some(id);
                    }
                    "base" | "color" => {
                        if let Some(g) = &cur_group {
                            let c = a.get("displaycolor").or(a.get("color")).and_then(|s| parse_color(s)).unwrap_or([200, 200, 200]);
                            mf.groups.get_mut(g).unwrap().push(c);
                        }
                    }
                    "mesh" => {
                        if let Some((_, o)) = &mut cur_obj {
                            o.mesh = Some(Mesh::new());
                        }
                    }
                    "vertex" => {
                        if let Some((_, o)) = &mut cur_obj {
                            let f = |k: &str| a.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                            o.mesh.as_mut().unwrap().add_vert(DVec3::new(f("x"), f("y"), f("z")));
                        }
                    }
                    "triangle" => {
                        if let Some((_, o)) = &mut cur_obj {
                            let g = |k: &str| a.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                            o.mesh.as_mut().unwrap().add_tri(g("v1"), g("v2"), g("v3"));
                            let col = a.get("pid").and_then(|pid| {
                                let idx = a.get("p1").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                                mf.groups.get(pid).and_then(|g| g.get(idx).copied())
                            });
                            o.tri_color.push(col);
                            o.tri_paint.push(
                                a.get("paint_color").or(a.get("mmu_segmentation")).and_then(|s| decode_bambu_paint(s)).filter(|&n| n > 0),
                            );
                        }
                    }
                    "component" => {
                        if let Some((_, o)) = &mut cur_obj {
                            let path = a.get("path").cloned().unwrap_or_else(|| cur_path.to_string());
                            o.components.push((
                                path,
                                a.get("objectid").cloned().unwrap_or_default(),
                                a.get("transform").map(|t| parse_transform(t)).unwrap_or(DMat4::IDENTITY),
                            ));
                        }
                    }
                    "item" => {
                        mf.build.push((
                            a.get("objectid").cloned().unwrap_or_default(),
                            a.get("transform").map(|t| parse_transform(t)).unwrap_or(DMat4::IDENTITY),
                        ));
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "object" {
                    if let Some((id, o)) = cur_obj.take() {
                        mf.objects.insert(id, o);
                    }
                } else if name == "basematerials" || name == "colorgroup" {
                    cur_group = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("3MF XML error: {e}")),
            _ => {}
        }
    }
    Ok(mf)
}

pub fn read_3mf<R: Read + Seek>(reader: R) -> Res<ImportResult> {
    let mut z = zip::ZipArchive::new(reader).map_err(err)?;
    let mut files: HashMap<String, ModelFile> = HashMap::new();
    let names: Vec<String> = z.file_names().map(|s| s.to_string()).collect();
    let mut filament_colors: Vec<[u8; 3]> = Vec::new();
    let mut obj_settings: HashMap<String, (String, Option<u32>)> = HashMap::new();
    let mut part_ext: HashMap<String, u32> = HashMap::new();
    for n in &names {
        if n.ends_with(".model") {
            let mut s = String::new();
            z.by_name(n).map_err(err)?.read_to_string(&mut s).map_err(err)?;
            let key = format!("/{}", n.trim_start_matches('/'));
            files.insert(key.clone(), parse_model(&s, &key)?);
        }
        if n.ends_with("model_settings.config") {
            let mut s = String::new();
            z.by_name(n).map_err(err)?.read_to_string(&mut s).map_err(err)?;
            let (o, p) = parse_model_settings_full(&s);
            obj_settings = o;
            part_ext = p;
        }
        if n.ends_with("project_settings.config") {
            let mut s = String::new();
            z.by_name(n).map_err(err)?.read_to_string(&mut s).map_err(err)?;
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                if let Some(arr) = v["filament_colour"].as_array() {
                    filament_colors = arr.iter().filter_map(|c| c.as_str().and_then(parse_color)).collect();
                }
            }
        }
    }
    let root_key = files.keys().find(|k| k.ends_with("3D/3dmodel.model")).cloned().ok_or("no 3D/3dmodel.model in file")?;
    let mut colors: Vec<[u8; 3]> = filament_colors.clone();
    let color_idx = |c: [u8; 3], colors: &mut Vec<[u8; 3]>| -> u8 {
        if let Some(i) = colors.iter().position(|x| *x == c) {
            return i as u8;
        }
        colors.push(c);
        (colors.len() - 1).min(250) as u8
    };
    fn gather(
        files: &HashMap<String, ModelFile>,
        path: &str,
        id: &str,
        xf: DMat4,
        inherited: Option<[u8; 3]>,
        out: &mut Vec<(Mesh, Vec<Option<[u8; 3]>>, Vec<Option<u32>>, Option<[u8; 3]>)>,
        depth: u32,
        part_col: &dyn Fn(&str) -> Option<[u8; 3]>,
    ) {
        if depth > 16 {
            return;
        }
        let Some(f) = files.get(path) else { return };
        let Some(o) = f.objects.get(id) else { return };
        let col = o.obj_color.or(inherited);
        if let Some(m) = &o.mesh {
            out.push((m.transformed(&xf), o.tri_color.clone(), o.tri_paint.clone(), col));
        }
        for (p, cid, t) in &o.components {
            gather(files, p, cid, xf * *t, part_col(cid).or(col), out, depth + 1, part_col);
        }
    }
    let root = &files[&root_key];
    let mut objects = Vec::new();
    for (i, (id, xf)) in root.build.iter().enumerate() {
        let mut parts = Vec::new();
        let ext_color = obj_settings
            .get(id)
            .and_then(|(_, e)| *e)
            .and_then(|e| filament_colors.get((e as usize).wrapping_sub(1)).copied());
        let part_col = |cid: &str| part_ext.get(cid).and_then(|&e| filament_colors.get((e as usize).wrapping_sub(1)).copied());
        gather(&files, &root_key, id, *xf, ext_color, &mut parts, 0, &part_col);
        let mut mesh = Mesh::new();
        for (mut m, tc, tp, oc) in parts {
            for t in 0..m.tris.len() {
                let c = if let Some(Some(e)) = tp.get(t) {
                    let e = *e as usize;
                    if e >= 1 && e <= filament_colors.len() { Some(filament_colors[e - 1]) } else { None }
                } else {
                    tc.get(t).copied().flatten()
                };
                m.colors[t] = match c.or(oc) {
                    Some(c) => color_idx(c, &mut colors),
                    None => NO_COLOR,
                };
            }
            mesh.append(&m);
        }
        mesh.weld(1e-6);
        let name = obj_settings
            .get(id)
            .map(|x| x.0.clone())
            .or(root.objects.get(id).map(|o| o.name.clone()))
            .filter(|s| !s.is_empty())
            .unwrap_or(format!("Object {}", i + 1));
        objects.push(Imported { name, mesh });
    }
    Ok(ImportResult { objects, colors })
}

/// object id -> (name, extruder) from Bambu's Metadata/model_settings.config
/// (object id -> (name, extruder), part/volume id -> extruder)
fn parse_model_settings_full(xml: &str) -> (HashMap<String, (String, Option<u32>)>, HashMap<String, u32>) {
    let mut parts: HashMap<String, u32> = HashMap::new();
    let mut cur_part: Option<String> = None;
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = HashMap::new();
    let mut cur: Option<String> = None;
    let mut depth_in_part = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                let get = |k: &str| {
                    e.attributes().flatten().find(|a| a.key.as_ref() == k).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.to_string()))
                };
                match name.as_str() {
                    "object" => {
                        let id = get("id").unwrap_or_default();
                        out.insert(id.clone(), (String::new(), None));
                        cur = Some(id);
                    }
                    "part" => {
                        depth_in_part = true;
                        cur_part = get("id");
                    }
                    "metadata" if depth_in_part => {
                        if let (Some(pid), Some(k), Some(v)) = (&cur_part, get("key"), get("value")) {
                            if k == "extruder" {
                                if let Ok(e) = v.parse() {
                                    parts.insert(pid.clone(), e);
                                }
                            }
                        }
                    }
                    "metadata" if !depth_in_part => {
                        if let (Some(id), Some(k), Some(v)) = (&cur, get("key"), get("value")) {
                            let ent = out.get_mut(id).unwrap();
                            if k == "name" {
                                ent.0 = v;
                            } else if k == "extruder" {
                                ent.1 = v.parse().ok();
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "part" {
                    depth_in_part = false;
                } else if name == "object" {
                    cur = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (out, parts)
}

pub fn import_file(path: &Path) -> Res<ImportResult> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "stl" => {
            let data = std::fs::read(path).map_err(err)?;
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            Ok(ImportResult { objects: vec![Imported { name, mesh: read_stl(&data)? }], colors: vec![] })
        }
        "obj" => read_obj(path),
        "3mf" => read_3mf(std::fs::File::open(path).map_err(err)?),
        _ => Err(format!("Unsupported file type: .{ext} (use STL, OBJ or 3MF)")),
    }
}

// ------------------------------------------------------------------ project files

pub fn save_project(path: &Path, doc: &Document) -> Res<()> {
    let f = std::fs::File::create(path).map_err(err)?;
    let mut z = zip::ZipWriter::new(f);
    z.start_file("document.json", zip_opts()).map_err(err)?;
    serde_json::to_writer(&mut z, doc).map_err(err)?;
    z.finish().map_err(err)?;
    Ok(())
}

pub fn load_project(path: &Path) -> Res<Document> {
    let f = std::fs::File::open(path).map_err(err)?;
    let mut z = zip::ZipArchive::new(f).map_err(err)?;
    let mut s = String::new();
    z.by_name("document.json").map_err(err)?.read_to_string(&mut s).map_err(err)?;
    serde_json::from_str(&s).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_codes_match_bambu() {
        assert_eq!(bambu_paint_code(1), "4");
        assert_eq!(bambu_paint_code(2), "8");
        assert_eq!(bambu_paint_code(3), "0C");
        assert_eq!(bambu_paint_code(4), "1C");
        assert_eq!(bambu_paint_code(16), "DC");
        assert_eq!(bambu_paint_code(18), "0FC");
        for e in 1..40 {
            assert_eq!(decode_bambu_paint(&bambu_paint_code(e)), Some(e));
        }
    }

    #[test]
    fn config_shapes() {
        let f = crate::scene::default_filaments();
        let refs: Vec<&Filament> = f.iter().take(3).collect();
        let c = bambu_project_config(&refs);
        assert_eq!(c["filament_colour"].as_array().unwrap().len(), 3);
        assert_eq!(c["filament_settings_id"].as_array().unwrap().len(), 3);
        assert_eq!(c["flush_volumes_matrix"].as_array().unwrap().len(), 18);
        assert_eq!(c["nozzle_temperature"].as_array().unwrap().len(), 9);
        assert_eq!(c["printer_settings_id"], "Bambu Lab H2C 0.4 nozzle");
    }
}
