//! The window as a pane of frosted glass: the desktop shows through, blurred by Windows'
//! acrylic backdrop, under a smoky tint, a soft sheen and a light edge.
//! There is no system frame, so the title bar with its caption buttons and the edge resizing live here.

use crate::theme::{self, white};
use eframe::egui::{self, pos2, vec2, Color32, CornerRadius, Rect, Stroke, Vec2};

const TITLE_BAR: f32 = 40.0;

pub fn window_size() -> Vec2 {
    vec2(960.0, 600.0)
}

pub fn min_window_size() -> Vec2 {
    vec2(760.0, 460.0)
}

pub fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn corner_radius(maximized: bool) -> CornerRadius {
    CornerRadius::same(if maximized { 0 } else { theme::PANEL_RADIUS })
}

#[derive(Default)]
pub struct Glass {
    /// Whether Windows blurs what is behind the window; `None` before the first frame (tests).
    blurred: Option<bool>,
    #[cfg(windows)]
    clip: Option<Vec<i32>>,
}

impl Glass {
    /// Work that needs the native window: the acrylic backdrop, once, and the rounded clip
    /// that makes the corners really transparent. Cheap when nothing changed.
    pub fn window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            if self.blurred.is_none() {
                let ok = crate::acrylic::set(frame, Some(ACRYLIC_TINT));
                if !ok {
                    tracing::warn!("no acrylic backdrop; the glass gets a denser tint instead");
                }
                self.blurred = Some(ok);
            }
            if ctx.input(|i| i.viewport().minimized == Some(true)) {
                return; // a minimised window reports itself as not maximised; keep the clip as it was
            }
            let ppp = ctx.pixels_per_point();
            let size = ctx.screen_rect().size() * ppp;
            let shape = if maximized(ctx) {
                Vec::new()
            } else {
                vec![0, 0, size.x.round() as i32, size.y.round() as i32, (theme::PANEL_RADIUS as f32 * ppp).round() as i32]
            };
            if self.clip.as_ref() != Some(&shape) {
                crate::acrylic::clip(frame, &shape);
                self.clip = Some(shape);
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (ctx, frame);
            self.blurred = Some(false);
        }
    }

    /// The tint, sheen and edge of the glass, behind everything else.
    pub fn paint(&mut self, ctx: &egui::Context, panel: Rect) {
        let radius = corner_radius(maximized(ctx));
        let painter = ctx.layer_painter(egui::LayerId::background());
        // Without the system blur the desktop would show through sharp, so the glass is denser.
        let tint = if self.blurred == Some(false) { theme::GLASS_TINT_UNBLURRED } else { theme::GLASS_TINT };
        painter.rect_filled(panel, radius, tint);
        // The sheen: light falling on the glass from above, fading out by a third of the way down.
        painter.add(sheen(panel, radius.nw as f32));
        painter.rect_stroke(panel, radius, Stroke::new(1.0_f32, theme::GLASS_EDGE), egui::StrokeKind::Inside);
        let inset = radius.nw as f32;
        let top = panel.top() + 0.5;
        painter.line_segment([pos2(panel.left() + inset, top), pos2(panel.right() - inset, top)], Stroke::new(1.0_f32, theme::GLASS_EDGE_TOP));
    }
}

/// The tint Windows lays over its acrylic blur, as RGBA.
#[cfg_attr(not(windows), allow(dead_code))]
pub const ACRYLIC_TINT: [u8; 4] = [16, 16, 20, 140];

/// A vertical white-to-clear gradient over the top third of the panel, following the
/// rounded top corners (egui has no gradient fill for rounded rectangles).
fn sheen(panel: Rect, radius: f32) -> egui::Mesh {
    let height = panel.height() / 3.0;
    let mut ys: Vec<f32> = (0..=8).map(|k| radius * k as f32 / 8.0).collect();
    ys.push(height);
    let mut mesh = egui::Mesh::default();
    for (row, dy) in ys.iter().enumerate() {
        // How far the rounded corner pulls the edge in at this height.
        let inset = if *dy < radius { radius - (radius * radius - (radius - dy).powi(2)).sqrt() } else { 0.0 };
        let color = theme::GLASS_SHEEN.gamma_multiply(1.0 - (dy / height).min(1.0));
        mesh.colored_vertex(pos2(panel.left() + inset, panel.top() + dy), color);
        mesh.colored_vertex(pos2(panel.right() - inset, panel.top() + dy), color);
        if row > 0 {
            let i = (row as u32) * 2;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    mesh
}

/// The title bar across the top of `panel`: the page name in an address field in the middle,
/// the Windows caption buttons on the right, a hairline below. Dragging it moves the window.
pub fn title_bar(ui: &mut egui::Ui, panel: Rect, page: &str) {
    let rect = Rect::from_min_size(panel.min, vec2(panel.width(), TITLE_BAR));
    let ctx = ui.ctx().clone();
    let maximized = maximized(&ctx);
    let bar = ui.interact(rect, egui::Id::new("title bar"), egui::Sense::click_and_drag());
    if bar.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bar.double_clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let painter = ui.painter_at(rect);
    // Windows-style caption buttons at the right: minimise, maximise or restore, close.
    // Kept clear of the rounded corner, which the window clip cuts away.
    let size = vec2(44.0, 28.0);
    let close = Rect::from_min_size(pos2(rect.right() - 12.0 - size.x, rect.top() + 6.0), size);
    let buttons = [
        (Glyph::Close, "Close window", egui::ViewportCommand::Close, close),
        (if maximized { Glyph::Restore } else { Glyph::Maximize }, if maximized { "Restore" } else { "Maximize" }, egui::ViewportCommand::Maximized(!maximized), close.translate(vec2(-size.x, 0.0))),
        (Glyph::Minimize, "Minimize", egui::ViewportCommand::Minimized(true), close.translate(vec2(-2.0 * size.x, 0.0))),
    ];
    for (glyph, name, command, button) in buttons {
        let response = ui.interact(button, egui::Id::new(("caption button", name)), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let (fill, stroke) = match (glyph, response.is_pointer_button_down_on(), response.hovered()) {
            (Glyph::Close, true, _) => (theme::CLOSE_PRESSED, Color32::WHITE),
            (Glyph::Close, _, true) => (theme::CLOSE_HOVER, Color32::WHITE),
            (_, true, _) => (white(30), Color32::WHITE),
            (_, _, true) => (white(18), Color32::WHITE),
            _ => (Color32::TRANSPARENT, theme::TEXT),
        };
        painter.rect_filled(button, CornerRadius::same(6), fill);
        paint_glyph(&painter, glyph, button.center(), Stroke::new(1.0_f32, stroke));
        if response.clicked() {
            ctx.send_viewport_cmd(command);
        }
    }
    let field = Rect::from_center_size(rect.center(), vec2((rect.width() * 0.36).clamp(180.0, 340.0), 24.0));
    painter.rect(field, CornerRadius::same(12), white(7), Stroke::new(1.0_f32, white(24)), egui::StrokeKind::Inside);
    let galley = painter.layout(page.to_string(), egui::FontId::proportional(12.0), theme::TEXT_DIM, field.width() - 24.0);
    painter.galley(field.center() - galley.size() / 2.0, galley, theme::TEXT_DIM);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, white(24)));
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// The caption glyphs as thin 10 px lines, like Windows draws them.
fn paint_glyph(painter: &egui::Painter, glyph: Glyph, center: egui::Pos2, stroke: Stroke) {
    let half = 5.0;
    match glyph {
        Glyph::Minimize => {
            painter.hline(center.x - half..=center.x + half, center.y, stroke);
        }
        Glyph::Maximize => {
            painter.rect_stroke(Rect::from_center_size(center, Vec2::splat(2.0 * half)), CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
        }
        Glyph::Restore => {
            let front = Rect::from_min_size(center + vec2(-half, -half + 2.0), Vec2::splat(2.0 * half - 2.0));
            painter.rect_stroke(front, CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            // The window behind: only its top and right edge show.
            let (left, top, right, bottom) = (front.left() + 2.0, front.top() - 2.0, front.right() + 2.0, front.bottom() - 2.0);
            painter.line(vec![pos2(left, front.top()), pos2(left, top), pos2(right, top), pos2(right, bottom), pos2(front.right(), bottom)], stroke);
        }
        Glyph::Close => {
            painter.line_segment([center + vec2(-half, -half), center + vec2(half, half)], stroke);
            painter.line_segment([center + vec2(-half, half), center + vec2(half, -half)], stroke);
        }
    }
}

/// The space under the title bar, for the page.
pub fn content_rect(panel: Rect) -> Rect {
    Rect::from_min_max(panel.min + vec2(26.0, TITLE_BAR + 18.0), panel.max - vec2(26.0, 16.0))
}

/// Resizing from the panel's edges, since there is no system frame. Call after the page,
/// so its cursor wins over the widgets'.
pub fn resize_edges(ctx: &egui::Context, panel: Rect) {
    use egui::{CursorIcon, ResizeDirection as D};
    if maximized(ctx) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    const GRIP: f32 = 6.0;
    const CORNER: f32 = 22.0;
    // Only inside the panel: outside it the window is clipped away and gets no input.
    if !panel.contains(pos) {
        return;
    }
    let (left, right) = (pos.x - panel.left(), panel.right() - pos.x);
    let (top, bottom) = (pos.y - panel.top(), panel.bottom() - pos.y);
    let (w, e, n, s) = (left < GRIP, right < GRIP, top < GRIP, bottom < GRIP);
    // In the rounded corners the edge curves away, so the grip there is larger.
    let (cw, ce, cn, cs) = (left < CORNER, right < CORNER, top < CORNER, bottom < CORNER);
    let direction = match () {
        _ if cn && cw && (n || w || left + top < CORNER * 1.3) => D::NorthWest,
        _ if cn && ce && (n || e || right + top < CORNER * 1.3) => D::NorthEast,
        _ if cs && cw && (s || w || left + bottom < CORNER * 1.3) => D::SouthWest,
        _ if cs && ce && (s || e || right + bottom < CORNER * 1.3) => D::SouthEast,
        _ if n => D::North,
        _ if s => D::South,
        _ if w => D::West,
        _ if e => D::East,
        _ => return,
    };
    ctx.set_cursor_icon(match direction {
        D::North | D::South => CursorIcon::ResizeVertical,
        D::West | D::East => CursorIcon::ResizeHorizontal,
        D::NorthWest | D::SouthEast => CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// `top` (premultiplied) over an opaque `below`, blended in gamma space as egui does.
    fn over(below: Color32, top: Color32) -> Color32 {
        let a = top.a() as f32 / 255.0;
        let mix = |t: u8, b: u8| (t as f32 + b as f32 * (1.0 - a)).round().min(255.0) as u8;
        Color32::from_rgb(mix(top.r(), below.r()), mix(top.g(), below.g()), mix(top.b(), below.b()))
    }

    /// Readability over what is behind the glass, on the brightest spot: under the sheen,
    /// inside a selected tile, and in dialogs. Every text colour keeps 4.5:1 over a light grey
    /// desktop (#C8C8C8). The glass was made clearer on request, so over pure white the text
    /// drops below that; the second limit only catches it getting worse.
    #[test]
    fn text_stays_readable_over_a_light_desktop() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let visuals = ctx.style().visuals.clone();
        let [r, g, b, a] = ACRYLIC_TINT;
        for (desktop, minimum) in [(Color32::from_gray(200), 4.5), (Color32::WHITE, 3.8)] {
            let blurred = over(desktop, Color32::from_rgba_unmultiplied(r, g, b, a));
            let glass = over(over(blurred, theme::GLASS_TINT), theme::GLASS_SHEEN);
            let unblurred = over(over(desktop, theme::GLASS_TINT_UNBLURRED), theme::GLASS_SHEEN);
            for (place, background) in [("glass", glass), ("glass without blur", unblurred)] {
                for (name, text, on) in [
                    ("TEXT", theme::TEXT, over(background, white(16))),
                    ("TEXT_TITLE", theme::TEXT_TITLE, over(background, white(6))),
                    ("TEXT_DIM", theme::TEXT_DIM, over(background, white(6))),
                    ("dialog text", visuals.text_color(), over(background, visuals.window_fill)),
                    ("weak dialog text", visuals.weak_text_color(), over(background, visuals.window_fill)),
                ] {
                    let ratio = theme::contrast(text, on);
                    assert!(ratio >= minimum, "{name} on {place} over {desktop:?}: {ratio:.2}, needs {minimum}");
                }
            }
            println!("desktop {desktop:?}: glass {glass:?}, TEXT_DIM {:.2}:1", theme::contrast(theme::TEXT_DIM, over(glass, white(6))));
        }
    }
    #[test]
    fn the_sheen_stays_inside_the_rounded_corners() {
        let panel = Rect::from_min_size(pos2(0.0, 0.0), window_size());
        let r = theme::PANEL_RADIUS as f32;
        for v in sheen(panel, r).vertices {
            // Distance from the nearest top corner's centre must not exceed the radius.
            let cx = v.pos.x.clamp(r, panel.right() - r);
            let cy = v.pos.y.max(r);
            assert!(vec2(v.pos.x - cx, v.pos.y - cy).length() <= r + 0.01, "{:?}", v.pos);
        }
    }
}
