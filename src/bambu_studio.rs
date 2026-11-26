//! Find Bambu Studio on this computer and open a file in it.

use std::path::{Path, PathBuf};
use std::process::Command;

const KEY: &str = "bambu_studio_path";

fn candidates() -> Vec<PathBuf> {
    let mut v = vec![];
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(var) {
                v.push(PathBuf::from(p).join("Bambu Studio").join("bambu-studio.exe"));
            }
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(&p).join("Programs").join("Bambu Studio").join("bambu-studio.exe"));
        }
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Applications/BambuStudio.app"));
        v.push(PathBuf::from("/Applications/Bambu Studio.app"));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join("Applications").join("BambuStudio.app"));
        }
    } else {
        for d in std::env::var("PATH").unwrap_or_default().split(':') {
            v.push(Path::new(d).join("bambu-studio"));
        }
        v.push(PathBuf::from("/opt/bambu-studio/bambu-studio"));
    }
    v
}

/// The Bambu Studio program, if we can find it (a path the user chose wins).
pub fn find() -> Option<PathBuf> {
    if let Some(p) = crate::settings::get(KEY).first().map(PathBuf::from) {
        if p.exists() {
            return Some(p);
        }
    }
    candidates().into_iter().find(|p| p.exists())
}

pub fn remember(p: &Path) {
    crate::settings::set(KEY, &[p.to_string_lossy().to_string()]);
}

/// Start Bambu Studio with `file`. If it's already running it opens the file in a new
/// tab/window as Bambu Studio decides.
pub fn open(app: &Path, file: &Path) -> std::io::Result<()> {
    if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        if app.extension().map(|e| e == "app").unwrap_or(false) {
            c.arg("-a").arg(app);
        } else {
            c.arg("-a").arg("BambuStudio");
        }
        c.arg(file).spawn().map(|_| ())
    } else {
        Command::new(app).arg(file).spawn().map(|_| ())
    }
}

/// Folder for files sent to Bambu Studio.
pub fn print_folder() -> PathBuf {
    let base = crate::settings::dir().unwrap_or_else(std::env::temp_dir);
    let d = base.join("Prints");
    let _ = std::fs::create_dir_all(&d);
    d
}
