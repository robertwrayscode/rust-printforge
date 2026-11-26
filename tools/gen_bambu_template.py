#!/usr/bin/env python3
"""Build assets/bambu_h2c_template.json from a Bambu Studio checkout.

Usage: gen_bambu_template.py <BambuStudio repo> <H2D sample project_settings.config> <out.json>

Takes the key set and array layout of a real dual-nozzle Bambu Studio project and
overlays the fully resolved system presets for the H2C (printer, process, filament),
so the exported project matches Bambu's own presets as closely as possible.
"""
import json, os, sys, glob

repo, sample, out = sys.argv[1:4]
prof = os.path.join(repo, "resources/profiles/BBL")
index = {}
for kind in ("machine", "process", "filament"):
    for f in glob.glob(os.path.join(prof, kind, "**/*.json"), recursive=True):
        try:
            d = json.load(open(f, encoding="utf-8"))
        except Exception:
            continue
        if "name" in d:
            index[(kind, d["name"])] = d

def resolve(kind, name):
    d = index[(kind, name)]
    base = resolve(kind, d["inherits"]) if d.get("inherits") else {}
    r = dict(base); r.update(d); return r

SKIP = {"name", "inherits", "from", "instantiation", "type", "setting_id", "description"}
machine = resolve("machine", "Bambu Lab H2C 0.4 nozzle")
for (k, n), d in index.items():
    if k == "machine" and n.startswith("Bambu Lab H2C 0.4 nozzle template"):
        machine.update({kk: v for kk, v in d.items() if kk not in SKIP})
process = resolve("process", "0.20mm Standard @BBL H2C")
fil = resolve("filament", "Bambu PLA Basic @BBL H2C")

tmpl = json.load(open(sample, encoding="utf-8"))
nfil = len(tmpl["filament_colour"])
variants = ["Direct Drive Standard", "Direct Drive High Flow", "Direct Drive E3D High Flow"]
nv = len(variants)

glob_cfg, per_fil, per_var = {}, {}, {}
fil_keys = set(k for k in fil if k not in SKIP)
for k, v in tmpl.items():
    if isinstance(v, list) and len(v) == nfil and k != "different_settings_to_system":
        per_fil[k] = v[:1]
    elif isinstance(v, list) and len(v) == 2 * nfil and k.startswith(("filament_", "nozzle_temperature")) or k in ("flush_volumes_vector",):
        per_var[k] = (v[:2] + v[:1])[:nv]
    elif k in ("flush_volumes_matrix", "different_settings_to_system", "filament_self_index", "filament_extruder_variant"):
        continue
    else:
        glob_cfg[k] = v

for src in (machine, process):
    for k, v in src.items():
        if k in SKIP or k in per_fil or k in per_var:
            continue
        glob_cfg[k] = v
for k in fil_keys:
    v = fil[k]
    if not isinstance(v, list):
        glob_cfg.setdefault(k, v); continue
    if len(v) == nv or k in per_var:
        per_var[k] = v if len(v) == nv else (v * nv)[:nv]
        per_fil.pop(k, None)
    else:
        per_fil[k] = v[:1]
per_var.pop("flush_volumes_vector", None)

glob_cfg.update({
    "printer_settings_id": "Bambu Lab H2C 0.4 nozzle",
    "print_settings_id": "0.20mm Standard @BBL H2C",
    "printer_model": "Bambu Lab H2C",
    "filament_map_mode": "Auto For Flush",
    "version": tmpl.get("version", "02.00.02.01"),
})
for k in ("filament_settings_id",):
    per_fil[k] = ["Bambu PLA Basic @BBL H2C"]
per_fil["filament_ids"] = [fil.get("filament_id", "GFA00")]
per_fil.pop("filament_colour", None)
per_fil.pop("filament_map", None)
for k in ("compatible_printers", "compatible_printers_condition", "compatible_prints", "compatible_prints_condition"):
    glob_cfg.pop(k, None); per_fil.pop(k, None); per_var.pop(k, None)
# ---- other filament presets: only the values that differ from PLA Basic
MATERIALS = [
    "Bambu PLA Basic", "Bambu PLA Matte", "Bambu PLA Silk+", "Bambu PLA Silk", "Bambu PLA Tough+",
    "Bambu PLA Translucent", "Bambu PLA Wood", "Bambu PLA Glow", "Bambu PLA Marble", "Bambu PLA Sparkle",
    "Bambu PLA Metal", "Bambu PLA Galaxy", "Bambu PLA-CF", "Bambu PETG Basic", "Bambu PETG HF",
    "Bambu PETG Translucent", "Bambu PETG-CF", "Bambu ABS", "Bambu ASA", "Bambu PC", "Bambu TPU 95A HF",
    "Bambu TPU for AMS", "Bambu PA6-CF", "Bambu Support For PLA", "Bambu PVA",
    "Generic PLA", "Generic PLA Silk", "Generic PETG", "Generic PCTG", "Generic ABS", "Generic ASA", "Generic TPU",
]
def first(v):
    return v[0] if isinstance(v, list) and v else v
materials = {}
for name in MATERIALS:
    f = resolve("filament", name + " @BBL H2C")
    pf, pv = {}, {}
    for k in fil_keys | set(k for k in f if k not in SKIP):
        if k in ("compatible_printers", "compatible_printers_condition", "compatible_prints", "compatible_prints_condition", "filament_colour"):
            continue
        v = f.get(k)
        if v is None:
            continue
        if k in per_var:
            vv = v if isinstance(v, list) else [v]
            vv = vv if len(vv) == nv else (vv * nv)[:nv]
            if vv != per_var[k]:
                pv[k] = vv
        elif k in per_fil:
            vv = (v if isinstance(v, list) else [v])[:1]
            if vv != per_fil[k]:
                pf[k] = vv
    pf["filament_settings_id"] = [name + " @BBL H2C"]
    pf["filament_ids"] = [f.get("filament_id", "")]
    materials[name] = {
        "type": first(f.get("filament_type", ["PLA"])),
        "density": float(first(f.get("filament_density", ["1.24"]))),
        "nozzle_temp": int(float(first(f.get("nozzle_temperature", ["220"])))),
        "per_filament": pf,
        "per_variant": pv,
    }
json.dump({"variants": variants, "global": glob_cfg, "per_filament": per_fil, "per_variant": per_var, "materials": materials},
          open(out, "w"), indent=1, sort_keys=True)
print("global", len(glob_cfg), "per_filament", len(per_fil), "per_variant", len(per_var))
