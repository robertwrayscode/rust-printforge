//! "My Parts": designs saved for reuse, shown in their own section of the Shapes panel.
//!
//! Each part is one JSON file in the settings folder's `My Parts` folder, holding the
//! objects (centred, sitting at z = 0) and the filaments they were coloured with, so the
//! colours can be matched to whatever filaments the next design uses.

use crate::scene::{Filament, Object};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct MyPart {
    pub name: String,
    #[serde(default)]
    pub saved: String,
    pub filaments: Vec<Filament>,
    pub objects: Vec<Object>,
    #[serde(skip)]
    pub path: PathBuf,
}

pub fn dir() -> Option<PathBuf> {
    let d = crate::settings::dir()?.join("My Parts");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "part".into() } else { s }
}

/// All saved parts, sorted by name.
pub fn list() -> Vec<MyPart> {
    let Some(d) = dir() else { return vec![] };
    let mut out: Vec<MyPart> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let mut part: MyPart = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            part.path = p;
            Some(part)
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Save a part; a part with the same name is replaced.
pub fn save(part: &MyPart) -> Result<PathBuf, String> {
    let d = dir().ok_or("no settings folder")?;
    let p = d.join(format!("{}.json", slug(&part.name)));
    let mut part = part.clone();
    if part.saved.is_empty() {
        part.saved = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    }
    std::fs::write(&p, serde_json::to_string(&part).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn delete(part: &MyPart) {
    let _ = std::fs::remove_file(&part.path);
}

pub fn rename(part: &MyPart, name: &str) -> Result<(), String> {
    let mut p = part.clone();
    p.name = name.trim().to_string();
    if p.name.is_empty() {
        return Err("The name can't be empty.".into());
    }
    let new = save(&p)?;
    if new != part.path {
        delete(part);
    }
    Ok(())
}

/// For each filament the part used, the filament number to use in a design with `fils`:
/// the same colour if it's there, otherwise the closest colour.
pub fn color_map(part: &MyPart, fils: &[Filament]) -> Vec<u8> {
    part.filaments
        .iter()
        .map(|f| {
            fils.iter()
                .enumerate()
                .min_by_key(|(_, g)| {
                    let d: i32 = (0..3).map(|k| (f.color[k] as i32 - g.color[k] as i32).pow(2)).sum();
                    // prefer the same material when colours tie
                    d * 2 + i32::from(g.material != f.material)
                })
                .map(|(i, _)| i as u8)
                .unwrap_or(0)
        })
        .collect()
}

/// Thumbnail picture of a part (RGBA, THUMB × THUMB).
pub fn thumbnail(part: &MyPart, cache: &mut crate::scene::MeshCache) -> Vec<u8> {
    let mut m = crate::mesh::Mesh::new();
    for o in part.objects.iter().filter(|o| o.visible && !o.hole) {
        let mut w = cache.world_mesh(o);
        w.fill_unset_color(o.color);
        m.append(&w);
    }
    let cols: Vec<[u8; 3]> = part.filaments.iter().map(|f| f.color).collect();
    crate::thumbs::render_mesh(&m, &|t| {
        let c = m.colors.get(t).copied().unwrap_or(0) as usize;
        let c = cols.get(c).copied().unwrap_or([180, 180, 180]);
        // keep very dark parts visible on the dark panel
        let l = c[0] as u32 + c[1] as u32 + c[2] as u32;
        if l < 120 { [70, 70, 76] } else { c }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_match_by_closeness() {
        let fils = crate::scene::default_filaments();
        let mut red = fils[2].clone();
        red.color = [200, 20, 30];
        let part = MyPart { name: "x".into(), saved: String::new(), filaments: vec![fils[0].clone(), red], objects: vec![], path: PathBuf::new() };
        let m = color_map(&part, &fils);
        assert_eq!(m[0], 0);
        assert_eq!(fils[m[1] as usize].name, fils[2].name);
    }
}
