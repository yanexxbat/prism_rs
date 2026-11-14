//! The look: smoked, frosted glass. Neutral greys, thin light edges, flat outlined controls.
//! The blur comes from the window's acrylic backdrop (see `main.rs`); egui only adds the tint.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Shadow, Stroke};

/// The one highlighted control (Launch): light on dark, like the rest of the type.
pub const ACCENT: Color32 = Color32::from_rgb(232, 232, 236);
pub const ON_ACCENT: Color32 = Color32::from_rgb(24, 24, 27);

/// How much of the smoke tint covers the blurred desktop (0 = clear, 255 = solid).
const BACKDROP: u8 = 40;

const TEXT: Color32 = Color32::from_rgb(226, 226, 230);
const TEXT_DIM: Color32 = Color32::from_rgb(160, 160, 168);

fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, alpha)
}

/// Set once at startup.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(14.0, 6.0);
    style.spacing.window_margin = Margin::same(16);

    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = Color32::TRANSPARENT;
    // Dialogs are denser than the window so text stays readable over whatever is behind.
    visuals.window_fill = Color32::from_rgba_unmultiplied(30, 30, 33, 240);
    visuals.window_stroke = Stroke::new(1.0_f32, white(38));
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.window_shadow = Shadow { offset: [0, 12], blur: 36, spread: 0, color: Color32::from_black_alpha(120) };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.extreme_bg_color = Color32::from_rgba_unmultiplied(0, 0, 0, 70); // text fields
    visuals.faint_bg_color = white(6);
    visuals.selection.bg_fill = white(34);
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = Color32::WHITE;

    // Flat and outlined: a hairline at rest, a light wash on hover, no size change.
    let widgets = &mut visuals.widgets;
    for (state, fill, edge, text) in [
        (&mut widgets.inactive, 0, 42, TEXT),
        (&mut widgets.hovered, 18, 90, Color32::WHITE),
        (&mut widgets.active, 34, 130, Color32::WHITE),
        (&mut widgets.open, 18, 90, Color32::WHITE),
    ] {
        state.bg_fill = white(fill);
        state.weak_bg_fill = white(fill);
        state.bg_stroke = Stroke::new(1.0_f32, white(edge));
        state.corner_radius = CornerRadius::same(8);
        state.expansion = 0.0;
        state.fg_stroke = Stroke::new(1.0_f32, text);
    }
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, white(24)); // separators
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    widgets.noninteractive.corner_radius = CornerRadius::same(8);
    ctx.set_style(style);
}

/// The frame of a main-window panel: a barely lighter pane with a hairline edge.
pub fn glass() -> Frame {
    Frame::new()
        .fill(white(5))
        .stroke(Stroke::new(1.0_f32, white(26)))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(14))
        .outer_margin(Margin::same(6))
}

/// The smoke tint over the blurred desktop, a little lighter towards the top.
pub fn paint_background(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let top = Color32::from_rgba_unmultiplied(44, 44, 48, BACKDROP);
    let bottom = Color32::from_rgba_unmultiplied(22, 22, 24, BACKDROP);
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [(rect.left_top(), top), (rect.right_top(), top), (rect.left_bottom(), bottom), (rect.right_bottom(), bottom)] {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(mesh);
}
