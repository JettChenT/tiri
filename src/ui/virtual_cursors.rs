use std::cell::RefCell;
use std::collections::HashMap;

use niri_ipc::{
    RgbaColor, VirtualCursor, VirtualCursorAnimation, VirtualCursorAppearance, VirtualCursorCreate,
    VirtualCursorCurve, VirtualCursorShape, VirtualCursorSource, VirtualCursorUpdate,
};
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::Color32F;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};

use crate::animation::{Animation, Clock, CubicBezier, Curve};
use crate::cursor::XCursor;
use crate::niri::Niri;
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::window::mapped::MappedId;

niri_render_elements! {
    VirtualCursorRenderElement<R> => {
        SolidColor = SolidColorRenderElement,
        Themed = MemoryRenderBufferRenderElement<R>,
    }
}

#[derive(Debug, Default)]
pub struct VirtualCursorUi {
    cursors: HashMap<String, PinnedCursor>,
}

#[derive(Debug)]
struct PinnedCursor {
    cursor_id: String,
    window_id: MappedId,
    position: Point<f64, Logical>,
    appearance: VirtualCursorAppearance,
    animation_config: VirtualCursorAnimation,
    animation: Option<CursorAnimation>,
    visible: bool,
    z_index: i32,
    buffers: RefCell<Vec<SolidColorBuffer>>,
}

#[derive(Debug)]
struct CursorAnimation {
    x: Animation,
    y: Animation,
}

#[derive(Debug)]
pub enum VirtualCursorError {
    AlreadyExists(String),
    NotFound(String),
}

impl std::fmt::Display for VirtualCursorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists(id) => write!(f, "virtual cursor {id:?} already exists"),
            Self::NotFound(id) => write!(f, "virtual cursor {id:?} not found"),
        }
    }
}

impl std::error::Error for VirtualCursorError {}

impl VirtualCursorUi {
    pub fn list(&self) -> Vec<VirtualCursor> {
        let mut cursors = self
            .cursors
            .values()
            .map(PinnedCursor::to_ipc)
            .collect::<Vec<_>>();
        cursors.sort_by(|a, b| {
            a.z_index
                .cmp(&b.z_index)
                .then_with(|| a.cursor_id.cmp(&b.cursor_id))
        });
        cursors
    }

    pub fn create(
        &mut self,
        clock: Clock,
        create: VirtualCursorCreate,
    ) -> Result<VirtualCursor, VirtualCursorError> {
        if !create.replace_existing && self.cursors.contains_key(&create.cursor_id) {
            return Err(VirtualCursorError::AlreadyExists(create.cursor_id));
        }

        let cursor = PinnedCursor {
            cursor_id: create.cursor_id.clone(),
            window_id: MappedId::from_raw(create.window_id),
            position: Point::new(create.x, create.y),
            appearance: create.appearance.unwrap_or_default(),
            animation_config: create.animation.unwrap_or_default(),
            animation: None,
            visible: create.visible.unwrap_or(true),
            z_index: create.z_index.unwrap_or_default(),
            buffers: Default::default(),
        };
        let rv = cursor.to_ipc();
        self.cursors.insert(create.cursor_id, cursor);

        // Keep the clock live in the same frame as creation when callers immediately move.
        let _ = clock;
        Ok(rv)
    }

    pub fn update(
        &mut self,
        clock: Clock,
        update: VirtualCursorUpdate,
    ) -> Result<VirtualCursor, VirtualCursorError> {
        let cursor = self
            .cursors
            .get_mut(&update.cursor_id)
            .ok_or_else(|| VirtualCursorError::NotFound(update.cursor_id.clone()))?;

        if let Some(window_id) = update.window_id {
            cursor.window_id = MappedId::from_raw(window_id);
        }
        if let Some(appearance) = update.appearance {
            cursor.appearance = appearance;
        }
        if let Some(animation) = update.animation {
            cursor.animation_config = animation;
        }
        if let Some(visible) = update.visible {
            cursor.visible = visible;
        }
        if let Some(z_index) = update.z_index {
            cursor.z_index = z_index;
        }
        match (update.x, update.y) {
            (Some(x), Some(y)) => cursor.move_to(clock, Point::new(x, y), update.animation),
            (Some(x), None) => {
                let current = cursor.destination();
                cursor.move_to(clock, Point::new(x, current.y), update.animation);
            }
            (None, Some(y)) => {
                let current = cursor.destination();
                cursor.move_to(clock, Point::new(current.x, y), update.animation);
            }
            (None, None) => {}
        }

        Ok(cursor.to_ipc())
    }

    pub fn destroy(&mut self, cursor_id: &str) -> Result<(), VirtualCursorError> {
        self.cursors
            .remove(cursor_id)
            .map(|_| ())
            .ok_or_else(|| VirtualCursorError::NotFound(cursor_id.to_owned()))
    }

    pub fn move_cursor(
        &mut self,
        clock: Clock,
        cursor_id: &str,
        x: f64,
        y: f64,
        duration_ms: Option<u32>,
    ) -> Result<VirtualCursor, VirtualCursorError> {
        let cursor = self
            .cursors
            .get_mut(cursor_id)
            .ok_or_else(|| VirtualCursorError::NotFound(cursor_id.to_owned()))?;
        let animation = duration_ms.map(|duration_ms| VirtualCursorAnimation {
            duration_ms,
            ..cursor.animation_config
        });
        cursor.move_to(clock, Point::new(x, y), animation);
        Ok(cursor.to_ipc())
    }

    pub fn action_target(&self, cursor_id: &str) -> Result<(u64, Point<f64, Logical>), String> {
        let cursor = self
            .cursors
            .get(cursor_id)
            .ok_or_else(|| format!("virtual cursor {cursor_id:?} not found"))?;
        let position = cursor.destination();
        Ok((cursor.window_id.get(), position))
    }

    pub fn remove_window(&mut self, id: MappedId) {
        self.cursors.retain(|_, cursor| cursor.window_id != id);
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.cursors
            .values()
            .any(|cursor| cursor.animation.is_some())
    }

    pub fn advance_animations(&mut self) {
        for cursor in self.cursors.values_mut() {
            if cursor.animation.as_ref().is_some_and(|anim| anim.is_done()) {
                cursor.animation = None;
            }
        }
    }

    pub fn render_output<R: NiriRenderer>(
        &self,
        niri: &Niri,
        renderer: &mut R,
        output: &Output,
        animation_millis: u32,
        push: &mut dyn FnMut(VirtualCursorRenderElement<R>),
    ) {
        let mut cursors = self.cursors.values().collect::<Vec<_>>();
        // Output render order is top-to-bottom, so higher z-index cursors go first.
        cursors.sort_by(|a, b| {
            b.z_index
                .cmp(&a.z_index)
                .then_with(|| a.cursor_id.cmp(&b.cursor_id))
        });

        for cursor in cursors {
            if !cursor.visible {
                continue;
            }
            let Some(pos) = niri.window_relative_point_on_output(
                output,
                cursor.window_id,
                cursor.visual_position(),
            ) else {
                continue;
            };
            cursor.render(niri, renderer, output, pos, animation_millis, push);
        }
    }
}

impl PinnedCursor {
    fn to_ipc(&self) -> VirtualCursor {
        let position = self.destination();
        VirtualCursor {
            cursor_id: self.cursor_id.clone(),
            window_id: self.window_id.get(),
            x: position.x,
            y: position.y,
            appearance: self.appearance.clone(),
            animation: self.animation_config,
            visible: self.visible,
            z_index: self.z_index,
        }
    }

    fn destination(&self) -> Point<f64, Logical> {
        self.position
    }

    fn visual_position(&self) -> Point<f64, Logical> {
        self.animation.as_ref().map_or(self.position, |anim| {
            Point::new(anim.x.clamped_value(), anim.y.clamped_value())
        })
    }

    fn move_to(
        &mut self,
        clock: Clock,
        target: Point<f64, Logical>,
        animation: Option<VirtualCursorAnimation>,
    ) {
        let config = animation.unwrap_or(self.animation_config);
        self.animation_config = config;

        let from = self.visual_position();
        self.position = target;

        if config.duration_ms == 0 || from == target {
            self.animation = None;
            return;
        }

        self.animation = Some(CursorAnimation {
            x: make_animation(clock.clone(), from.x, target.x, config),
            y: make_animation(clock, from.y, target.y, config),
        });
    }

    fn render<R: NiriRenderer>(
        &self,
        niri: &Niri,
        renderer: &mut R,
        output: &Output,
        point: Point<f64, Logical>,
        animation_millis: u32,
        push: &mut dyn FnMut(VirtualCursorRenderElement<R>),
    ) {
        match &self.appearance.source {
            VirtualCursorSource::Theme { icon } => {
                self.render_themed(
                    niri,
                    renderer,
                    output,
                    point,
                    icon.as_deref(),
                    animation_millis,
                    push,
                );
            }
            VirtualCursorSource::Builtin { shape } => {
                self.render_builtin(point, *shape, push);
            }
        }
    }

    fn render_themed<R: NiriRenderer>(
        &self,
        niri: &Niri,
        renderer: &mut R,
        output: &Output,
        point: Point<f64, Logical>,
        icon: Option<&str>,
        animation_millis: u32,
        push: &mut dyn FnMut(VirtualCursorRenderElement<R>),
    ) {
        let cursor_scale = output.current_scale().integer_scale();
        let default_cursor = || niri.cursor_manager.get_default_cursor(cursor_scale);
        let (cache_name, cursor) = match icon {
            Some(name) => niri
                .cursor_manager
                .get_cursor_with_icon_name(name, cursor_scale, Some(self.appearance.size))
                .map(|cursor| (name, cursor))
                .unwrap_or_else(|| ("default", default_cursor())),
            None => ("default", default_cursor()),
        };

        let (idx, frame) = cursor.frame(animation_millis);
        let hotspot = XCursor::hotspot(frame).to_logical(cursor_scale);
        let output_scale = Scale::from(output.current_scale().fractional_scale());
        let loc = (point - hotspot.to_f64()).to_physical_precise_round(output_scale);
        let texture = niri
            .cursor_texture_cache
            .get_named(cache_name, cursor_scale, &cursor, idx);

        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc,
            &texture,
            None,
            None,
            None,
            Kind::Cursor,
        ) {
            Ok(element) => push(element.into()),
            Err(err) => warn!("error importing a virtual cursor texture: {err:?}"),
        }
    }

    fn render_builtin<R: NiriRenderer>(
        &self,
        point: Point<f64, Logical>,
        shape: VirtualCursorShape,
        push: &mut dyn FnMut(VirtualCursorRenderElement<R>),
    ) {
        let rects = self.rects(point, shape);
        let mut buffers = self.buffers.borrow_mut();
        if buffers.len() < rects.len() {
            buffers.resize_with(rects.len(), SolidColorBuffer::default);
        }

        for (idx, (rect, color)) in rects.into_iter().enumerate() {
            let buffer = &mut buffers[idx];
            buffer.update(rect.size, color);
            push(
                SolidColorRenderElement::from_buffer(
                    buffer,
                    rect.loc,
                    self.appearance.opacity.clamp(0., 1.),
                    Kind::Cursor,
                )
                .into(),
            );
        }
    }

    fn rects(
        &self,
        point: Point<f64, Logical>,
        shape: VirtualCursorShape,
    ) -> Vec<(Rectangle<f64, Logical>, Color32F)> {
        let size = f64::from(self.appearance.size.max(4));
        let stroke = (size / 7.).clamp(2., 5.);
        let half = size / 2.;
        let color = color32(self.appearance.color);
        let outline = color32(self.appearance.outline_color);

        match shape {
            VirtualCursorShape::Ring => {
                let outer =
                    Rectangle::new(point - Point::new(half, half), Size::from((size, size)));
                vec![
                    (
                        Rectangle::new(outer.loc, Size::from((size, stroke))),
                        outline,
                    ),
                    (
                        Rectangle::new(
                            Point::new(outer.loc.x, outer.loc.y + size - stroke),
                            Size::from((size, stroke)),
                        ),
                        outline,
                    ),
                    (
                        Rectangle::new(outer.loc, Size::from((stroke, size))),
                        outline,
                    ),
                    (
                        Rectangle::new(
                            Point::new(outer.loc.x + size - stroke, outer.loc.y),
                            Size::from((stroke, size)),
                        ),
                        outline,
                    ),
                    (
                        Rectangle::new(
                            point - Point::new(stroke, stroke),
                            Size::from((stroke * 2., stroke * 2.)),
                        ),
                        color,
                    ),
                ]
            }
            VirtualCursorShape::Crosshair => vec![
                (
                    Rectangle::new(
                        Point::new(point.x - half, point.y - stroke / 2.),
                        Size::from((size, stroke)),
                    ),
                    outline,
                ),
                (
                    Rectangle::new(
                        Point::new(point.x - stroke / 2., point.y - half),
                        Size::from((stroke, size)),
                    ),
                    outline,
                ),
                (
                    Rectangle::new(
                        point - Point::new(stroke, stroke),
                        Size::from((stroke * 2., stroke * 2.)),
                    ),
                    color,
                ),
            ],
            VirtualCursorShape::Dot => vec![(
                Rectangle::new(point - Point::new(half, half), Size::from((size, size))),
                color,
            )],
            VirtualCursorShape::Arrow => vec![
                (
                    Rectangle::new(point, Size::from((stroke * 2., size))),
                    outline,
                ),
                (
                    Rectangle::new(point, Size::from((size * 0.7, stroke * 2.))),
                    outline,
                ),
                (
                    Rectangle::new(
                        point + Point::new(stroke * 1.5, stroke * 1.5),
                        Size::from((stroke * 2., size * 0.65)),
                    ),
                    color,
                ),
            ],
        }
    }
}

impl CursorAnimation {
    fn is_done(&self) -> bool {
        self.x.is_done() && self.y.is_done()
    }
}

fn make_animation(clock: Clock, from: f64, to: f64, config: VirtualCursorAnimation) -> Animation {
    let curve = match config.curve {
        VirtualCursorCurve::Linear => Curve::Linear,
        VirtualCursorCurve::EaseOutCubic => Curve::EaseOutCubic,
        VirtualCursorCurve::EaseInOutCubic => {
            Curve::CubicBezier(CubicBezier::new(0.65, 0., 0.35, 1.))
        }
    };
    Animation::ease(clock, from, to, 0., u64::from(config.duration_ms), curve)
}

fn color32(color: RgbaColor) -> Color32F {
    let a = color.a.clamp(0., 1.);
    Color32F::from([
        color.r.clamp(0., 1.) * a,
        color.g.clamp(0., 1.) * a,
        color.b.clamp(0., 1.) * a,
        a,
    ])
}
