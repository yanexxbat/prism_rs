//! The look: flat widgets on translucent "glass" panels over a colour gradient.
//! egui cannot blur what is behind a panel, so the glass is a tint, a light edge and a soft shadow.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Shadow, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(96, 148, 255);

fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, alpha)
}

/// Set once at startup.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = Margin::same(14);

    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(236, 240, 250));
    visuals.panel_fill = Color32::TRANSPARENT;
    // Dialogs are darker than panels: without blur, text over a busy background needs it.
    visuals.window_fill = Color32::from_rgba_unmultiplied(22, 27, 48, 236);
    visuals.window_stroke = Stroke::new(1.0_f32, white(46));
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.window_shadow = Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(110) };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.extreme_bg_color = Color32::from_rgba_unmultiplied(8, 10, 22, 150); // text fields
    visuals.faint_bg_color = white(8);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(96, 148, 255, 110);
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.hyperlink_color = ACCENT;

    // Flat: one tint per state, no outlines, no size change on hover.
    let widgets = &mut visuals.widgets;
    for (state, alpha) in [(&mut widgets.inactive, 9), (&mut widgets.hovered, 20), (&mut widgets.active, 32), (&mut widgets.open, 20)] {
        state.bg_fill = white(alpha);
        state.weak_bg_fill = white(alpha);
        state.bg_stroke = Stroke::NONE;
        state.corner_radius = CornerRadius::same(8);
        state.expansion = 0.0;
        state.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(236, 240, 250));
    }
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, white(26)); // separators
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(214, 220, 236));
    widgets.noninteractive.corner_radius = CornerRadius::same(8);
    ctx.set_style(style);
}

/// The frame of a main-window panel.
pub fn glass() -> Frame {
    Frame::new()
        .fill(white(9))
        .stroke(Stroke::new(1.0_f32, white(40)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(12))
        .outer_margin(Margin::same(6))
        .shadow(Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(60) })
}

/// The gradient behind everything, with two soft glows for the glass to sit on.
pub fn paint_background(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let mut mesh = egui::Mesh::default();
    let corners = [
        (rect.left_top(), Color32::from_rgb(38, 30, 84)),
        (rect.right_top(), Color32::from_rgb(20, 44, 92)),
        (rect.left_bottom(), Color32::from_rgb(16, 22, 48)),
        (rect.right_bottom(), Color32::from_rgb(14, 62, 78)),
    ];
    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(mesh);

    // A glow is a disc that fades from its centre to nothing at the rim.
    let glow = |center: egui::Pos2, radius: f32, color: [u8; 3]| {
        const SEGMENTS: u32 = 48;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 70));
        for i in 0..=SEGMENTS {
            let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(center + radius * egui::vec2(angle.cos(), angle.sin()), Color32::TRANSPARENT);
        }
        for i in 1..=SEGMENTS {
            mesh.add_triangle(0, i, i + 1);
        }
        painter.add(mesh);
    };
    glow(rect.left_top() + egui::vec2(rect.width() * 0.22, rect.height() * 0.25), rect.height() * 0.7, [150, 90, 255]);
    glow(rect.right_bottom() - egui::vec2(rect.width() * 0.2, rect.height() * 0.2), rect.height() * 0.8, [40, 200, 210]);
}
