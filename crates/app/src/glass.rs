//! The window: chrome objects floating behind a frosted, rounded glass panel, a Mac-style
//! title bar, and resizing from the panel's edges (the system frame is turned off).
//!
//! The window is a little larger than the panel. Around the panel it is transparent, apart
//! from the chrome that sticks out; on Windows the window is clipped to exactly that, so
//! clicks there reach whatever is behind.

use crate::blur::{self, Scene};
use crate::theme::{self, white};
use eframe::egui::{self, pos2, vec2, Color32, CornerRadius, Rect, Stroke, Vec2};
use std::sync::{Arc, Mutex};

struct Sprite {
    png: &'static [u8],
    /// Logical size on screen.
    diameter: f32,
    /// Centre as a fraction of the panel, then moved by `offset` logical pixels.
    anchor: [f32; 2],
    offset: [f32; 2],
    /// How much it follows the pointer (near objects more than far ones).
    depth: f32,
}

/// Far ones first. Rendered by `assets/chrome/render_chrome.py`.
const SPRITES: [Sprite; 5] = [
    Sprite { png: include_bytes!("../assets/chrome/sphere_tiny.png"), diameter: 44.0, anchor: [0.62, 0.0], offset: [0.0, 12.0], depth: 0.3 },
    Sprite { png: include_bytes!("../assets/chrome/sphere_small.png"), diameter: 74.0, anchor: [0.0, 0.62], offset: [-4.0, 0.0], depth: 0.5 },
    Sprite { png: include_bytes!("../assets/chrome/sphere_mid.png"), diameter: 108.0, anchor: [1.0, 0.27], offset: [16.0, 0.0], depth: 0.6 },
    Sprite { png: include_bytes!("../assets/chrome/torus.png"), diameter: 190.0, anchor: [0.22, 1.0], offset: [0.0, -28.0], depth: 0.8 },
    Sprite { png: include_bytes!("../assets/chrome/sphere_big.png"), diameter: 290.0, anchor: [1.0, 1.0], offset: [-46.0, -62.0], depth: 1.0 },
];

/// How far the chrome moves with the pointer, in logical pixels.
const MAX_SHIFT: f32 = 10.0;
/// Room around the panel for the chrome that sticks out: left, top, right, bottom.
const MARGIN: [f32; 4] = [46.0, 22.0, 116.0, 96.0];
/// The panel's size at startup.
const PANEL: Vec2 = vec2(960.0, 600.0);
const TITLE_BAR: f32 = 40.0;

pub fn window_size() -> Vec2 {
    PANEL + vec2(MARGIN[0] + MARGIN[2], MARGIN[1] + MARGIN[3])
}

pub fn min_window_size() -> Vec2 {
    vec2(760.0, 460.0) + vec2(MARGIN[0] + MARGIN[2], MARGIN[1] + MARGIN[3])
}

pub fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The panel inside the window. A maximised window is all panel.
pub fn panel_rect(screen: Rect, maximized: bool) -> Rect {
    if maximized {
        return screen;
    }
    let [left, top, right, bottom] = MARGIN;
    Rect::from_min_max(screen.min + vec2(left, top), screen.max - vec2(right, bottom))
}

fn corner_radius(maximized: bool) -> CornerRadius {
    CornerRadius::same(if maximized { 0 } else { theme::PANEL_RADIUS })
}

/// Where each sprite is drawn for the panel and the pointer shift.
fn sprite_rects(panel: Rect, shift: Vec2) -> impl Iterator<Item = (usize, Rect)> {
    SPRITES.iter().enumerate().map(move |(index, sprite)| {
        let center = panel.min + panel.size() * Vec2::from(sprite.anchor) + Vec2::from(sprite.offset) + shift * sprite.depth;
        (index, Rect::from_center_size(center, Vec2::splat(sprite.diameter)))
    })
}

/// What the blur sees, relative to its top-left corner `PAD` outside the panel.
fn scene(panel: Rect, shift: Vec2) -> Scene {
    let origin = panel.min - Vec2::splat(blur::PAD);
    Scene {
        panel: panel.translate(-origin.to_vec2()),
        sprites: sprite_rects(panel, shift).map(|(index, rect)| (index, rect.translate(-origin.to_vec2()))).collect(),
    }
}

/// The blurred backdrop, from the GPU when there is a glow context, else from the CPU.
enum Blurred {
    Gpu { blur: Arc<Mutex<blur::GlBlur>>, texture: Option<egui::TextureId> },
    Cpu { texture: Option<egui::TextureHandle> },
}

#[derive(Default)]
pub struct Glass {
    images: Vec<image::RgbaImage>,
    /// The sprites shrunk to blur texels, for both blurs.
    small: Vec<blur::Small>,
    sprites: Vec<egui::TextureHandle>,
    grain: Option<egui::TextureHandle>,
    blurred: Option<Blurred>,
    /// The scene the blurred texture holds now.
    shown: Option<Scene>,
    #[cfg(windows)]
    applied: Option<(bool, Vec<i32>)>,
}

fn decode(sprite: &Sprite) -> image::RgbaImage {
    image::load_from_memory_with_format(sprite.png, image::ImageFormat::Png).expect("bundled sprite").to_rgba8()
}

impl Glass {
    fn small(&mut self) -> &[blur::Small] {
        if self.small.is_empty() {
            let small = self.images().iter().zip(&SPRITES).map(|(image, sprite)| blur::Small::new(image, sprite.diameter)).collect();
            self.small = small;
        }
        &self.small
    }

    fn images(&mut self) -> &[image::RgbaImage] {
        if self.images.is_empty() {
            self.images = SPRITES.iter().map(decode).collect();
        }
        &self.images
    }

    /// Sets up the GPU blur. Without it (or if it fails) the CPU blur is used.
    pub fn init_gl(&mut self, gl: Option<&eframe::glow::Context>) {
        let Some(gl) = gl else { return };
        match blur::GlBlur::new(gl, self.small()) {
            Ok(blur) => self.blurred = Some(Blurred::Gpu { blur: Arc::new(Mutex::new(blur)), texture: None }),
            Err(e) => tracing::warn!("no GPU blur, using the CPU blur: {e}"),
        }
    }

    /// Work that needs the native window: the GPU texture's egui id, the acrylic backdrop and
    /// the window's clip shape. Cheap when nothing changed.
    pub fn window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame, acrylic: bool) {
        if let Some(Blurred::Gpu { blur, texture: texture @ None }) = &mut self.blurred {
            let output = blur.lock().expect("blur lock").output;
            *texture = Some(frame.register_native_glow_texture(output));
        }
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            return; // a minimised window reports itself as not maximised; keep the clip as it was
        }
        #[cfg(windows)]
        {
            let maximized = maximized(ctx);
            let ppp = ctx.pixels_per_point();
            let px = |r: Rect| [r.left(), r.top(), r.right(), r.bottom()].map(|v| (v * ppp).round() as i32);
            let panel = panel_rect(ctx.screen_rect(), maximized);
            let mut shape = Vec::new();
            if !maximized {
                shape.extend(px(panel));
                shape.push((theme::PANEL_RADIUS as f32 * ppp).round() as i32);
                if !acrylic {
                    // Room for the pointer shift, so the clip need not follow every frame.
                    sprite_rects(panel, Vec2::ZERO).for_each(|(_, r)| shape.extend(px(r.expand(MAX_SHIFT))));
                }
            }
            let state = (acrylic, shape);
            if self.applied.as_ref() != Some(&state) {
                let changed_acrylic = self.applied.as_ref().is_none_or(|(old, _)| *old != acrylic);
                if changed_acrylic && !crate::acrylic::set(frame, acrylic.then_some(ACRYLIC_TINT)) && acrylic {
                    tracing::warn!("the acrylic backdrop is not available");
                }
                crate::acrylic::clip(frame, &state.1);
                self.applied = Some(state);
            }
        }
        #[cfg(not(windows))]
        let _ = (ctx, acrylic);
    }

    /// The chrome, the blurred backdrop, the glass tint, grain and edge, behind everything else.
    pub fn paint(&mut self, ctx: &egui::Context, panel: Rect, acrylic: bool) {
        let maximized = maximized(ctx);
        let radius = corner_radius(maximized);
        if self.sprites.is_empty() {
            let images: Vec<_> = self.images().to_vec();
            self.sprites = images
                .iter()
                .enumerate()
                .map(|(i, image)| {
                    let size = [image.width() as usize, image.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
                    ctx.load_texture(format!("chrome {i}"), color, options)
                })
                .collect();
            self.grain = Some(ctx.load_texture("grain", grain(), egui::TextureOptions::NEAREST_REPEAT));
        }
        let painter = ctx.layer_painter(egui::LayerId::background());
        let tint = if acrylic {
            theme::GLASS_TINT_ACRYLIC
        } else {
            // The sharp chrome first: the opaque blurred panel then covers it except where it sticks out.
            // No parallax on the CPU blur: it would re-blur on every frame of the easing.
            let gpu = matches!(self.blurred, Some(Blurred::Gpu { .. }));
            let shift = if gpu { self.parallax(ctx, panel) } else { Vec2::ZERO };
            for (index, rect) in sprite_rects(panel, shift) {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(self.sprites[index].id(), rect, uv, Color32::WHITE);
            }
            let scene = scene(panel, shift);
            let texture = self.blurred_texture(ctx, &painter, &scene);
            painter.add(egui::epaint::RectShape::filled(panel, radius, Color32::WHITE).with_texture(texture, scene.panel_uv()));
            theme::GLASS_TINT
        };
        painter.rect_filled(panel, radius, tint);
        if let Some(grain) = &self.grain {
            let tiles = panel.size() * ctx.pixels_per_point() / GRAIN_SIZE as f32;
            let uv = Rect::from_min_size(pos2(0.0, 0.0), tiles);
            painter.add(egui::epaint::RectShape::filled(panel, radius, Color32::WHITE).with_texture(grain.id(), uv));
        }
        painter.rect_stroke(panel, radius, Stroke::new(1.0_f32, theme::GLASS_EDGE), egui::StrokeKind::Inside);
        let inset = radius.nw as f32;
        let top = panel.top() + 0.5;
        painter.line_segment([pos2(panel.left() + inset, top), pos2(panel.right() - inset, top)], Stroke::new(1.0_f32, theme::GLASS_EDGE_TOP));
    }

    /// The chrome follows the pointer a little and eases back when it leaves.
    /// This only animates after pointer moves, so an idle window does not repaint.
    fn parallax(&self, ctx: &egui::Context, panel: Rect) -> Vec2 {
        let target = ctx.input(|i| i.pointer.hover_pos()).map_or(Vec2::ZERO, |p| {
            let d = (p - panel.center()) / (panel.size() * 0.5);
            vec2(d.x.clamp(-1.0, 1.0), d.y.clamp(-1.0, 1.0)) * MAX_SHIFT
        });
        let id = egui::Id::new("glass parallax");
        let x = ctx.animate_value_with_time(id.with(0), target.x, 0.8);
        let y = ctx.animate_value_with_time(id.with(1), target.y, 0.8);
        // Quarter pixels: finer steps would re-blur for changes nobody can see.
        (vec2(x, y) * 4.0).round() / 4.0
    }

    /// The blurred backdrop for `scene`, recomputed only when the scene changed.
    fn blurred_texture(&mut self, ctx: &egui::Context, painter: &egui::Painter, scene: &Scene) -> egui::TextureId {
        if let Some(Blurred::Gpu { blur, .. }) = &self.blurred {
            if blur.lock().expect("blur lock").failed {
                self.blurred = None;
                self.shown = None;
            }
        }
        if self.blurred.is_none() {
            self.small();
            self.blurred = Some(Blurred::Cpu { texture: None });
        }
        match self.blurred.as_mut().expect("set above") {
            Blurred::Gpu { blur, texture } => {
                if blur.lock().expect("blur lock").rendered.as_ref() != Some(scene) {
                    let (blur, scene, repaint) = (blur.clone(), scene.clone(), ctx.clone());
                    let callback = eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                        let mut blur = blur.lock().expect("blur lock");
                        blur.render(painter.gl(), &scene, theme::BACKDROP_CENTER, theme::BACKDROP_EDGE);
                        if blur.failed {
                            repaint.request_repaint(); // so the CPU blur takes over at once
                        }
                    });
                    painter.add(egui::PaintCallback { rect: ctx.screen_rect(), callback: Arc::new(callback) });
                }
                // Registered in `window`, which runs before the first paint.
                texture.expect("GPU blur texture registered")
            }
            Blurred::Cpu { texture } => {
                if self.shown.as_ref() != Some(scene) || texture.is_none() {
                    let image = blur::cpu_blur(scene, &self.small, theme::BACKDROP_CENTER, theme::BACKDROP_EDGE);
                    match texture {
                        Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
                        None => *texture = Some(ctx.load_texture("glass blur", image, egui::TextureOptions::LINEAR)),
                    }
                    self.shown = Some(scene.clone());
                }
                texture.as_ref().expect("set above").id()
            }
        }
    }
}

#[cfg(windows)]
/// The tint Windows lays over its acrylic blur, as RGBA.
const ACRYLIC_TINT: [u8; 4] = [28, 28, 31, 96];

const GRAIN_SIZE: usize = 128;

/// Film grain: a fixed pattern of light and dark specks, `theme::GRAIN` strong.
fn grain() -> egui::ColorImage {
    let mut state = 0x2545_f491_u32;
    let pixels = (0..GRAIN_SIZE * GRAIN_SIZE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let alpha = (noise.abs() * theme::GRAIN * 255.0).round() as u8;
            if noise > 0.0 {
                white(alpha)
            } else {
                Color32::from_black_alpha(alpha)
            }
        })
        .collect();
    egui::ColorImage { size: [GRAIN_SIZE, GRAIN_SIZE], pixels }
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

    fn default_panel() -> Rect {
        panel_rect(Rect::from_min_size(pos2(0.0, 0.0), window_size()), false)
    }

    #[test]
    fn the_margin_holds_the_chrome_that_sticks_out() {
        let window = Rect::from_min_size(pos2(0.0, 0.0), window_size());
        let panel = default_panel();
        assert_eq!(panel.size(), PANEL);
        for shift in [vec2(MAX_SHIFT, MAX_SHIFT), vec2(-MAX_SHIFT, -MAX_SHIFT)] {
            for (index, rect) in sprite_rects(panel, shift) {
                assert!(window.contains_rect(rect), "sprite {index} at {rect:?} leaves the window");
                assert!(rect.intersects(panel), "sprite {index} is not behind the glass at all");
            }
        }
    }

    /// Lesbarkeit zuerst: every text colour keeps 4.5:1 against the brightest spot of the glass,
    /// including where chrome glows through, with the grain's light specks on top.
    #[test]
    fn text_stays_readable_over_the_brightest_glass() {
        let images: Vec<_> = SPRITES.iter().map(decode).collect();
        let small: Vec<_> = images.iter().zip(&SPRITES).map(|(image, sprite)| blur::Small::new(image, sprite.diameter)).collect();
        let over = |below: Color32, top: Color32| {
            let a = top.a() as f32 / 255.0;
            let mix = |t: u8, b: u8| (t as f32 + b as f32 * (1.0 - a)).round() as u8;
            Color32::from_rgb(mix(top.r(), below.r()), mix(top.g(), below.g()), mix(top.b(), below.b()))
        };
        let grain = white((theme::GRAIN * 255.0).round() as u8);
        let mut brightest = Color32::BLACK;
        for shift in [Vec2::ZERO, vec2(MAX_SHIFT, MAX_SHIFT), vec2(-MAX_SHIFT, MAX_SHIFT)] {
            let scene = scene(default_panel(), shift);
            let image = blur::cpu_blur(&scene, &small, theme::BACKDROP_CENTER, theme::BACKDROP_EDGE);
            let [w, _] = image.size;
            for (i, &pixel) in image.pixels.iter().enumerate() {
                let p = pos2((i % w) as f32 + 0.5, (i / w) as f32 + 0.5) * blur::SCALE;
                if scene.panel.contains(p) {
                    // The details tile adds its own light fill on top.
                    let glass = over(over(over(pixel, theme::GLASS_TINT), grain), white(6));
                    if glass.r() as u32 + glass.g() as u32 + glass.b() as u32 > brightest.r() as u32 + brightest.g() as u32 + brightest.b() as u32 {
                        brightest = glass;
                    }
                }
            }
        }
        // Acrylic: the worst case is a white desktop under Windows' own tint and ours.
        let desktop = over(Color32::WHITE, Color32::from_rgba_unmultiplied(28, 28, 31, 96));
        let acrylic = over(over(desktop, theme::GLASS_TINT_ACRYLIC), grain);
        for (name, text) in [("TEXT", theme::TEXT), ("TEXT_TITLE", theme::TEXT_TITLE), ("TEXT_DIM", theme::TEXT_DIM)] {
            for (glass, background) in [("own blur", brightest), ("acrylic", acrylic)] {
                let ratio = theme::contrast(text, background);
                assert!(ratio >= 4.5, "{name} on {glass} {background:?}: {ratio:.2}");
            }
        }
        // Dialogs: body and weak text (`ui.weak`) on the dialog fill over the brightest glass.
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let visuals = ctx.style().visuals.clone();
        let dialog = over(brightest, visuals.window_fill);
        for (name, text) in [("dialog text", visuals.text_color()), ("weak dialog text", visuals.weak_text_color())] {
            let ratio = theme::contrast(text, dialog);
            assert!(ratio >= 4.5, "{name} {text:?} on {dialog:?}: {ratio:.2}");
        }
        println!("brightest glass {brightest:?}, TEXT_DIM {:.2}:1", theme::contrast(theme::TEXT_DIM, brightest));
    }

    #[test]
    fn grain_is_faint_and_balanced() {
        let image = grain();
        let max = image.pixels.iter().map(|p| p.a()).max().unwrap_or(0);
        assert!(max as f32 <= theme::GRAIN * 255.0 + 1.0 && max > 0);
        let light = image.pixels.iter().filter(|p| p.r() > 0).count() as f32 / image.pixels.len() as f32;
        assert!((light - 0.5).abs() < 0.05, "{light}");
    }
}
