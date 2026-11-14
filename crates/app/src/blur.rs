//! The blur behind the glass: the backdrop and the chrome sprites, drawn small and blurred
//! with four separable Gaussian passes (horizontal, vertical, twice).
//!
//! Two implementations of the same maths: `GlBlur` runs it on the GPU through glow, and
//! `cpu_blur` is the fallback when there is no glow context (tests, other renderers).
//! Both run only when the scene changes, never on an idle frame.

use eframe::egui::{self, Rect, Vec2};
use eframe::glow::{self, HasContext as _};

/// Logical pixels per blur texel. The blurred image has no fine detail, so a quarter is plenty.
pub const SCALE: f32 = 4.0;
/// Standard deviation of the whole blur in logical pixels; three of them make a radius of 60 px.
pub const SIGMA: f32 = 20.0;
/// How far around the panel the blur looks, so chrome just outside it glows in at the edge.
pub const PAD: f32 = 3.0 * SIGMA;
/// Room for the weights of one pass in the shader.
const MAX_TAPS: usize = 24;

/// What the blur sees, in logical pixels from its own top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// The panel, `PAD` in from every side.
    pub panel: Rect,
    /// Sprite index and where it is drawn, far ones first.
    pub sprites: Vec<(usize, Rect)>,
}

impl Scene {
    pub fn texels(&self) -> [usize; 2] {
        let size = self.panel.size() + Vec2::splat(2.0 * PAD);
        [(size.x / SCALE).ceil().max(1.0) as usize, (size.y / SCALE).ceil().max(1.0) as usize]
    }

    /// The logical size the texels cover (a little more than the panel and its padding).
    fn extent(&self) -> Vec2 {
        let [w, h] = self.texels();
        egui::vec2(w as f32, h as f32) * SCALE
    }

    /// Where the panel is in the blurred texture.
    pub fn panel_uv(&self) -> Rect {
        let extent = self.extent();
        Rect::from_min_max((self.panel.min.to_vec2() / extent).to_pos2(), (self.panel.max.to_vec2() / extent).to_pos2())
    }

    /// 0 in the middle of the panel, 1 towards and past its edges.
    fn vignette(&self, p: egui::Pos2) -> f32 {
        let d = ((p - self.panel.center()) / (self.panel.size() * 0.5)).length();
        smoothstep(0.2, 1.3, d)
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Half of one pass's Gaussian, centre first, normalised. Four passes of this give `SIGMA`.
pub fn kernel() -> Vec<f32> {
    let sigma = SIGMA / SCALE / std::f32::consts::SQRT_2;
    let radius = (3.0 * sigma).ceil() as usize;
    let weights: Vec<f32> = (0..=radius).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let total = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
    weights.iter().map(|w| w / total).collect()
}

fn linear(color: egui::Color32) -> [f32; 3] {
    let [r, g, b, _] = egui::Rgba::from(color).to_array();
    [r, g, b]
}

/// A sprite shrunk to blur texels, in linear light with premultiplied alpha.
pub struct Small {
    side: usize,
    texels: Vec<[f32; 4]>,
}

impl Small {
    /// Averages the source pixels under each texel; `diameter` is the sprite's logical size.
    pub fn new(image: &image::RgbaImage, diameter: f32) -> Self {
        let side = (diameter / SCALE).ceil().max(1.0) as usize;
        let (w, h) = (image.width() as usize, image.height() as usize);
        let mut texels = vec![[0.0; 4]; side * side];
        for (j, row) in texels.chunks_mut(side).enumerate() {
            let (y0, y1) = (j * h / side, ((j + 1) * h).div_ceil(side));
            for (i, texel) in row.iter_mut().enumerate() {
                let (x0, x1) = (i * w / side, ((i + 1) * w).div_ceil(side));
                let mut sum = [0.0f32; 4];
                for y in y0..y1 {
                    for x in x0..x1 {
                        let [r, g, b, a] = image.get_pixel(x as u32, y as u32).0;
                        let c = egui::Rgba::from(egui::Color32::from_rgb(r, g, b));
                        let a = a as f32 / 255.0;
                        for (s, v) in sum.iter_mut().zip([c.r() * a, c.g() * a, c.b() * a, a]) {
                            *s += v;
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as f32;
                *texel = sum.map(|s| s / n);
            }
        }
        Small { side, texels }
    }

    /// Bilinear, transparent outside.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| {
            if i < 0.0 || j < 0.0 || i >= self.side as f32 || j >= self.side as f32 {
                [0.0; 4]
            } else {
                self.texels[j as usize * self.side + i as usize]
            }
        };
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        std::array::from_fn(|k| (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy)
    }
}

/// The fallback: the same picture as `GlBlur`, computed on the CPU (about 50 000 texels).
pub fn cpu_blur(scene: &Scene, sprites: &[Small], center: egui::Color32, edge: egui::Color32) -> egui::ColorImage {
    let [w, h] = scene.texels();
    let (center, edge) = (linear(center), linear(edge));
    let mut image: Vec<[f32; 4]> = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let p = egui::pos2((x as f32 + 0.5) * SCALE, (y as f32 + 0.5) * SCALE);
            let t = scene.vignette(p);
            let mut color = [0, 1, 2].map(|k| center[k] + (edge[k] - center[k]) * t);
            for (index, rect) in &scene.sprites {
                if rect.contains(p) {
                    let small = &sprites[*index];
                    let uv = (p - rect.min) / rect.size() * small.side as f32 - Vec2::splat(0.5);
                    let s = small.sample(uv.x, uv.y);
                    color = [0, 1, 2].map(|k| s[k] + color[k] * (1.0 - s[3]));
                }
            }
            image.push([color[0], color[1], color[2], 1.0]);
        }
    }
    let weights = kernel();
    let mut scratch = image.clone();
    for _ in 0..2 {
        pass(&image, &mut scratch, [w, h], [1, 0], &weights);
        pass(&scratch, &mut image, [w, h], [0, 1], &weights);
    }
    let pixels = image.iter().map(|c| egui::Rgba::from_rgb(c[0], c[1], c[2]).into()).collect();
    egui::ColorImage { size: [w, h], pixels }
}

/// One direction of the blur, clamped at the edges like the GPU's sampler.
fn pass(from: &[[f32; 4]], to: &mut [[f32; 4]], [w, h]: [usize; 2], [dx, dy]: [isize; 2], weights: &[f32]) {
    for y in 0..h {
        for x in 0..w {
            let at = |offset: isize| {
                let sx = (x as isize + dx * offset).clamp(0, w as isize - 1) as usize;
                let sy = (y as isize + dy * offset).clamp(0, h as isize - 1) as usize;
                from[sy * w + sx]
            };
            let mut sum = at(0).map(|v| v * weights[0]);
            for (i, weight) in weights.iter().enumerate().skip(1) {
                let (a, b) = (at(i as isize), at(-(i as isize)));
                for k in 0..4 {
                    sum[k] += (a[k] + b[k]) * weight;
                }
            }
            to[y * w + x] = sum;
        }
    }
}

const VERTEX: &str = r#"
uniform vec4 u_rect; // x0, y0, x1, y1 in clip space
out vec2 v_uv;
void main() {
    vec2 corner = vec2(float(gl_VertexID % 2), float(gl_VertexID / 2));
    v_uv = corner;
    gl_Position = vec4(mix(u_rect.xy, u_rect.zw, corner), 0.0, 1.0);
}
"#;

/// The backdrop (`u_backdrop == 1`) or one sprite, in linear light, premultiplied.
const DRAW: &str = r#"
uniform sampler2D u_tex;
uniform int u_backdrop;
uniform vec4 u_panel; // panel min and max, 0..1 of the target
uniform vec3 u_center;
uniform vec3 u_edge;
in vec2 v_uv;
out vec4 f_color;
void main() {
    if (u_backdrop == 1) {
        vec2 c = (u_panel.xy + u_panel.zw) * 0.5;
        vec2 half_size = (u_panel.zw - u_panel.xy) * 0.5;
        float t = smoothstep(0.2, 1.3, length((v_uv - c) / half_size));
        f_color = vec4(mix(u_center, u_edge, t), 1.0);
    } else {
        f_color = texture(u_tex, v_uv); // already linear and premultiplied
    }
}
"#;

const BLUR: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_step;
uniform float u_weights[24];
uniform int u_radius;
in vec2 v_uv;
out vec4 f_color;
void main() {
    vec4 sum = texture(u_tex, v_uv) * u_weights[0];
    for (int i = 1; i < 24; i++) {
        if (i > u_radius) break;
        vec2 offset = u_step * float(i);
        sum += (texture(u_tex, v_uv + offset) + texture(u_tex, v_uv - offset)) * u_weights[i];
    }
    f_color = sum;
}
"#;

struct Program {
    program: glow::Program,
    uniforms: Vec<Option<glow::UniformLocation>>,
}

/// The GPU blur. Its `output` texture is registered with egui once and keeps its handle
/// when it is resized, so egui always paints the latest blur.
pub struct GlBlur {
    draw: Program,
    blur: Program,
    vao: glow::VertexArray,
    sprites: Vec<glow::Texture>,
    fbo: glow::Framebuffer,
    ping: glow::Texture,
    pong: glow::Texture,
    pub output: glow::Texture,
    size: [usize; 2],
    /// Set when the driver refused the framebuffer; the app then uses `cpu_blur`.
    pub failed: bool,
    /// The scene `output` holds. egui may drop a frame's callbacks (a discarded pass), so
    /// the app compares against this rather than against what it asked for.
    pub rendered: Option<Scene>,
}

// SAFETY for every `unsafe` block below: these are plain OpenGL calls on the context egui
// renders with, made while it is current (at creation and inside egui's paint callback),
// on objects this struct created.
impl GlBlur {
    pub fn new(gl: &glow::Context, sprites: &[Small]) -> Result<Self, String> {
        let version = egui_glow_version(gl)?;
        let srgb = version.contains("es") || gl.supported_extensions().iter().any(|e| e.contains("sRGB"));
        if !srgb {
            return Err("no sRGB textures".into());
        }
        unsafe {
            let draw = program(gl, version, DRAW, &["u_rect", "u_tex", "u_backdrop", "u_panel", "u_center", "u_edge"])?;
            let blur = program(gl, version, BLUR, &["u_rect", "u_tex", "u_step", "u_weights", "u_radius"])?;
            let vao = gl.create_vertex_array()?;
            let fbo = gl.create_framebuffer()?;
            let mut textures = Vec::new();
            // The same shrunk, premultiplied texels the CPU blur samples, so both paths match.
            for small in sprites {
                let texture = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                let side = small.side as i32;
                let bytes: Vec<u8> = small.texels.iter().flatten().flat_map(|v| v.to_ne_bytes()).collect();
                let data = glow::PixelUnpackData::Slice(Some(&bytes));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA32F as i32, side, side, 0, glow::RGBA, glow::FLOAT, data);
                sampler(gl, glow::LINEAR);
                textures.push(texture);
            }
            let [ping, pong, output] = [gl.create_texture()?, gl.create_texture()?, gl.create_texture()?];
            let blur = GlBlur { draw, blur, vao, sprites: textures, fbo, ping, pong, output, size: [0, 0], failed: false, rendered: None };
            gl.bind_texture(glow::TEXTURE_2D, None);
            Ok(blur)
        }
    }

    /// Draws `scene` into `output`. Called from an egui paint callback, which restores egui's
    /// own state afterwards except for the framebuffer.
    pub fn render(&mut self, gl: &glow::Context, scene: &Scene, center: egui::Color32, edge: egui::Color32) {
        if self.failed {
            return;
        }
        let [w, h] = scene.texels();
        unsafe {
            if self.size != [w, h] {
                for texture in [self.ping, self.pong, self.output] {
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                    let empty = glow::PixelUnpackData::Slice(None);
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA16F as i32, w as i32, h as i32, 0, glow::RGBA, glow::FLOAT, empty);
                    sampler(gl, glow::LINEAR);
                }
                self.size = [w, h];
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(self.ping), 0);
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                tracing::warn!("the blur framebuffer is incomplete; using the CPU blur");
                self.failed = true;
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                return;
            }
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);

            // Backdrop, then the sprites over it.
            let extent = scene.extent();
            let u = &self.draw.uniforms;
            gl.use_program(Some(self.draw.program));
            gl.uniform_1_i32(u[1].as_ref(), 0);
            gl.uniform_4_f32(u[0].as_ref(), -1.0, -1.0, 1.0, 1.0);
            gl.uniform_1_i32(u[2].as_ref(), 1);
            let panel = scene.panel_uv();
            gl.uniform_4_f32(u[3].as_ref(), panel.min.x, panel.min.y, panel.max.x, panel.max.y);
            let ([cr, cg, cb], [er, eg, eb]) = (linear(center), linear(edge));
            gl.uniform_3_f32(u[4].as_ref(), cr, cg, cb);
            gl.uniform_3_f32(u[5].as_ref(), er, eg, eb);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.uniform_1_i32(u[2].as_ref(), 0);
            for (index, rect) in &scene.sprites {
                // Logical top maps to clip -1, which is texture row 0: egui's uv origin.
                let clip = |p: egui::Pos2| (p.to_vec2() / extent) * 2.0 - Vec2::splat(1.0);
                let (a, b) = (clip(rect.min), clip(rect.max));
                gl.uniform_4_f32(u[0].as_ref(), a.x, a.y, b.x, b.y);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.sprites[*index]));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            gl.disable(glow::BLEND);

            // Horizontal, vertical, horizontal, vertical.
            let weights = kernel();
            let mut padded = [0.0f32; MAX_TAPS];
            padded[..weights.len()].copy_from_slice(&weights);
            let u = &self.blur.uniforms;
            gl.use_program(Some(self.blur.program));
            gl.uniform_1_i32(u[1].as_ref(), 0);
            gl.uniform_4_f32(u[0].as_ref(), -1.0, -1.0, 1.0, 1.0);
            gl.uniform_1_f32_slice(u[3].as_ref(), &padded);
            gl.uniform_1_i32(u[4].as_ref(), weights.len() as i32 - 1);
            let (sx, sy) = (1.0 / w as f32, 1.0 / h as f32);
            for (from, to, step) in [
                (self.ping, self.pong, [sx, 0.0]),
                (self.pong, self.ping, [0.0, sy]),
                (self.ping, self.pong, [sx, 0.0]),
                (self.pong, self.output, [0.0, sy]),
            ] {
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(to), 0);
                gl.bind_texture(glow::TEXTURE_2D, Some(from));
                gl.uniform_2_f32(u[2].as_ref(), step[0], step[1]);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.rendered = Some(scene.clone());
        }
    }
}

/// The shader header egui itself uses; only GLSL versions with `in`/`out` and `gl_VertexID`.
fn egui_glow_version(gl: &glow::Context) -> Result<&'static str, String> {
    let version = eframe::egui_glow::ShaderVersion::get(gl);
    if !version.is_new_shader_interface() {
        return Err(format!("{version:?} is too old for the blur shaders"));
    }
    Ok(version.version_declaration())
}

unsafe fn sampler(gl: &glow::Context, min_filter: u32) {
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min_filter as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
}

unsafe fn program(gl: &glow::Context, version: &str, fragment: &str, uniforms: &[&str]) -> Result<Program, String> {
    let header = format!("{version}\n#ifdef GL_ES\nprecision highp float;\n#endif\n");
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, fragment)] {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, &format!("{header}{source}"));
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            return Err(gl.get_shader_info_log(shader));
        }
        gl.attach_shader(program, shader);
        shaders.push(shader);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(gl.get_program_info_log(program));
    }
    for shader in shaders {
        gl.detach_shader(program, shader);
        gl.delete_shader(shader);
    }
    let uniforms = uniforms.iter().map(|name| gl.get_uniform_location(program, name)).collect();
    Ok(Program { program, uniforms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_passes_make_the_whole_sigma() {
        let weights = kernel();
        assert!(weights.len() <= MAX_TAPS);
        let total = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
        assert!((total - 1.0).abs() < 1e-5);
        let variance: f32 = weights.iter().enumerate().skip(1).map(|(i, w)| 2.0 * w * (i * i) as f32).sum();
        let sigma = (4.0 * variance).sqrt() * SCALE / std::f32::consts::SQRT_2; // two passes per axis
        assert!((sigma - SIGMA).abs() < 0.5, "{sigma}");
    }
}
