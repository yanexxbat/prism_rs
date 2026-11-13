
//! Glassmorphism theme: translucent panels over a colourful gradient.
//! Normal opaque Windows window, transparent egui panels.

use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Shadow, Stroke,
};

pub const ACCENT: Color32 = Color32::from_rgb(96, 148, 255);
pub const ON_ACCENT: Color32 = Color32::WHITE;

const BACKDROP: u8 = 100;

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn white(alpha: u8) -> Color32 {
    rgba(255, 255, 255, alpha)
}

/// Apply the application theme once at startup.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.spacing.item_spacing = egui::vec2(9.0, 9.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.window_margin = Margin::same(14);

    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();

    visuals.override_text_color = Some(rgba(238, 242, 255, 255));

    // Keep the native window opaque, but make egui panels transparent.
    visuals.panel_fill = Color32::TRANSPARENT;

    // Floating dialogs.
    visuals.window_fill = rgba(24, 30, 52, 205);
    visuals.window_stroke = Stroke::new(1.0, white(45));
    visuals.window_corner_radius = CornerRadius::same(14);

    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: rgba(0, 0, 0, 100),
    };

    visuals.popup_shadow = visuals.window_shadow;
    visuals.menu_corner_radius = CornerRadius::same(10);

    // Input fields.
    visuals.extreme_bg_color = rgba(10, 14, 30, 115);
    visuals.faint_bg_color = white(12);

    // Selection.
    visuals.selection.bg_fill = rgba(96, 148, 255, 100);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    visuals.hyperlink_color = ACCENT;

    // Flat translucent widgets.
    let widgets = &mut visuals.widgets;

    for (state, alpha) in [
        (&mut widgets.inactive, 12),
        (&mut widgets.hovered, 28),
        (&mut widgets.active, 42),
        (&mut widgets.open, 24),
    ] {
        state.bg_fill = white(alpha);
        state.weak_bg_fill = white(alpha);
        state.bg_stroke = Stroke::NONE;
        state.corner_radius = CornerRadius::same(8);
        state.expansion = 0.0;
        state.fg_stroke = Stroke::new(
            1.0,
            rgba(238, 242, 255, 255),
        );
    }

    widgets.noninteractive.bg_stroke = Stroke::new(1.0, white(30));
    widgets.noninteractive.fg_stroke =
        Stroke::new(1.0, rgba(214, 220, 236, 255));
    widgets.noninteractive.corner_radius = CornerRadius::same(8);

    ctx.set_style(style);
}

/// Main glass panel.


pub fn glass() -> Frame {
    Frame::new()
        .fill(rgba(255, 255, 255, 22))
        .stroke(Stroke::new(1.0, white(55)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(14))
        .outer_margin(Margin::same(6))
        .shadow(Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: rgba(0, 0, 0, 45),
        })
}


/// Background gradient.
pub fn paint_background(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());

    let mut mesh = egui::Mesh::default();

    let corners = [
        (rect.left_top(), rgba(38, 30, 84, BACKDROP)),
        (rect.right_top(), rgba(20, 44, 92, BACKDROP)),
        (rect.left_bottom(), rgba(16, 22, 48, BACKDROP)),
        (rect.right_bottom(), rgba(14, 62, 78, BACKDROP)),
    ];

    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }

    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(mesh);

    // Soft purple glow.
    let glow = |center: egui::Pos2, radius: f32, color: [u8; 3]| {
        const SEGMENTS: u32 = 64;

        let mut mesh = egui::Mesh::default();

        mesh.colored_vertex(
            center,
            rgba(color[0], color[1], color[2], 75),
        );

        for i in 0..=SEGMENTS {
            let angle = i as f32 / SEGMENTS as f32
                * std::f32::consts::TAU;

            mesh.colored_vertex(
                center + radius * egui::vec2(angle.cos(), angle.sin()),
                Color32::TRANSPARENT,
            );
        }

        for i in 1..=SEGMENTS {
            mesh.add_triangle(0, i, i + 1);
        }

        painter.add(mesh);
    };

    glow(
        rect.left_top()
            + egui::vec2(rect.width() * 0.22, rect.height() * 0.25),
        rect.height() * 0.7,
        [150, 90, 255],
    );

    glow(
        rect.right_bottom()
            - egui::vec2(rect.width() * 0.2, rect.height() * 0.2),
        rect.height() * 0.8,
        [40, 200, 210],
    );
}
