# Changelog

The version shows in the window title and the status bar, and in the download file names.

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
