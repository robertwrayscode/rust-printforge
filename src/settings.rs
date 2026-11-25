//! Small persistent settings (theme, recent files) and the auto-save location.

use std::path::PathBuf;

pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"))?;
    let d = PathBuf::from(base).join(if cfg!(windows) { "RoboForge" } else { ".roboforge" });
    let _ = std::fs::create_dir_all(&d);
    Some(d)
}

fn file() -> Option<PathBuf> {
    dir().map(|d| d.join("settings.txt"))
}

pub fn get(key: &str) -> Vec<String> {
    let Some(f) = file() else { return vec![] };
    std::fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
        .collect()
}

pub fn set(key: &str, values: &[String]) {
    let Some(f) = file() else { return };
    let mut keep: Vec<String> = std::fs::read_to_string(&f)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once('=').map(|(k, _)| k != key).unwrap_or(false))
        .map(|s| s.to_string())
        .collect();
    keep.extend(values.iter().map(|v| format!("{key}={v}")));
    let _ = std::fs::write(f, keep.join("\n") + "\n");
}

pub fn recent() -> Vec<PathBuf> {
    get("recent").into_iter().map(PathBuf::from).filter(|p| p.exists()).collect()
}

pub fn add_recent(p: &std::path::Path) {
    let mut r: Vec<PathBuf> = recent();
    r.retain(|x| x != p);
    r.insert(0, p.to_path_buf());
    r.truncate(10);
    set("recent", &r.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
}

pub fn autosave_path() -> Option<PathBuf> {
    dir().map(|d| d.join("autosave.pforge"))
}
