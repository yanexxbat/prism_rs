//! The window as a pane of frosted glass: the desktop shows through, blurred by Windows'
//! acrylic backdrop, under a smoky tint, a soft sheen and a light edge.
//! There is no system frame, so the Mac-style title bar and the edge resizing live here.

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

/// The Mac-style title bar across the top of `panel`: traffic lights on the left, the page
/// name in an address field in the middle, a hairline below. Dragging it moves the window.
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
    let lights = Rect::from_min_size(rect.min + vec2(14.0, 10.0), vec2(64.0, 20.0));
    let show_symbols = ui.rect_contains_pointer(lights);
    let commands = [
        ("Close window", egui::ViewportCommand::Close),
        ("Minimize", egui::ViewportCommand::Minimized(true)),
        ("Maximize", egui::ViewportCommand::Maximized(!maximized)),
    ];
    for (i, ((name, command), color)) in commands.into_iter().zip(theme::TRAFFIC_LIGHTS).enumerate() {
        let center = pos2(rect.left() + 24.0 + 20.0 * i as f32, rect.center().y);
        let response = ui.interact(Rect::from_center_size(center, Vec2::splat(16.0)), egui::Id::new(name), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let ring = Color32::from_rgb((color.r() as f32 * 0.78) as u8, (color.g() as f32 * 0.78) as u8, (color.b() as f32 * 0.78) as u8);
        painter.circle(center, 6.5, color, Stroke::new(0.8_f32, ring));
        // A glossy highlight: a soft cap and a small bright point near the top.
        painter.circle_filled(center + vec2(0.0, -2.2), 3.6, white(50));
        painter.circle_filled(center + vec2(-1.4, -2.6), 1.5, white(170));
        if show_symbols {
            let symbol = ["×", "−", "+"][i];
            painter.text(center, egui::Align2::CENTER_CENTER, symbol, egui::FontId::proportional(10.0), Color32::from_black_alpha(150));
        }
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

    /// Lesbarkeit zuerst: the desktop behind the glass can be pure white. Every text colour
    /// keeps 4.5:1 there, on the brightest spot: under the sheen,
    /// inside a selected tile, and in dialogs.
    #[test]
    fn text_stays_readable_over_a_white_desktop() {
        let [r, g, b, a] = ACRYLIC_TINT;
        let blurred = over(Color32::WHITE, Color32::from_rgba_unmultiplied(r, g, b, a));
        let glass = |tint| over(over(blurred, tint), theme::GLASS_SHEEN);
        let unblurred = over(over(Color32::WHITE, theme::GLASS_TINT_UNBLURRED), theme::GLASS_SHEEN);
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let visuals = ctx.style().visuals.clone();
        for (place, background) in [("glass", glass(theme::GLASS_TINT)), ("glass without blur", unblurred)] {
            let selected_tile = over(background, white(16));
            for (name, text, on) in [
                ("TEXT", theme::TEXT, selected_tile),
                ("TEXT_TITLE", theme::TEXT_TITLE, over(background, white(6))),
                ("TEXT_DIM", theme::TEXT_DIM, over(background, white(6))),
                ("dialog text", visuals.text_color(), over(background, visuals.window_fill)),
                ("weak dialog text", visuals.weak_text_color(), over(background, visuals.window_fill)),
            ] {
                let ratio = theme::contrast(text, on);
                assert!(ratio >= 4.5, "{name} on {place} {on:?}: {ratio:.2}");
            }
        }
        println!("white desktop through the glass: {:?}, TEXT_DIM {:.2}:1", glass(theme::GLASS_TINT), theme::contrast(theme::TEXT_DIM, over(glass(theme::GLASS_TINT), white(6))));
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
