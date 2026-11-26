//! AI helper: describe what you want, an AI model (local or cloud) writes a plan made of
//! RoboForge building blocks, and RoboForge builds it.
//!
//! The model never makes triangles itself. It returns JSON "actions" (add a shape with a
//! size/position/colour/settings, change or delete a part, group parts). RoboForge
//! validates and applies them, so whatever comes back is clean and printable.

use crate::scene::{Document, Filament, Kind, Object};
use crate::shapes::{Shape, LIBRARY};
use glam::DVec3;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    /// llama.cpp server, Ollama, LM Studio, vLLM, OpenAI…
    OpenAi,
    /// Anthropic's Claude API
    Anthropic,
}

#[derive(Clone, Debug)]
pub struct AiSettings {
    pub provider: Provider,
    pub url: String,
    pub model: String,
    pub key: String,
}

impl AiSettings {
    pub fn load() -> Self {
        let g = |k: &str, d: &str| crate::settings::get(k).first().cloned().unwrap_or_else(|| d.to_string());
        let provider = if g("ai_provider", "openai") == "anthropic" { Provider::Anthropic } else { Provider::OpenAi };
        Self { provider, url: g("ai_url", "http://localhost:8080/v1"), model: g("ai_model", ""), key: g("ai_key", "") }
    }
    pub fn save(&self) {
        let s = |k: &str, v: &str| crate::settings::set(k, &[v.to_string()]);
        s("ai_provider", if self.provider == Provider::Anthropic { "anthropic" } else { "openai" });
        s("ai_url", &self.url);
        s("ai_model", &self.model);
        s("ai_key", &self.key);
    }
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub user: bool,
    pub text: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).http_status_as_error(false).build().new_agent()
}

/// Send a chat and return the model's text reply.
pub fn chat(s: &AiSettings, system: &str, history: &[Msg]) -> Result<String, String> {
    let msgs: Vec<Value> = history.iter().map(|m| json!({"role": if m.user {"user"} else {"assistant"}, "content": m.text})).collect();
    let a = agent();
    match s.provider {
        Provider::OpenAi => {
            let mut all = vec![json!({"role": "system", "content": system})];
            all.extend(msgs);
            let mut body = json!({"messages": all, "temperature": 0.2, "max_tokens": 6000, "stream": false});
            if !s.model.trim().is_empty() {
                body["model"] = json!(s.model.trim());
            }
            let url = format!("{}/chat/completions", s.url.trim().trim_end_matches('/'));
            let mut req = a.post(&url).header("Content-Type", "application/json");
            if !s.key.trim().is_empty() {
                req = req.header("Authorization", &format!("Bearer {}", s.key.trim()));
            }
            let mut resp = req.send_json(&body).map_err(|e| friendly(&e.to_string(), &s.url))?;
            let status = resp.status();
            let v: Value = resp.body_mut().read_json().map_err(|e| format!("Unexpected reply from the AI ({status}): {e}"))?;
            if !status.is_success() {
                return Err(format!("The AI server said {status}: {}", v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or(&v.to_string())));
            }
            let msg = &v["choices"][0]["message"];
            let text = msg["content"].as_str().unwrap_or("").to_string();
            if text.trim().is_empty() {
                return Err("The AI sent an empty answer (it may have run out of room while thinking). Try again or use a shorter request.".into());
            }
            Ok(text)
        }
        Provider::Anthropic => {
            let model = if s.model.trim().is_empty() { "claude-sonnet-5-5" } else { s.model.trim() };
            let body = json!({"model": model, "max_tokens": 6000, "system": system, "messages": msgs, "temperature": 0.2});
            let base = if s.url.contains("anthropic.com") { s.url.trim().trim_end_matches('/').to_string() } else { "https://api.anthropic.com/v1".into() };
            let mut resp = a
                .post(&format!("{base}/messages"))
                .header("x-api-key", s.key.trim())
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .send_json(&body)
                .map_err(|e| friendly(&e.to_string(), &base))?;
            let status = resp.status();
            let v: Value = resp.body_mut().read_json().map_err(|e| format!("Unexpected reply ({status}): {e}"))?;
            if !status.is_success() {
                return Err(format!("Claude API said {status}: {}", v["error"]["message"].as_str().unwrap_or(&v.to_string())));
            }
            Ok(v["content"].as_array().map(|c| c.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")).unwrap_or_default())
        }
    }
}

fn friendly(e: &str, url: &str) -> String {
    if e.contains("onnect") || e.contains("refused") || e.contains("resolve") || e.contains("dns") {
        format!("Couldn't reach the AI at {url}. Is the model server running, and is the address right? ({e})")
    } else if e.contains("imeout") {
        "The AI took too long to answer (10 minutes).".into()
    } else {
        format!("Couldn't talk to the AI: {e}")
    }
}

/// Quick connection test: list models (OpenAI-style) or send a tiny message.
pub fn test(s: &AiSettings) -> Result<String, String> {
    if s.provider == Provider::OpenAi {
        let url = format!("{}/models", s.url.trim().trim_end_matches('/'));
        let mut req = agent().get(&url);
        if !s.key.trim().is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", s.key.trim()));
        }
        if let Ok(mut r) = req.call() {
            if r.status().is_success() {
                if let Ok(v) = r.body_mut().read_json::<Value>() {
                    let names: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)).take(8).collect();
                    if !names.is_empty() {
                        return Ok(format!("Connected. Models: {}", names.join(", ")));
                    }
                }
            }
        }
    }
    let r = chat(s, "Reply with the single word OK.", &[Msg { user: true, text: "ping".into() }])?;
    Ok(format!("Connected. The AI replied: {}", r.trim().chars().take(60).collect::<String>()))
}

// ------------------------------------------------------------------ prompt

pub fn system_prompt(doc: &Document, selected: &[u64]) -> String {
    let mut shapes = String::new();
    for info in LIBRARY {
        let s = (info.shape)();
        let params = serde_json::to_value(&s).ok().and_then(|v| v.as_object().and_then(|o| o.values().next().cloned())).filter(|p| p.is_object()).map(|p| p.to_string()).unwrap_or_else(|| "{}".into());
        let d = info.default_size;
        shapes.push_str(&format!("- \"{}\" ({}) default size [{}, {}, {}] params {}\n", info.name, info.category, d[0], d[1], d[2], params));
    }
    let fils: Vec<String> = doc.filaments.iter().enumerate().map(|(i, f)| format!("{i}: {} {}", f.name, f.hex())).collect();
    let mut scene = String::new();
    for o in &doc.objects {
        let kind = match &o.kind {
            Kind::Shape(s @ Shape::Sketch { .. }) => s.label().to_string(),
            Kind::Shape(s) => {
                let p = serde_json::to_value(s).ok().and_then(|v| v.as_object().and_then(|o| o.values().next().cloned())).filter(|p| p.is_object()).map(|p| format!(" params {p}")).unwrap_or_default();
                format!("{}{p}", s.label())
            }
            Kind::Mesh(_) => "imported mesh".into(),
            Kind::Group { .. } => "group".into(),
        };
        scene.push_str(&format!(
            "- id {} \"{}\" ({kind}) size [{:.1}, {:.1}, {:.1}] pos [{:.1}, {:.1}, {:.1}] rot [{:.0}, {:.0}, {:.0}] colour {} {}{}\n",
            o.id, o.name, o.size.x, o.size.y, o.size.z, o.pos.x, o.pos.y, o.pos.z, o.rot.x, o.rot.y, o.rot.z, o.color,
            if o.hole { "HOLE" } else { "" },
            if selected.contains(&o.id) { " SELECTED" } else { "" }
        ));
    }
    if scene.is_empty() {
        scene = "(empty)\n".into();
    }
    format!(
        r#"You are the design assistant inside RoboForge, a Tinkercad-style 3D design program for a Bambu Lab H2C multi-colour 3D printer. You build designs from RoboForge's building blocks. You do NOT write meshes or code.

Answer with ONE JSON object and nothing else:
{{"say": "one or two friendly sentences for the user", "actions": [ ... ]}}

Actions:
- {{"op":"add","id":"a1","shape":"<shape name>","name":"Lid","size":[W,D,H],"pos":[x,y,z],"rot":[rx,ry,rz],"color":<filament number or name>,"hole":false,"params":{{...}}}}
- {{"op":"modify","target":<object id or your new id>,"size":[...],"pos":[...],"rot":[...],"color":..,"hole":..,"name":"..","params":{{...}}}}  (only include what changes)
- {{"op":"delete","target":<object id>}}
- {{"op":"group","targets":[ids...],"name":"..."}}  (merges them into one part; HOLE parts cut away from solid parts in the same group)

Rules:
- Units are millimetres. size = [width X, depth Y, height Z] of the part's bounding box.
- Directions as the user sees the plate: LEFT = -X, RIGHT = +X, FRONT (towards the user) = -Y, BACK = +Y, UP = +Z.
  A part's edges: left x - W/2, right x + W/2, front y - D/2, back y + D/2, top z + H.
  Example: to stand a 30 mm cup against the outside of the LEFT wall of a tray centred at x=0 that is 120 wide, put the cup at x = -60 - 15 = -75 (same y as the tray).
  To put a feature INSIDE a part along its back edge, keep it within the part: y = back - (feature depth)/2.
- Before answering, check every part: is it where the user said (left/right/front/back/on top), touching what it should touch, and not sticking out where it shouldn't?
- Parts that come apart (lids, jar caps, separate pieces) go BESIDE what they fit, on the plate (z = 0), at least 10 mm apart — never stacked on top and never in the same group. Decorations like raised text that belong to the lid sit on the lid and may be grouped with the lid only.
- Only group parts that must become one solid piece (or holes with what they cut).
- pos is the CENTRE OF THE BOTTOM of the part. z=0 means sitting on the print plate. To put B on top of A: B.z = A.z + A.height.
- rot is in degrees around X, Y, Z (applied in that order). Usually [0,0,0].
- To cut a hole or pocket: add a shape with "hole":true overlapping the solid, then group them. Holes should poke slightly out of the surface (0.5 mm) so the cut is clean.
- Text: shape "Text" with params {{"text":"ABC"}}; size is the size of the whole text block. Raised text sits on a surface (z = top of that surface), about 1-2 mm tall.
- Make parts touch or overlap slightly (0.1 mm) when they must join; keep separate parts at least 5 mm apart.
- The build plate is 300 x 320 mm and 325 mm tall; centre is [0,0].
- Walls at least 1.2 mm, small details at least 0.8 mm. Prefer flat bottoms; avoid overhangs steeper than 45°.
- Use the ready-made parts (hinges, bolts, gears, jars, hooks…) when they fit the request, and set their params.
- Use existing object ids from the scene when changing what is already there. Your own new parts get ids you choose (like "a1").
- If the request is unclear, make a sensible choice and mention it in "say".

Shapes (name, category, default size, settings you may set in params):
{shapes}
Filaments (use the number):
{fils}

Current scene ("SELECTED" = what the user has selected; "this", "it" or "the selected part" usually means those):
{scene}"#,
        fils = fils.join("\n")
    )
}

// ------------------------------------------------------------------ apply

/// Pull the JSON object out of a reply (models sometimes add thinking or ``` fences).
pub fn extract_json(text: &str) -> Option<Value> {
    let mut t = text.to_string();
    while let (Some(a), Some(b)) = (t.find("<think>"), t.find("</think>")) {
        if b < a {
            break;
        }
        t.replace_range(a..b + 8, "");
    }
    if let Ok(v) = serde_json::from_str::<Value>(t.trim()) {
        return Some(v);
    }
    // try every '{' from the first, matched to the last '}'
    let end = t.rfind('}')?;
    for (i, _) in t.match_indices('{') {
        if i < end {
            if let Ok(v) = serde_json::from_str::<Value>(&t[i..=end]) {
                if v.get("actions").is_some() || v.get("say").is_some() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn vec3(v: &Value) -> Option<DVec3> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some(DVec3::new(a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?))
}

fn colour(v: &Value, fils: &[Filament]) -> Option<u8> {
    if let Some(n) = v.as_u64() {
        return (n < fils.len() as u64).then_some(n as u8);
    }
    let s = v.as_str()?.trim();
    if let Ok(n) = s.parse::<u8>() {
        return ((n as usize) < fils.len()).then_some(n);
    }
    if let Some(i) = fils.iter().position(|f| f.name.eq_ignore_ascii_case(s)) {
        return Some(i as u8);
    }
    let rgb = crate::materials::parse_hex(s).or_else(|| named_rgb(s))?;
    let d = |c: [u8; 3]| (0..3).map(|k| (c[k] as f64 - rgb[k] as f64).powi(2)).sum::<f64>();
    (0..fils.len()).min_by(|&a, &b| d(fils[a].color).total_cmp(&d(fils[b].color))).map(|i| i as u8)
}

fn named_rgb(s: &str) -> Option<[u8; 3]> {
    Some(match s.to_ascii_lowercase().as_str() {
        "white" => [240, 240, 240],
        "black" => [20, 20, 20],
        "red" => [200, 30, 30],
        "blue" => [20, 80, 200],
        "green" => [30, 160, 70],
        "yellow" => [245, 200, 20],
        "orange" => [240, 120, 20],
        "purple" => [120, 40, 170],
        "pink" => [240, 120, 170],
        "grey" | "gray" => [128, 128, 128],
        "brown" => [110, 70, 40],
        _ => return None,
    })
}

/// Make a library shape by name and apply `params` on top of its defaults.
fn make_shape(name: &str, params: Option<&Value>) -> Result<(Shape, DVec3), String> {
    let info = LIBRARY
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case(name))
        .or_else(|| LIBRARY.iter().find(|i| i.name.to_ascii_lowercase().replace(' ', "") == name.to_ascii_lowercase().replace([' ', '_', '-'], "")))
        .ok_or_else(|| format!("unknown shape \"{name}\""))?;
    let mut shape = (info.shape)();
    if let Some(g) = crate::settings::fit_gap() {
        shape.apply_fit(g);
    }
    if let Some(p) = params.and_then(|p| p.as_object()) {
        if let Ok(mut v) = serde_json::to_value(&shape) {
            if let Some(inner) = v.as_object_mut().and_then(|o| o.values_mut().next()).and_then(|i| i.as_object_mut()) {
                for (k, val) in p {
                    if inner.contains_key(k) {
                        inner.insert(k.clone(), val.clone());
                    }
                }
                if let Ok(s) = serde_json::from_value::<Shape>(v) {
                    shape = s;
                }
            }
        }
    }
    let size = shape.natural_size().unwrap_or(DVec3::from(info.default_size));
    Ok((shape, size))
}

pub struct Applied {
    pub say: String,
    pub changed: Vec<u64>,
    pub problems: Vec<String>,
}

/// Apply a plan to the document. Returns what to tell the user.
pub fn apply(plan: &Value, doc: &mut Document, cache: &mut crate::scene::MeshCache) -> Applied {
    let say = plan["say"].as_str().unwrap_or("").to_string();
    let mut ids: std::collections::HashMap<String, u64> = Default::default();
    let mut changed = vec![];
    let mut problems = vec![];
    let resolve = |v: &Value, ids: &std::collections::HashMap<String, u64>| -> Option<u64> {
        if let Some(n) = v.as_u64() {
            return Some(n);
        }
        let s = v.as_str()?;
        ids.get(s).copied().or_else(|| s.parse().ok())
    };
    for a in plan["actions"].as_array().into_iter().flatten() {
        match a["op"].as_str().unwrap_or("") {
            "add" => {
                let name = a["shape"].as_str().unwrap_or("Box");
                match make_shape(name, a.get("params")) {
                    Ok((shape, natural)) => {
                        let natural_sized = shape.natural_size().is_some();
                        let size = vec3(&a["size"]).filter(|s| s.min_element() > 0.05 && !natural_sized).unwrap_or(natural);
                        let id = doc.alloc_id();
                        let label = a["name"].as_str().map(String::from).unwrap_or_else(|| shape.label().to_string());
                        let mut o = Object::new(id, &label, Kind::Shape(shape), size.min(DVec3::splat(400.0)));
                        o.pos = vec3(&a["pos"]).unwrap_or(DVec3::ZERO);
                        o.rot = vec3(&a["rot"]).unwrap_or(DVec3::ZERO);
                        o.hole = a["hole"].as_bool().unwrap_or(false);
                        o.color = colour(&a["color"], &doc.filaments).unwrap_or(0);
                        doc.objects.push(o);
                        if let Some(k) = a["id"].as_str() {
                            ids.insert(k.to_string(), id);
                        }
                        changed.push(id);
                    }
                    Err(e) => problems.push(e),
                }
            }
            "modify" => {
                let Some(id) = resolve(&a["target"], &ids) else {
                    problems.push("a change pointed at no part".into());
                    continue;
                };
                let fils = doc.filaments.clone();
                let Some(o) = doc.find_mut(id) else {
                    problems.push(format!("no part with id {id}"));
                    continue;
                };
                if let Some(p) = a.get("params").filter(|p| p.is_object()) {
                    if let Kind::Shape(s) = &o.kind {
                        if let Ok(mut v) = serde_json::to_value(s) {
                            if let Some(inner) = v.as_object_mut().and_then(|o| o.values_mut().next()).and_then(|i| i.as_object_mut()) {
                                for (k, val) in p.as_object().unwrap() {
                                    if inner.contains_key(k) {
                                        inner.insert(k.clone(), val.clone());
                                    }
                                }
                            }
                            if let Ok(ns) = serde_json::from_value::<Shape>(v) {
                                if let Some(n) = ns.natural_size() {
                                    o.size = n;
                                }
                                o.kind = Kind::Shape(ns);
                            }
                        }
                    }
                }
                if let Some(s) = vec3(&a["size"]).filter(|s| s.min_element() > 0.05) {
                    o.size = s;
                }
                if let Some(p) = vec3(&a["pos"]) {
                    o.pos = p;
                }
                if let Some(r) = vec3(&a["rot"]) {
                    o.rot = r;
                }
                if let Some(h) = a["hole"].as_bool() {
                    o.hole = h;
                }
                if let Some(c) = colour(&a["color"], &fils) {
                    o.color = c;
                    o.paint = None;
                }
                if let Some(n) = a["name"].as_str() {
                    o.name = n.into();
                }
                changed.push(id);
            }
            "delete" => {
                if let Some(id) = resolve(&a["target"], &ids) {
                    doc.objects.retain(|o| o.id != id);
                    changed.retain(|&c| c != id);
                }
            }
            "group" => {
                let t: Vec<u64> = a["targets"].as_array().into_iter().flatten().filter_map(|v| resolve(v, &ids)).filter(|id| doc.find(*id).is_some()).collect();
                if t.len() >= 2 {
                    if let Some(g) = doc.group(&t, cache) {
                        if let Some(n) = a["name"].as_str() {
                            if let Some(o) = doc.find_mut(g) {
                                o.name = n.into();
                            }
                        }
                        changed.retain(|c| !t.contains(c));
                        changed.push(g);
                        if let Some(k) = a["id"].as_str() {
                            ids.insert(k.to_string(), g);
                        }
                    }
                }
            }
            other => problems.push(format!("unknown step \"{other}\"")),
        }
    }
    Applied { say, changed, problems }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_builds_a_box_with_raised_text_and_a_hole() {
        let mut doc = Document::default();
        let mut cache = crate::scene::MeshCache::default();
        let reply = r#"<think>user wants {a box}</think>
```json
{"say":"Here is a 60 mm box with your name.","actions":[
 {"op":"add","id":"b","shape":"Box","size":[60,40,20],"pos":[0,0,0],"color":"red"},
 {"op":"add","id":"t","shape":"Text","params":{"text":"ROBERT"},"size":[50,10,1.5],"pos":[0,0,20],"color":1},
 {"op":"add","id":"h","shape":"Cylinder","size":[6,6,22],"pos":[25,15,-1],"hole":true},
 {"op":"group","targets":["b","h"],"name":"Box"},
 {"op":"add","shape":"Screw jar","params":{"wall":2.5},"pos":[100,0,0]}
]}
```"#;
        let plan = extract_json(reply).expect("json");
        let r = apply(&plan, &mut doc, &mut cache);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(r.say, "Here is a 60 mm box with your name.");
        assert_eq!(doc.objects.len(), 3); // group, text, jar
        let text = doc.objects.iter().find(|o| matches!(&o.kind, Kind::Shape(Shape::Text { text, .. }) if text == "ROBERT")).unwrap();
        assert_eq!(text.pos.z, 20.0);
        assert_eq!(text.color, 1);
        let jar = doc.objects.iter().find(|o| matches!(o.kind, Kind::Shape(Shape::Jar { wall }) if wall == 2.5)).unwrap();
        assert_eq!(jar.pos.x, 100.0);
        // red resolved to the palette's red
        let g = doc.objects.iter().find(|o| o.name == "Box").unwrap();
        assert!(matches!(g.kind, Kind::Group { .. }));
        let p = system_prompt(&doc, &[]);
        assert!(p.contains("\"Hinge\"") && p.contains("Current scene") && p.contains("ROBERT"));
    }

    #[test]
    fn modify_and_delete_existing() {
        let mut doc = Document::default();
        let mut cache = crate::scene::MeshCache::default();
        let id = doc.alloc_id();
        doc.objects.push(Object::new(id, "Box", Kind::Shape(Shape::Box { radius: 0.0, steps: 1 }), DVec3::splat(20.0)));
        let plan = json!({"say":"ok","actions":[{"op":"modify","target":id,"size":[30,30,5],"color":"2"},{"op":"add","id":"x","shape":"Sphere","pos":[50,0,0]},{"op":"delete","target":"x"}]});
        let r = apply(&plan, &mut doc, &mut cache);
        assert!(r.problems.is_empty());
        assert_eq!(doc.objects.len(), 1);
        assert_eq!(doc.objects[0].size, DVec3::new(30.0, 30.0, 5.0));
        assert_eq!(doc.objects[0].color, 2);
    }
}
