//! Filament materials: Bambu Lab's own H2C presets (from the bundled template) plus a
//! personal "My Filaments" library saved between designs.

use crate::scene::Filament;
use serde_json::Value;
use std::sync::OnceLock;

pub struct Material {
    /// Bambu preset name without " @BBL H2C", e.g. "Bambu PETG HF".
    pub name: String,
    /// Family, e.g. "PLA", "PETG", "TPU".
    pub kind: String,
    pub density: f64,
    pub nozzle_temp: i64,
}

fn template() -> &'static Value {
    static T: OnceLock<Value> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str(crate::io::BAMBU_TEMPLATE).expect("template"))
}

pub fn all() -> &'static [Material] {
    static M: OnceLock<Vec<Material>> = OnceLock::new();
    M.get_or_init(|| {
        let mut v: Vec<Material> = template()["materials"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(name, d)| Material {
                        name: name.clone(),
                        kind: d["type"].as_str().unwrap_or("PLA").to_string(),
                        density: d["density"].as_f64().unwrap_or(1.24),
                        nozzle_temp: d["nozzle_temp"].as_i64().unwrap_or(220),
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Bambu first, then generic; PLA family first within each
        let rank = |m: &Material| (m.name.starts_with("Generic") as u8, family_rank(&m.kind));
        v.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.name.cmp(&b.name)));
        v
    })
}

fn family_rank(k: &str) -> u8 {
    match k {
        "PLA" => 0,
        "PLA-CF" => 1,
        "PETG" | "PCTG" => 2,
        "PETG-CF" => 3,
        "ABS" | "ASA" => 4,
        "PC" => 5,
        k if k.starts_with("TPU") => 6,
        _ => 7,
    }
}

/// The preset a filament's material string refers to. Older designs just said "PLA".
pub fn canonical(material: &str) -> &'static Material {
    let list = all();
    if let Some(m) = list.iter().find(|m| m.name.eq_ignore_ascii_case(material)) {
        return m;
    }
    let u = material.to_ascii_uppercase();
    let want = if u.contains("TPU") {
        "Bambu TPU 95A HF"
    } else if u.contains("PETG") {
        "Bambu PETG HF"
    } else if u.contains("ASA") {
        "Bambu ASA"
    } else if u.contains("ABS") {
        "Bambu ABS"
    } else if u.contains("PC") {
        "Bambu PC"
    } else if u.contains("SILK") {
        "Bambu PLA Silk+"
    } else if u.contains("MATTE") {
        "Bambu PLA Matte"
    } else {
        "Bambu PLA Basic"
    };
    list.iter().find(|m| m.name == want).unwrap_or(&list[0])
}

/// Per-filament / per-variant overrides for one material, relative to PLA Basic.
pub fn overrides(material: &str) -> (&'static Value, &'static Value) {
    let m = canonical(material);
    let d = &template()["materials"][&m.name];
    (&d["per_filament"], &d["per_variant"])
}

pub fn short(material: &str) -> String {
    canonical(material).name.trim_start_matches("Bambu ").to_string()
}

/// Plain-English warnings about mixing materials in one print.
pub fn warnings(fils: &[&Filament]) -> Vec<String> {
    let kinds: Vec<&str> = fils.iter().map(|f| canonical(&f.material).kind.as_str()).collect();
    let has = |p: &str| kinds.iter().any(|k| k.starts_with(p));
    let mut w = vec![];
    if has("PLA") && (has("PETG") || has("PCTG")) {
        w.push("PLA and PETG barely stick to each other. Where one is printed on top of the other the part can split — fine for support interfaces, not for parts that must hold together.".into());
    }
    if has("PLA") && (has("ABS") || has("ASA") || has("PC")) {
        w.push("PLA needs a cool chamber, ABS/ASA/PC want a warm one, and they don't bond well. Avoid mixing them in one print.".into());
    }
    if has("TPU") && kinds.len() > 1 {
        w.push("Flexible TPU in a multi-colour print: use the H2C's left (direct) hotend for it, or \"TPU for AMS\". Expect slower colour changes.".into());
    }
    if has("PVA") {
        w.push("PVA is water-soluble support — keep it dry and print it with PLA.".into());
    }
    w
}

// ------------------------------------------------------------- My Filaments

fn lib_path() -> Option<std::path::PathBuf> {
    crate::settings::dir().map(|d| d.join("my_filaments.json"))
}

pub fn library() -> Vec<Filament> {
    lib_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_library(l: &[Filament]) -> std::io::Result<()> {
    let p = lib_path().ok_or_else(|| std::io::Error::other("no settings folder"))?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(l).unwrap_or_default())
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_load_and_old_names_map() {
        assert!(all().len() >= 25);
        assert_eq!(canonical("PLA").name, "Bambu PLA Basic");
        assert_eq!(canonical("petg").name, "Bambu PETG HF");
        assert_eq!(canonical("Bambu ABS").kind, "ABS");
        let (pf, _) = overrides("Bambu PETG HF");
        assert_eq!(pf["filament_settings_id"][0], "Bambu PETG HF @BBL H2C");
        assert_eq!(parse_hex("#C8102E"), Some([0xC8, 0x10, 0x2E]));
    }
}
