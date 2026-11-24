# Changelog

The version shows in the window title and the status bar, and in the download file names.

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
