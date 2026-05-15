use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use niri_ipc::{
    CursorOverlay, CursorOverlayAnchor, CursorOverlayPlacement, CursorOverlayRect,
    CursorOverlayRegister, CursorOverlaySide, CursorOverlayUpdate,
};
use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::{LayerMap, LayerSurface, WindowSurfaceType};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::layer::mapped::LayerSurfaceRenderElement;
use crate::niri::Niri;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::RenderCtx;
use crate::utils::output_size;

#[derive(Debug, Default)]
pub struct CursorOverlayUi {
    overlays: HashMap<String, RegisteredCursorOverlay>,
}

#[derive(Debug)]
struct RegisteredCursorOverlay {
    overlay_id: String,
    layer_namespace: String,
    anchor: CursorOverlayAnchor,
    placement: CursorOverlayPlacement,
    visible: bool,
    interactive: bool,
    keyboard_focus: bool,
    z_index: i32,
    last_resolved: RefCell<Option<ResolvedOverlay>>,
}

#[derive(Debug, Clone)]
struct ResolvedOverlay {
    output: String,
    rect: Rectangle<f64, Logical>,
}

#[derive(Debug)]
pub enum CursorOverlayError {
    AlreadyExists(String),
    NotFound(String),
}

impl std::fmt::Display for CursorOverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "cursor overlay {id:?} already exists"),
            Self::NotFound(id) => write!(f, "cursor overlay {id:?} not found"),
        }
    }
}

impl std::error::Error for CursorOverlayError {}

impl CursorOverlayUi {
    pub fn list(&self) -> Vec<CursorOverlay> {
        let mut overlays = self
            .overlays
            .values()
            .map(RegisteredCursorOverlay::to_ipc)
            .collect::<Vec<_>>();
        overlays.sort_by(|a, b| {
            a.z_index
                .cmp(&b.z_index)
                .then_with(|| a.overlay_id.cmp(&b.overlay_id))
        });
        overlays
    }

    pub fn register(
        &mut self,
        register: CursorOverlayRegister,
    ) -> Result<CursorOverlay, CursorOverlayError> {
        if !register.replace_existing && self.overlays.contains_key(&register.overlay_id) {
            return Err(CursorOverlayError::AlreadyExists(register.overlay_id));
        }

        let overlay = RegisteredCursorOverlay {
            overlay_id: register.overlay_id.clone(),
            layer_namespace: register.layer_namespace,
            anchor: register.anchor,
            placement: register.placement,
            visible: register.visible.unwrap_or(true),
            interactive: register.interactive.unwrap_or(false),
            keyboard_focus: register.keyboard_focus.unwrap_or(false),
            z_index: register.z_index.unwrap_or_default(),
            last_resolved: RefCell::new(None),
        };
        let rv = overlay.to_ipc();
        self.overlays.insert(register.overlay_id, overlay);
        Ok(rv)
    }

    pub fn update(
        &mut self,
        update: CursorOverlayUpdate,
    ) -> Result<CursorOverlay, CursorOverlayError> {
        let overlay = self
            .overlays
            .get_mut(&update.overlay_id)
            .ok_or_else(|| CursorOverlayError::NotFound(update.overlay_id.clone()))?;

        if let Some(layer_namespace) = update.layer_namespace {
            overlay.layer_namespace = layer_namespace;
            *overlay.last_resolved.borrow_mut() = None;
        }
        if let Some(anchor) = update.anchor {
            overlay.anchor = anchor;
            *overlay.last_resolved.borrow_mut() = None;
        }
        if let Some(placement) = update.placement {
            overlay.placement = placement;
            *overlay.last_resolved.borrow_mut() = None;
        }
        if let Some(visible) = update.visible {
            overlay.visible = visible;
        }
        if let Some(interactive) = update.interactive {
            overlay.interactive = interactive;
        }
        if let Some(keyboard_focus) = update.keyboard_focus {
            overlay.keyboard_focus = keyboard_focus;
        }
        if let Some(z_index) = update.z_index {
            overlay.z_index = z_index;
        }

        Ok(overlay.to_ipc())
    }

    pub fn unregister(&mut self, overlay_id: &str) -> Result<(), CursorOverlayError> {
        self.overlays
            .remove(overlay_id)
            .map(|_| ())
            .ok_or_else(|| CursorOverlayError::NotFound(overlay_id.to_owned()))
    }

    pub fn is_claimed(&self, surface: &LayerSurface) -> bool {
        surface.layer() == Layer::Overlay
            && self
                .overlays
                .values()
                .any(|overlay| overlay.layer_namespace == surface.namespace())
    }

    pub fn remove_namespace(&mut self, namespace: &str) {
        for overlay in self.overlays.values_mut() {
            if overlay.layer_namespace == namespace {
                *overlay.last_resolved.borrow_mut() = None;
            }
        }
    }

    pub fn wants_keyboard_focus(&self, surface: &LayerSurface) -> bool {
        surface.layer() == Layer::Overlay
            && self.overlays.values().any(|overlay| {
                overlay.visible
                    && overlay.keyboard_focus
                    && overlay.layer_namespace == surface.namespace()
            })
    }

    pub fn surface_under(
        &self,
        niri: &Niri,
        output: &Output,
        layer_map: &LayerMap,
        pos_within_output: Point<f64, Logical>,
    ) -> Option<((WlSurface, Point<f64, Logical>), LayerSurface)> {
        let mut overlays = self.overlays.values().collect::<Vec<_>>();
        overlays.sort_by(|a, b| {
            b.z_index
                .cmp(&a.z_index)
                .then_with(|| a.overlay_id.cmp(&b.overlay_id))
        });

        let mut checked = HashSet::new();
        for overlay in overlays {
            if !overlay.visible || !overlay.interactive || !checked.insert(&overlay.layer_namespace)
            {
                continue;
            }

            let Some((layer_surface, geo)) = layer_map.layers_on(Layer::Overlay).find_map(|s| {
                if s.namespace() != overlay.layer_namespace {
                    return None;
                }
                let geo = layer_map.layer_geometry(s)?;
                Some((s, geo))
            }) else {
                continue;
            };

            let anchor = overlay.anchor_point(niri, output)?;
            let rect = overlay.placement_rect(anchor, geo.size.to_f64(), output_size(output));
            if !rect_contains(rect, pos_within_output) {
                continue;
            }

            let pos_within_overlay = pos_within_output - rect.loc;
            let Some((surface, surface_origin)) = under_from_surface_tree(
                layer_surface.wl_surface(),
                pos_within_overlay,
                Point::new(0, 0),
                WindowSurfaceType::ALL,
            ) else {
                continue;
            };

            return Some((
                (surface, surface_origin.to_f64() + rect.loc),
                layer_surface.clone(),
            ));
        }

        None
    }

    pub fn render_output<R: NiriRenderer>(
        &self,
        niri: &Niri,
        output: &Output,
        layer_map: &LayerMap,
        mut ctx: RenderCtx<R>,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        let mut overlays = self.overlays.values().collect::<Vec<_>>();
        overlays.sort_by(|a, b| {
            b.z_index
                .cmp(&a.z_index)
                .then_with(|| a.overlay_id.cmp(&b.overlay_id))
        });

        let mut rendered = HashSet::new();
        for overlay in overlays {
            if !overlay.visible || !rendered.insert(&overlay.layer_namespace) {
                continue;
            }

            let Some((mapped, geo)) = layer_map.layers_on(Layer::Overlay).find_map(|s| {
                if s.namespace() != overlay.layer_namespace {
                    return None;
                }
                let mapped = niri.mapped_layer_surfaces.get(s)?;
                let geo = layer_map.layer_geometry(s)?;
                Some((mapped, geo))
            }) else {
                continue;
            };

            let Some(anchor) = overlay.anchor_point(niri, output) else {
                continue;
            };

            let output_size = output_size(output);
            let size = geo.size.to_f64();
            let rect = overlay.placement_rect(anchor, size, output_size);
            *overlay.last_resolved.borrow_mut() = Some(ResolvedOverlay {
                output: output.name(),
                rect,
            });
            let xray_pos = XrayPos::new(rect.loc, 1.);
            mapped.render_popups(ctx.r(), None, rect.loc, xray_pos, push);
            mapped.render_normal(ctx.r(), None, rect.loc, xray_pos, push);
        }
    }
}

impl RegisteredCursorOverlay {
    fn to_ipc(&self) -> CursorOverlay {
        CursorOverlay {
            overlay_id: self.overlay_id.clone(),
            layer_namespace: self.layer_namespace.clone(),
            anchor: self.anchor.clone(),
            placement: self.placement,
            visible: self.visible,
            interactive: self.interactive,
            keyboard_focus: self.keyboard_focus,
            z_index: self.z_index,
            resolved_output: self
                .last_resolved
                .borrow()
                .as_ref()
                .map(|resolved| resolved.output.clone()),
            resolved_rect: self
                .last_resolved
                .borrow()
                .as_ref()
                .map(|resolved| CursorOverlayRect {
                    x: resolved.rect.loc.x,
                    y: resolved.rect.loc.y,
                    width: resolved.rect.size.w,
                    height: resolved.rect.size.h,
                }),
        }
    }

    fn anchor_point(&self, niri: &Niri, output: &Output) -> Option<Point<f64, Logical>> {
        match &self.anchor {
            CursorOverlayAnchor::HardwarePointer => {
                let pointer_pos = niri
                    .tablet_cursor_location
                    .unwrap_or_else(|| niri.seat.get_pointer().unwrap().current_location());
                let output_geo = niri.global_space.output_geometry(output)?;
                if !output_geo.contains(pointer_pos.to_i32_round()) {
                    return None;
                }
                Some(pointer_pos - output_geo.loc.to_f64())
            }
            CursorOverlayAnchor::VirtualCursor { cursor_id } => niri
                .virtual_cursor_ui
                .visual_point_on_output(niri, output, cursor_id),
        }
    }

    fn placement_rect(
        &self,
        anchor: Point<f64, Logical>,
        size: Size<f64, Logical>,
        output_size: Size<f64, Logical>,
    ) -> Rectangle<f64, Logical> {
        let preferred = place_on_side(anchor, size, self.placement, self.placement.side);
        let rect =
            if self.placement.flip && !fits(preferred, output_size, self.placement.edge_padding) {
                let side = opposite_side(self.placement.side);
                let flipped = place_on_side(anchor, size, self.placement, side);
                if fits(flipped, output_size, self.placement.edge_padding) {
                    flipped
                } else {
                    preferred
                }
            } else {
                preferred
            };

        clamp_rect(rect, output_size, self.placement.edge_padding)
    }
}

fn place_on_side(
    anchor: Point<f64, Logical>,
    size: Size<f64, Logical>,
    placement: CursorOverlayPlacement,
    side: CursorOverlaySide,
) -> Rectangle<f64, Logical> {
    let mut loc = match side {
        CursorOverlaySide::Right => Point::new(
            anchor.x + placement.gap,
            align_axis(anchor.y, size.h, placement.align),
        ),
        CursorOverlaySide::Left => Point::new(
            anchor.x - placement.gap - size.w,
            align_axis(anchor.y, size.h, placement.align),
        ),
        CursorOverlaySide::Above => Point::new(
            align_axis(anchor.x, size.w, placement.align),
            anchor.y - placement.gap - size.h,
        ),
        CursorOverlaySide::Below => Point::new(
            align_axis(anchor.x, size.w, placement.align),
            anchor.y + placement.gap,
        ),
    };
    loc.x += placement.offset_x;
    loc.y += placement.offset_y;
    Rectangle::new(loc, size)
}

fn align_axis(anchor: f64, size: f64, align: niri_ipc::CursorOverlayAlign) -> f64 {
    match align {
        niri_ipc::CursorOverlayAlign::Start => anchor,
        niri_ipc::CursorOverlayAlign::Center => anchor - size / 2.,
        niri_ipc::CursorOverlayAlign::End => anchor - size,
    }
}

fn opposite_side(side: CursorOverlaySide) -> CursorOverlaySide {
    match side {
        CursorOverlaySide::Right => CursorOverlaySide::Left,
        CursorOverlaySide::Left => CursorOverlaySide::Right,
        CursorOverlaySide::Above => CursorOverlaySide::Below,
        CursorOverlaySide::Below => CursorOverlaySide::Above,
    }
}

fn fits(rect: Rectangle<f64, Logical>, output_size: Size<f64, Logical>, edge_padding: f64) -> bool {
    rect.loc.x >= edge_padding
        && rect.loc.y >= edge_padding
        && rect.loc.x + rect.size.w <= output_size.w - edge_padding
        && rect.loc.y + rect.size.h <= output_size.h - edge_padding
}

fn rect_contains(rect: Rectangle<f64, Logical>, point: Point<f64, Logical>) -> bool {
    point.x >= rect.loc.x
        && point.y >= rect.loc.y
        && point.x < rect.loc.x + rect.size.w
        && point.y < rect.loc.y + rect.size.h
}

fn clamp_rect(
    mut rect: Rectangle<f64, Logical>,
    output_size: Size<f64, Logical>,
    edge_padding: f64,
) -> Rectangle<f64, Logical> {
    let max_x = (output_size.w - edge_padding - rect.size.w).max(edge_padding);
    let max_y = (output_size.h - edge_padding - rect.size.h).max(edge_padding);
    rect.loc.x = rect.loc.x.clamp(edge_padding, max_x);
    rect.loc.y = rect.loc.y.clamp(edge_padding, max_y);
    rect
}
