# Changelog (RoboForge, formerly PrintForge)

The version shows in the window title and the status bar, and in the download file names.

## 1.11.0 — 2026-11-28
- **Head** in Basic shapes: a smooth, friendly human head on a neck with a flat base
  (about 55 mm tall). Resize it, then shape it with the Sculpt tool (turn on Mirror) or add
  details with Raise/Sink.

## 1.10.2 — 2026-11-28
- **See-through plate from below:** orbit under the plate and it turns see-through, and every
  part's area that sits on the plate shows in green. A part with no green isn't touching
  the plate (press D to drop it down).

## 1.10.1 — 2026-11-28
- **Getting around:** right-drag now slides the view (Shift+right-drag or left-drag on empty
  space still orbits). Scrolling zooms towards whatever is under the pointer, and
  double-clicking a part zooms in on it (F still does too).
- **Snap ball:** the socket fingers cracked when pressing the ball in. They're now long,
  thin and springy (split almost to the base) with a smaller lip, so PLA bends instead of
  breaking.

## 1.10.0 — 2026-11-28
- **Sculpt** tool (toolbar): shape any part's surface with a soft round brush.
  - **Pull out** / **Push in** — raise or press in the area under the brush.
  - **Smooth** — even out bumps; **Flatten** — level an area.
  - **Drag** — grab a spot and pull it anywhere; the area around follows smoothly.
  - Brush size (also [ and ] keys) and strength; **Mirror left ↔ right** for symmetrical
    things like faces and animals. A ring shows the brush on the surface.
  - Detail is added under the brush automatically so strokes stay smooth; each stroke is
    one Ctrl+Z. The part becomes a mesh the first time you sculpt it (shape settings no
    longer apply).
- **Raise/Sink** tool: draw a rectangle, circle or your own outline on a flat face, set a
  height in mm (negative sinks it in) and Apply. Crisp edges and exact sizes; the area is its
  own part grouped with yours, so Ungroup to move or resize it later. Esc cancels.

## 1.9.0 — 2026-11-28
- **Drag preview:** dragging a shape from the Shapes panel now shows a faint see-through copy
  of it on the plate, with its outline, exactly where it will land (on top of other parts
  too). Let go to drop it there.
- **My Parts** tiles can be dragged onto the plate as well.

## 1.8.2 — 2026-11-28
- **Ball joint:** the neck snapped while freeing a stuck ball. The ball is bigger, the neck is
  about 40% thicker with a flare where it meets the block, and the cup has extra room under
  the ball so its underside can't sag onto the floor and stick. The cup never opens wide
  enough for the ball to pull out, even with big gaps.

## 1.8.1 — 2026-11-28
- **Snap ball:** the socket was too stiff and too tight for the ball to go in. The cup is now a
  little roomier, the four fingers are split deeper so they spread, the lip is gentler and
  the top has a lead-in so the ball finds its way in.

## 1.8.0 — 2026-11-26
- **My Parts:** select anything you've made and click **★ Save to My Parts…** in the
  Inspector. It appears at the top of the Shapes panel with its own picture; click it to add
  it to any design. Parts keep their colours (matched to the closest filament you have
  loaded) and their settings: Ungroup to edit a saved keychain's name, a joint's gap, etc.
  Right-click a part to rename or delete it. They're stored in the RoboForge settings folder
  under *My Parts*, one file each.
- New parts now go to an empty spot on the plate instead of lining up off the edge.

## 1.7.0 — 2026-11-26
- New **Joints** section in Shapes, with five joints for articulated toys:
  - **Side-to-side:** coned pin in a coned socket, the classic flexi-animal joint.
  - **Up & down:** the same joint on its side, for jaws, necks and tails that curl.
  - **Dragon joint:** two pin joints at right angles with a short link, so it bends both
    ways like the articulated dragons that wrap round your arm.
  - **Ball joint:** a ball printed inside a cup; tilts every way and twists.
  - **Snap ball:** ball-on-peg and a split socket, printed apart and pushed together, for
    stiff poseable joints (action-figure shoulders and hips).
  Each has size, swing and gap settings (the gap follows your Fit test).
- New library parts now start at their real size.

## 1.6.1 — 2026-11-26
- **Flexi joint** (Moving parts): the joint that articulated toy animals use, on its own: a
  coned pin in a coned socket, printed in place, with two stubs to build the body pieces
  onto. Set its size, gap and how far it swings each way.

## 1.6.0 — 2026-11-26
- **Quick makers** (new section in Shapes):
  - **Name keychain:** type a name; the tag follows the letters, with a key-ring hole on the
    left or top and the letters raised in a second colour.
  - **Cookie cutter:** draw the outline or **trace a picture** (a dark shape on a light
    background). Walls stay the same thickness when you resize it; optional grip rim.
- **Lamp shade** in Picture to 3D: a round lithophane that slips over an LED tea light. Set the
  diameter and how many times the picture goes round.
- Fixed: flat tops of extruded shapes could have a few flipped triangles.

## 1.5.0 — 2026-11-26
- **Multiple plates:** tabs at the top-left of the view. **+ Plate** adds one; click a tab to
  work on that plate. **Move to: Plate N** in the Inspector moves the selected parts. Right-click
  an empty plate's tab to delete it. **Export to Bambu** and **Print** put every plate in one
  project, laid out the way Bambu Studio does, so **Slice all** prints them one after another.
  STL, OBJ and standard 3MF export the plate you're on. Older designs open as one plate.

## 1.4.0 — 2026-11-26
- **Save time** in the Print preview: "Find ways to cut colour changes" tries printing each
  colour as each of the others and lists the swaps that save the most changes, with the time
  saved and whether the colours look alike. **Apply** recolours the design (Ctrl+Z undoes it).

## 1.3.0 — 2026-11-26
- **Wi-Fi QR code** and **QR code** (Electronics): a plaque with the code raised in a second
  colour. Wi-Fi codes let guests join by pointing a phone camera at it (WPA or open networks);
  the other kind holds any link or text (Home Assistant dashboards, manuals…). Optional pocket
  underneath for a 25 mm NFC sticker. Only the top layers change colour, so it prints quickly.
  Checked by rendering and decoding the code.

## 1.2.0 — 2026-11-26
- **Board case** (new Electronics section): a case for Raspberry Pi 5, Raspberry Pi 4,
  Arduino Uno R3, ESP32 DevKitC or a custom board size. Screw posts with pilot holes at the
  board's mounting holes (ledges for boards without holes), openings for each port (USB-C,
  HDMI, USB, Ethernet, audio, microSD…), extra height for HATs/fans, vent slots in the lid,
  optional wall-mount tabs. The lid prints beside the base and drops into it (uses your fit gap).
  Raspberry Pi positions come from the official mechanical drawings.

## 1.1.0 — 2026-11-26
- **Moving parts** (new section in Shapes):
  - **Flexi snake** and **Flexi chain**: print in one piece and come off the plate already
    bending. Each joint is a round knuckle in a socket with 45° chamfers top and bottom, so it
    can't pull apart and needs no supports. Set segments, width and joint gap.
  - **Spinning rings**: 2–8 concentric rings locked by a 45° V-ridge — a print-in-place bearing.
  - **Gear pair**: two gears with any tooth counts plus a base with posts at exactly the right
    distance apart (shows the ratio). Push the gears onto the posts.
  - Joint gaps follow your Fit test result (+0.1 mm) when it's set.

## 1.0.0 — 2026-11-26
- **Version history** (File → Version history…): every save keeps a copy (the last 100 per
  design). See when each was saved and how many parts it had; **Restore** loads it (Ctrl+Z
  brings back what you had), or **Save a copy…** keeps it as its own file.
- 1.0: everything on the feature list is in — parts library, sketch, paint & picture decals,
  text on surfaces, threads, rounded edges, splitting, Gridfinity, fit test, print preview,
  filament types, AI helper, and one-click Print to Bambu Studio.

## 0.9.4 — 2026-11-26
- **Add a thread** (Inspector) to any part: a **screw thread** on the top or bottom end (that end
  becomes a screw of the diameter you pick) or a **threaded hole** down the middle with a lead-in.
  Standard metric pitch is picked for you (or set your own); threaded holes get your Fit test gap
  so a screw of the same size fits.

## 0.9.3 — 2026-11-26
- **Aa Text on part** (tools bar): raised or cut-in lettering on any face of a part — flat or
  curved (letters follow a cylinder's curve). Pick text, letter height, how far it sticks out or
  cuts in, turn, font and colour; a preview follows the mouse and a click puts it on.
  Cut-in letters are filled with the chosen colour, so they print as coloured inlays.

## 0.9.2 — 2026-11-26
- **Split into pieces** (Inspector): cut any part across X, Y or Z — an orange plane shows where.
  Optional alignment pins: RoboForge finds two spots well inside the cut, makes matching holes
  in both pieces (sized with your Fit test gap) and adds the pins as separate parts. Cutting
  across Z puts the top piece on the plate cut-face down, beside the bottom piece.

## 0.9.1 — 2026-11-26
- **Round / bevel edges** for any solid part (Inspector → "Round / bevel edges"): pick Round or
  Bevel and a size, then "Apply to all sharp outside edges". Works on boxes, text, groups,
  imported models and sketches; box corners get proper ball-shaped rounds. By default the edges
  sitting on the plate stay sharp (rounded bottoms print as overhangs). Colours are kept.

## 0.9.0 — 2026-11-26
- **Gridfinity** (Home & storage), Zack Freedman's open modular drawer system:
  - **Gridfinity bin**: 1–7 × 1–7 grid units (42 mm) and 2–20 height units (7 mm), compartments
    across and front-to-back, optional magnet holes (6×2 mm) and stacking lip. Standard feet so
    it drops into any Gridfinity baseplate.
  - **Gridfinity baseplate**: 1–7 × 1–7 open-frame plate with the standard pocket profile (ridges
    kept at a printable 0.9 mm and a 0.5 mm border).

## 0.8.1 — 2026-11-26
- Fit test: numbers moved in from the plate's front edge (0.8.0 cut them right at the edge).

## 0.8.0 — 2026-11-26
- **Fit test** (Hardware): a plate with six numbered holes, each 0.1 mm looser than the last
  (0.0–0.5 mm gap), plus a 10 mm peg. Print it, push the peg into each hole, and click the number
  of the tightest hole it slides into in the Inspector. RoboForge remembers it and from then on
  new lids, jar caps, nuts, hinges and dovetails (including ones the AI helper makes) are made
  with the right gap for your printer.

## 0.7.2 — 2026-11-26
- AI helper's text box: **right-click → Paste / Copy all / Clear**, and a backup for Ctrl+V that
  reads the clipboard directly if Windows doesn't deliver the paste.

## 0.7.1 — 2026-11-26
- **Make it printable** now spots a lid or jar cap sitting on its box/jar and moves it (with any
  raised text on it) onto the plate beside it, so they print as separate pieces instead of fused.
- **AI helper** instructions improved: it now knows which way is left/right/front/back, how to
  work out edges when placing parts against each other, to double-check placements, and to put
  lids and caps beside what they fit instead of stacking or grouping them.

## 0.7.0 — 2026-11-26
- **✨ AI helper** (tools bar): describe what you want — "a 60 mm box with a lid and ROBERT
  raised on top in blue", "make the selected part 10 mm taller" — and an AI model builds it.
  - Works with your own local model through any OpenAI-style server (llama.cpp, Ollama,
    LM Studio, vLLM…) or with Claude using an Anthropic API key. Set the address/model in
    ⚙ Settings and click "Save & test connection".
  - The AI doesn't draw triangles: it sends a plan made of RoboForge's own shapes and parts
    (sizes, positions, colours, settings, holes, groups), so the result is always clean and
    printable. It sees what's on the plate and what you've selected, so you can keep refining.
  - Ctrl+Z undoes anything it did. Reasoning models' "thinking" is ignored automatically.

## 0.6.2 — 2026-11-26
- **▶ Print** button (top right, or Ctrl+P / Cmd+P): checks the design, saves it to RoboForge's
  Prints folder and opens it straight in Bambu Studio — just press Slice plate, then Print.
  No save dialog. Finds Bambu Studio in its usual place; if it's somewhere else it asks once and
  remembers. "Export to Bambu" is still there for saving a file wherever you like.

## 0.6.1 — 2026-11-26
- **Filament types**: each filament in the Filaments list now has a type — 32 of Bambu Lab's own
  H2C presets (PLA Basic/Matte/Silk+/Tough+/Wood/CF…, PETG Basic/HF/CF, ABS, ASA, PC, TPU 95A HF,
  PA6-CF, PVA, Support for PLA, and Generic PLA/PETG/PCTG/ABS/ASA/TPU for other brands).
  - **Fixes**: exports used to label every filament "Bambu PLA Basic". Each filament now exports
    with its own preset, so Bambu Studio uses the right temperatures and speeds with no setup.
  - Older designs (type "PLA") open as PLA Basic.
- **Colour codes**: type an exact hex code (e.g. #C8102E) next to each filament.
- **★ My Filaments**: save spools once (★ next to a filament, or "Save all filaments in this
  design") and add them to any later design from the My Filaments window.
- **Mixed-material warnings** in the Filaments panel and the print check (PLA + PETG layers
  barely bond, PLA with ABS/ASA/PC, TPU in multi-colour prints, PVA).
- The preview's gram estimates use each preset's real density.

## 0.6.0 — 2026-11-25
- **Preview** (top bar): see your design as it will print.
  - Layer slider (and ◀ ▶ buttons) cuts the view at any layer, with the layer outline drawn in
    each filament colour; shows which colours are on that layer and how many changes it needs.
  - Estimates for the Bambu Lab H2C: print time, grams of each filament, number of colour changes
    and the waste they cause. Knows the H2C's 7 hotends: up to 7 filaments change by hotend swap
    (~45 s, almost no waste); more than 7 means some full purges and it says so.
  - Rough (±30%), based on 0.2 mm layers, 2 walls and 15% infill — Bambu Studio's numbers after
    slicing are the exact ones. Updates by itself when you change the design.

## 0.5.2 — 2026-11-25
- **Hollow spun shapes**: tick "Hollow (open top)" in the sketch window's Spin mode (or in the
  Inspector) and set the wall and floor thickness. The inside follows your outline, so the wall
  is the same thickness everywhere — narrow necks included. The inside is drawn dashed while you
  sketch; a warning appears if a neck is too narrow for the wall you chose.

## 0.5.1 — 2026-11-25
- **👁 See-through** button (top bar, or press X): every part turns see-through with its sharp
  edges outlined, so you can look inside — check a hole you cut, a hollow, or a hidden cavity.
  Press it again to go back to normal.
- The **Home** view button now also zooms to fit everything on the plate.

## 0.5.0 — 2026-11-25
- **Sketch tool** (✏ Sketch on the toolbar, File menu, or the Shapes panel): draw your own outline
  on a millimetre grid, then
  - **Extrude**: push it straight up to a height, or
  - **Spin**: spin a half side-view round an axis (vases, knobs, bottles, rings).
  - Click to add points, click on a line to insert one, drag to move, right-click to delete,
    double-click a point to round that corner. Grid snap, zoom/pan, undo, live size readout.
  - Warns in plain English if lines cross or a spin outline is on the wrong side of the axis.
  - Starters: square, rounded square, arrow, vase profile.
- Sketched parts stay editable: select one and click **Edit sketch…** in the Inspector.
  They resize, colour, paint and group like any other part.

## 0.4.2 — 2026-11-24
- **Picture onto a part** (File menu, the Shapes panel, or "On a part's surface" in the
  Picture dialog): paints a photo or logo onto a part's surface in your filament colours.
  - A see-through preview follows the mouse so you can see where it lands; click to paint.
  - Width, turn (rotation), "leave the background unpainted" (skips the white around a
    logo; transparent PNG areas are always skipped) and which filaments it may use.
  - Lies flat on the face you point at, or "Project from my view" to wrap round parts.
  - Only surfaces facing the picture and not hidden behind other parts get painted; the
    surface is finely subdivided under the picture so detail is kept. Undo works as usual.

## 0.4.1 — 2026-11-24
- **24 more ready-made parts**, and the parts are now grouped into Hardware, Mounting and
  Home & storage:
  - Hardware: washer, standoff (hex), pulley, wheel with spokes, compression spring, gear rack
    (meshes with the Gear), sliding dovetail joint, living hinge.
  - Mounting: wall hook, pegboard hook (1/4" pegboard), pipe clamp (sized from the pipe),
    corner brace, keyhole hanging plate, cable comb, desk grommet.
  - Home & storage: divided bin (rows × columns), screw-top jar with matching cap (adding a jar
    adds its cap), planter with drain hole, saucer, funnel, drawer handle, phone stand, key tag.
- Every part was run through the print check; none has errors.
- `roboforge parts-sheet <file.pforge>` lays out every part and prints any check warnings.

## 0.4.0 — 2026-11-24
- **Parts library** (new PARTS section in the Shapes panel), all adjustable in the Inspector:
  - **Box + lid** — open container (wall, floor, corner radius) plus a matching lid with a lip
    and fit clearance (prints face-down).
  - **Bracket** — L-bracket with gusset and screw holes.
  - **Gear** — involute spur gear with any number of teeth and a bore.
  - **Knob** — grippy knob with a D-shaft hole.
  - **Cable clip** — sized from the cable diameter, with screw tab.
  - **Bolt** and **Nut** — metric coarse threads (M3–M20), nut with print clearance and
    countersunk ends so they actually screw together.
  - **Hinge** — print-in-place hinge (prints already assembled).
  - **Snap hook** — cantilever snap-fit with adjustable arm and catch.
- Print check: very small knife-edge slivers (e.g. thread tips) are now a warning, not an error.

## 0.3.0 — 2026-11-24
- **Picture to 3D** (File menu, Shapes panel, or drop a PNG/JPG onto the window):
  - *Lithophane* — thin where the photo is bright, thick where dark; glows when lit from behind.
  - *Raised relief* — a plate with the picture raised (or sunk, with Invert).
  - *Colour plate* — a flat plate coloured with your filaments, each spot using the closest
    filament colour (pick which filaments it may use).
  - Width, thicknesses, frame and detail are adjustable.
- **Logo from SVG**: each colour in the SVG becomes a part in the matching filament (new
  filaments are added automatically), stacked shapes are cut so they don't overlap, and the
  parts are grouped into one multi-colour logo. Strokes (outlines) are supported too.

## 0.2.0 — 2026-11-24
- **✨ Make it printable:** one click turns each part onto its best face (more flat contact,
  fewer overhangs), puts everything on the plate, spreads overlapping parts out, flags tall
  parts that need a brim, then runs the print check.
- **Overhangs view:** toggle to see in orange every face that will need support.
- **Colour by height (stripes):** in the Inspector, list heights and filaments
  ("from 5 mm use red…") and press Apply — the part gets crisp horizontal colour bands.
- **Drag and drop:** drop STL/OBJ/3MF files or .pforge projects onto the window.
- **Open recent** in the File menu (projects and imports).
- **Auto-save and crash recovery:** unsaved work is backed up every minute; if RoboForge
  closes unexpectedly, it offers to recover the design next time.
- Toolbar split into two rows so nothing is cut off on smaller screens.

## 0.1.10 — 2026-11-24
- **Fixed:** on Windows (and Macs) set to light mode, the menus and side panels stayed light
  while the rest of the app was dark. RoboForge now uses the theme chosen under
  View ▸ Dark theme regardless of the operating system's light/dark setting.

## 0.1.9 — 2026-11-24
- **New name: RoboForge** (was PrintForge). Downloads are now `RoboForge-<version>-…`.
  Existing `.pforge` project files still open.
- **Dark professional theme** by default, with section headers, a blue accent colour, and a
  darker 3D workspace so filament colours stand out. Switch back under View ▸ Dark theme
  (your choice is remembered).
- The Inspector no longer jumps around when you switch a part between Solid and Hole.
- Colour swatches have an outline so dark filaments (e.g. black) are visible.

## 0.1.8 — 2026-11-24
- **Side handles (Tinkercad-style):** a selected part shows small white squares on its sides
  and top. Drag one and only that side moves — the opposite side stays put. The new size
  shows while dragging and snaps to the grid (hold Shift for free sizing).
- **Push/Pull tool:** click any flat face and drag it outward to extrude or inward to cut,
  with a live green/red preview and distance readout; or type an exact distance and press
  Apply. Works on any part, including imported and painted ones (the part becomes a mesh).

## 0.1.7 — 2026-11-24
- **Shape tiles:** the Shapes panel shows a small 3D picture of each shape (drawn from the
  real shape) instead of text buttons. Click a tile to add it, or drag it straight into the
  workspace or onto another part.

## 0.1.6 — 2026-11-24
- **Print check verdict:** a clear green "Ready to print" or red "Not ready to print" box at
  the top of the results, with a colour key (red = must fix, orange = worth knowing,
  blue = info only).
- **Unsaved-changes warning:** closing the app, starting a New design or opening another
  project with unsaved work asks *Save / Don't save / Cancel*. The window title shows ● and
  the status bar says "unsaved changes" until you save.
- **Fixed:** clicking a shape in the Shapes panel didn't add it (only dragging worked) since 0.1.2.

## 0.1.5 — 2026-11-24
- **Fixed:** a part sitting on top of another part (e.g. a cone on a box) was reported as
  "Floating above the build plate" and blocked export. It's now recognised as stacked.
- Stacked parts are exported to Bambu Studio as **one object with several parts**, each keeping
  its own filament and paint. (Separate objects would be dropped to the plate by Bambu Studio.)
- A stack that genuinely floats above the plate is still reported as an error.
- Importing a Bambu 3MF now keeps each part's own filament colour.

## 0.1.4 — 2026-11-24
- **The paint brush now paints like a brush:** a round dab the size of the brush radius,
  and smooth strokes when you drag, instead of colouring whole triangles (which on a box
  meant half a side at a time). The surface is refined automatically just under the brush.
  Painting a shape turns it into a mesh (size and position stay editable).
- Bucket fill still fills whole flat areas up to sharp edges.

## 0.1.3 — 2026-11-24
- **Fixed:** couldn't line parts up exactly. Grid snap now snaps the part's *position*
  (e.g. X = 0.0), not the distance moved, so a part that started at X = -8.57 no longer
  stays stuck on .57 values.
- **Fixed:** edge/centre snapping now works at any zoom level (it uses a distance on screen),
  and also works when moving with the gizmo arrows.
- **Fixed:** grabbing the middle of a part sometimes grabbed the gizmo's centre circle, which
  moved parts parallel to the screen and changed their height (e.g. 20 → 19.88). The
  centre handles are removed; drag the part itself, or the arrows/plane square.
- Dragging only follows upward-facing surfaces (tops), not side walls.

## 0.1.2 — 2026-11-24
- **Objects snap to each other.**
  - *Ride on top:* drag a part over another and it sits on that part's top surface;
    drag it off and it drops back to the plate. Dragging follows the surface under the cursor.
  - *Edge snapping:* while dragging, edges and centres line up with nearby parts (or butt
    flush against them), shown with a pink guide line.
  - *Snap faces* tool: click a face on the part to move, then a face on another part; the
    first is turned and moved so the two faces sit flush.
  - Shapes can be dragged from the Shapes panel straight onto the plate or onto another part.
  - Hold **Shift** while dragging to move freely without any snapping.

## 0.1.1 — 2026-11-24
- **Fixed:** clicking and dragging in the 3D view stopped working once anything was selected
  (the move/rotate/scale gizmo was capturing every click). Selecting, moving, orbiting and
  deselecting now work with the gizmo showing; it only takes clicks on its own handles.
- Gizmo handles now respect Windows display scaling.
- Smoother dragging with large models (picking data is rebuilt after the drag, not every frame).
- Version number shown in the title bar, status bar and download names.

## 0.1.0 — 2026-11-23
- First release: shapes, holes, groups, 3D text, painting, print checker, Bambu H2C colour 3MF export, STL/OBJ/3MF import and export.
