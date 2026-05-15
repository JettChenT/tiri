# Cursor-Anchored Overlay Surfaces

## Goal

Let a client register a visible surface that tiri positions next to a real or virtual cursor. The client owns the pixels and content. The compositor owns placement, clipping, z-order, and cursor-follow behavior.

The target use case is an agent or shell client that wants to show compact contextual UI next to the cursor without moving the real pointer, stealing focus, or polling output geometry from the outside.

## Current State

tiri already has two useful halves:

- Virtual cursors are compositor-rendered and pinned to a mapped window plus window-relative coordinates. `VirtualCursorUi` resolves the pinned point into output-local render coordinates every frame, so the cursor follows layout and overview transforms.
- `WindowPreviewUi` is compositor-owned overlay UI. It renders above normal windows from explicit output-local bounds supplied by an action.

What is missing is a surface path where an external client supplies the UI content while tiri supplies cursor-relative placement.

Normal application windows are not a good fit. Moving an `xdg_toplevel` next to the cursor fights the tiling layout model. Existing layer-shell surfaces are closer, but their public protocol placement is edge/corner based. A layer-shell client can choose anchors and margins, but cannot ask "place this surface to the right of cursor `agent-primary`, flip if it hits the output edge, and keep following that cursor through overview transforms."

## Proposal

Add a compositor-owned overlay registry for client surfaces:

```rust
pub enum Request {
    CursorOverlays,
    RegisterCursorOverlay {
        overlay: CursorOverlayRegister,
    },
    UpdateCursorOverlay {
        overlay: CursorOverlayUpdate,
    },
    UnregisterCursorOverlay {
        overlay_id: String,
    },
}

pub struct CursorOverlayRegister {
    pub overlay_id: String,
    pub surface_selector: CursorOverlaySurfaceSelector,
    pub anchor: CursorOverlayAnchor,
    pub placement: CursorOverlayPlacement,
    pub visible: Option<bool>,
    pub z_index: Option<i32>,
    pub replace_existing: bool,
}

pub enum CursorOverlaySurfaceSelector {
    LayerNamespace { namespace: String },
    LayerSurfaceId { id: u64 },
}

pub enum CursorOverlayAnchor {
    HardwarePointer,
    VirtualCursor { cursor_id: String },
}

pub struct CursorOverlayPlacement {
    pub side: CursorOverlaySide,
    pub align: CursorOverlayAlign,
    pub gap: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub edge_padding: f64,
    pub flip: bool,
}
```

The client creates a layer-shell surface on the overlay layer, usually with a stable namespace like `agent-cursor-menu`. It then registers that surface with tiri over IPC. Once registered, tiri removes that surface from normal layer-shell arrangement and renders it through a `CursorOverlayUi` manager.

This keeps the content path Wayland-native. The client can keep using GTK, Qt, GPUI, webview, or QML to draw a real surface. The compositor only takes over where the surface appears.

## Placement Model

Every frame, tiri resolves the anchor to output-local logical coordinates:

- `HardwarePointer` uses the current seat pointer location, or tablet cursor location when active.
- `VirtualCursor { cursor_id }` uses the same resolved visual point as `VirtualCursorUi`, including animation and window-relative layout transforms.

Then tiri computes the overlay rectangle from the surface's configured size:

1. Place the surface on the requested side: right, left, above, below, or preferred ordered list.
2. Apply `align`, `gap`, and explicit offsets.
3. Clamp to the output with `edge_padding`.
4. If `flip` is true and the preferred side does not fit, try the opposite side before clamping.

The overlay follows the cursor while visible. No client-side geometry polling is required.

## Surface Lifecycle

Recommended client flow:

```sh
# Client starts a layer-shell surface with namespace agent-cursor-menu.
niri msg register-cursor-overlay \
  --overlay-id agent-menu \
  --layer-namespace agent-cursor-menu \
  --anchor-virtual-cursor agent-primary \
  --side right \
  --gap 10 \
  --edge-padding 8
```

If the matching surface does not exist yet, registration can remain pending for a short timeout or until a layer surface with that namespace appears. If the surface disappears, tiri marks the overlay inactive and emits an event. If the target virtual cursor is destroyed, tiri hides or unregisters the overlay depending on a policy flag.

## Rendering Order

Render order should be:

1. Normal windows and layer-shell layers.
2. Window previews and MRU UI.
3. Cursor-anchored overlay surfaces.
4. Virtual cursors.
5. High-priority compositor UI and the hardware pointer.

This makes the overlay feel attached to the cursor while still allowing the cursor visual to remain visible above it. For modal compositor UI, tiri can temporarily suppress cursor overlays.

## Input And Focus

The default policy should be non-intrusive:

- Pointer focus can enter the overlay only when the real pointer is physically over it.
- Showing or moving the overlay does not change keyboard focus.
- Keyboard interactivity follows the layer-shell surface's existing mode, but the recommended default is no keyboard focus.
- Clicks through a virtual cursor still target the virtual cursor's pinned window, not the overlay, unless the caller explicitly moves the real pointer over the overlay or requests an interactive overlay mode.

Add an explicit `interactive` flag later if we need menu-like overlays that can take pointer and keyboard focus. The first version should optimize for visual/contextual UI next to an agent cursor.

## IPC Events

Expose enough state for clients to debug registration without owning placement:

```rust
pub enum Event {
    CursorOverlayRegistered(CursorOverlay),
    CursorOverlayUpdated(CursorOverlay),
    CursorOverlayUnregistered { overlay_id: String },
    CursorOverlayAnchorUnavailable { overlay_id: String, reason: String },
}
```

The `CursorOverlay` response should include `overlay_id`, selector, anchor, placement, visible state, resolved output name when available, and the last resolved rectangle.

## Implementation Sketch

1. Add IPC request/response structs in `niri-ipc`.
2. Add CLI commands for register, update, list, and unregister.
3. Add `CursorOverlayUi` under `src/ui/`.
4. Teach layer-shell handling to let `CursorOverlayUi` claim a matched overlay-layer surface and skip it during normal layer arrangement/rendering.
5. Reuse `window_relative_point_on_output` or extract a helper so both virtual cursors and cursor overlays resolve virtual cursor coordinates through the same path.
6. Render claimed surfaces from `Niri::render_inner` near the existing window preview and virtual cursor passes.
7. Queue redraw while an anchored virtual cursor is animating, while the hardware pointer moves, or while the overlay surface commits a new size.

## Open Questions

- Should registration match only layer-shell surfaces, or should it also support unmanaged floating `xdg_toplevel` windows later?
- Should virtual-cursor anchored overlays render when the target window is only visible inside a compositor preview?
- Should a cursor overlay be captured by screencopy by default? For agent UI, visible output capture is probably correct, but privacy-sensitive overlays may need a flag.
- Should overlay placement be a single preferred side or an ordered fallback list?

## Recommendation

Start with overlay-layer surfaces selected by namespace, anchored to either the hardware pointer or an existing virtual cursor. Keep input non-intrusive by default. Do not try to move normal application windows. This matches tiri's current architecture: clients provide Wayland surfaces, while the compositor owns layout-aware placement and rendering.
