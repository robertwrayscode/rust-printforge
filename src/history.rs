//! Version history: every time a design is saved, a copy goes into RoboForge's History
//! folder (per design), so you can look back and restore any earlier version.

use std::path::{Path, PathBuf};

const KEEP: usize = 100;

fn folder_for(project: &Path) -> Option<PathBuf> {
    let base = crate::settings::dir()?.join("History");
    let stem = project.file_stem()?.to_string_lossy().to_string();
    // keep designs with the same name in different folders apart
    let mut h: u64 = 1469598103934665603;
    for b in project.to_string_lossy().bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    let safe: String = stem.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    let d = base.join(format!("{safe}-{:08x}", h as u32));
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// Keep a copy of the file that was just saved.
pub fn record(project: &Path) {
    let Some(d) = folder_for(project) else { return };
    let name = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let _ = std::fs::copy(project, d.join(format!("{name}.pforge")));
    // trim old versions
    let mut all = list(project);
    if all.len() > KEEP {
        all.sort_by(|a, b| b.when.cmp(&a.when));
        for v in &all[KEEP..] {
            let _ = std::fs::remove_file(&v.path);
        }
    }
}

pub struct Version {
    pub path: PathBuf,
    pub when: String,
    pub objects: usize,
    pub bytes: u64,
}

/// Saved versions of a design, newest first.
pub fn list(project: &Path) -> Vec<Version> {
    let Some(d) = folder_for(project) else { return vec![] };
    let mut v: Vec<Version> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "pforge").unwrap_or(false))
        .map(|e| {
            let p = e.path();
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let when = chrono::NaiveDateTime::parse_from_str(&stem, "%Y-%m-%d_%H-%M-%S")
                .map(|t| t.format("%a %b %-d, %Y  %-I:%M:%S %p").to_string())
                .unwrap_or(stem.clone());
            let objects = crate::io::load_project(&p).map(|d| d.objects.len()).unwrap_or(0);
            Version { bytes: e.metadata().map(|m| m.len()).unwrap_or(0), path: p, when: format!("{stem}|{when}"), objects }
        })
        .collect();
    v.sort_by(|a, b| b.when.cmp(&a.when));
    for x in &mut v {
        x.when = x.when.split('|').nth(1).unwrap_or("").to_string();
    }
    v
}
