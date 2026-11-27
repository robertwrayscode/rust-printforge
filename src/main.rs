#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

pub mod ai;
pub mod app;
pub mod bambu_studio;
pub mod check;
pub mod csg;
pub mod decal;
pub mod demo;
pub mod edges;
pub mod io;
pub mod materials;
pub mod mesh;
pub mod render;
pub mod scene;
pub mod shapes;
pub mod split;
pub mod surface_text;
pub mod sketch;
pub mod thumbs;
pub mod theme;
pub mod settings;
pub mod history;
pub mod imaging;
pub mod parts;
pub mod preview;

use std::path::PathBuf;

fn usage() {
    eprintln!(
        "RoboForge {}\n\nUsage:\n  roboforge [project.pforge | model.stl/.obj/.3mf]   open the editor\n  roboforge check <file>                            print a printability report\n  roboforge export <in> <out.3mf|.stl|.obj> [--generic]\n                                                     convert / export (3MF = Bambu H2C project unless --generic)\n  roboforge demo <out.pforge>                        write a sample multi-colour project",
        env!("CARGO_PKG_VERSION")
    );
}

fn load_any(path: &std::path::Path) -> Result<scene::Document, String> {
    if path.extension().map(|e| e.eq_ignore_ascii_case("pforge")).unwrap_or(false) {
        io::load_project(path)
    } else {
        let r = io::import_file(path)?;
        Ok(demo::doc_from_import(r))
    }
}

#[cfg(windows)]
fn attach_console() {
    // The release build is a GUI app; reattach to the terminal so CLI output is visible.
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if matches!(args.first().map(|s| s.as_str()), Some("check" | "export" | "demo" | "parts-sheet" | "--help" | "-h")) {
        attach_console();
    }
    match args.first().map(|s| s.as_str()) {
        Some("--help") | Some("-h") => usage(),
        Some("check") if args.len() >= 2 => {
            let doc = match load_any(std::path::Path::new(&args[1])) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let mut cache = scene::MeshCache::default();
            let input = demo::check_input(&doc, &mut cache);
            let r = check::run(&input);
            for s in &r.summary {
                println!("{s}");
            }
            for i in &r.issues {
                println!("[{:?}] {} — {} ({} faces)", i.severity, i.title, i.detail, i.tris.len());
            }
            if r.issues.iter().any(|i| i.severity == check::Severity::Error) {
                std::process::exit(1);
            }
        }
        Some("export") if args.len() >= 3 => {
            let doc = load_any(std::path::Path::new(&args[1])).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(2)
            });
            let mut cache = scene::MeshCache::default();
            let objs = io::collect_export(&doc, &mut cache);
            let out = PathBuf::from(&args[2]);
            let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let generic = args.iter().any(|a| a == "--generic");
            let r = match ext.as_str() {
                "3mf" if generic => io::write_generic_3mf(&out, &doc, &objs),
                "3mf" => io::write_bambu_3mf(&out, &doc, &objs),
                "stl" => io::write_stl(&out, &objs),
                "obj" => io::write_obj(&out, &doc, &objs),
                _ => Err("output must be .3mf, .stl or .obj".into()),
            };
            match r {
                Ok(()) => println!("wrote {}", out.display()),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            }
        }
        Some("parts-sheet") if args.len() >= 2 => {
            // every library part, each checked on its own, then saved together
            let mut all = scene::Document::default();
            let (mut x, mut y, mut row_d) = (-140.0f64, -140.0f64, 0.0f64);
            for info in shapes::LIBRARY.iter().filter(|i| i.category != "Basic shapes") {
                let size = glam::DVec3::from(info.default_size);
                let mut d = scene::Document::default();
                let id = d.alloc_id();
                d.objects.push(scene::Object::new(id, info.name, scene::Kind::Shape((info.shape)()), size));
                let mut cache = scene::MeshCache::default();
                let r = check::run(&demo::check_input(&d, &mut cache));
                for i in r.issues.iter().filter(|i| i.severity != check::Severity::Info) {
                    println!("{:<16} [{:?}] {} — {}", info.name, i.severity, i.title, i.detail);
                }
                if x + size.x > 140.0 {
                    x = -140.0;
                    y += row_d + 10.0;
                    row_d = 0.0;
                }
                let id = all.alloc_id();
                let mut o = scene::Object::new(id, info.name, scene::Kind::Shape((info.shape)()), size);
                o.pos = glam::DVec3::new(x + size.x / 2.0, y + size.y / 2.0, 0.0);
                o.color = (all.objects.len() % 4) as u8;
                all.objects.push(o);
                x += size.x + 10.0;
                row_d = row_d.max(size.y);
            }
            io::save_project(std::path::Path::new(&args[1]), &all).expect("save");
            println!("wrote {}", args[1]);
        }
        Some("demo") if args.len() >= 2 => {
            let d = demo::demo_doc();
            io::save_project(std::path::Path::new(&args[1]), &d).expect("save");
            println!("wrote {}", args[1]);
        }
        other => {
            let mut doc = scene::Document::default();
            let mut file = None;
            if let Some(p) = other {
                let p = PathBuf::from(p);
                match load_any(&p) {
                    Ok(d) => {
                        doc = d;
                        if p.extension().map(|e| e == "pforge").unwrap_or(false) {
                            file = Some(p);
                        }
                    }
                    Err(e) => eprintln!("couldn't open {}: {e}", p.display()),
                }
            }
            if std::env::var("ROBOFORGE_DEMO").is_ok() {
                doc = demo::demo_doc();
            }
            let shot = std::env::var("ROBOFORGE_SCREENSHOT").ok().map(PathBuf::from);
            // Try with anti-aliasing first; some GPUs/VMs don't offer it.
            for msaa in [4u16, 0] {
                let opts = eframe::NativeOptions {
                    viewport: eframe::egui::ViewportBuilder::default()
                        .with_title(format!("RoboForge v{} — 3D design for Bambu Lab H2C", env!("CARGO_PKG_VERSION")))
                        .with_inner_size([1400.0, 860.0])
                        .with_min_inner_size([900.0, 600.0]),
                    depth_buffer: 24,
                    multisampling: msaa,
                    ..Default::default()
                };
                let (d, f, s) = (doc.clone(), file.clone(), shot.clone());
                match eframe::run_native("RoboForge", opts, Box::new(move |cc| Ok(Box::new(app::App::new(cc, d, f, s))))) {
                    Ok(()) => break,
                    Err(e) if msaa > 0 => eprintln!("retrying without anti-aliasing ({e})"),
                    Err(e) => eprintln!("failed to start: {e}"),
                }
            }
        }
    }
}
