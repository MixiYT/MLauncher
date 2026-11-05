//! GPU backdrop capture and rounded, refractive blur for Liquid Glass panels.
use eframe::{
    egui, egui_glow,
    glow::{self, HasContext},
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct Glass {
    state: Arc<Mutex<Option<Result<Resources, String>>>>,
}

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    size: [i32; 2],
    captured: bool,
}

impl Glass {
    pub fn register(&self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("liquid_backdrop"), self.clone()));
    }

    pub fn capture(&self, ui: &egui::Ui) {
        let state = self.state.clone();
        ui.painter().add(egui::PaintCallback {
            rect: ui.max_rect(),
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let Ok(mut state) = state.lock() else { return };
                let gl = painter.gl();
                let resources = state.get_or_insert_with(|| unsafe { Resources::new(gl) });
                if let Ok(resources) = resources {
                    unsafe {
                        resources.capture(gl, info.screen_size_px);
                    }
                }
            })),
        });
    }

    pub fn destroy(&self, gl: &glow::Context) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(Ok(resources)) = state.take() {
                unsafe {
                    gl.delete_program(resources.program);
                    gl.delete_vertex_array(resources.vao);
                    gl.delete_texture(resources.texture);
                    gl.delete_framebuffer(resources.framebuffer);
                }
            }
        }
    }
}

pub fn panel(ui: &egui::Ui, rect: egui::Rect, radius: f32) {
    let Some(glass) = ui
        .ctx()
        .data(|data| data.get_temp::<Glass>(egui::Id::new("liquid_backdrop")))
    else {
        return;
    };
    let opacity = ui.opacity();
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            let Ok(state) = glass.state.lock() else {
                return;
            };
            let Some(Ok(resources)) = state.as_ref() else {
                return;
            };
            if !resources.captured {
                return;
            }
            let viewport = info.viewport_in_pixels();
            let gl = painter.gl();
            unsafe {
                gl.use_program(Some(resources.program));
                gl.bind_vertex_array(Some(resources.vao));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(resources.texture));
                gl.uniform_1_i32(
                    gl.get_uniform_location(resources.program, "scene").as_ref(),
                    0,
                );
                gl.uniform_2_f32(
                    gl.get_uniform_location(resources.program, "screen")
                        .as_ref(),
                    info.screen_size_px[0] as f32,
                    info.screen_size_px[1] as f32,
                );
                gl.uniform_4_f32(
                    gl.get_uniform_location(resources.program, "panel").as_ref(),
                    viewport.left_px as f32,
                    viewport.from_bottom_px as f32,
                    viewport.width_px as f32,
                    viewport.height_px as f32,
                );
                gl.uniform_1_f32(
                    gl.get_uniform_location(resources.program, "radius")
                        .as_ref(),
                    radius * info.pixels_per_point,
                );
                gl.uniform_1_f32(
                    gl.get_uniform_location(resources.program, "opacity")
                        .as_ref(),
                    opacity,
                );
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        })),
    });
}

impl Resources {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        let result = (|| {
            for (kind, source) in [
                (glow::VERTEX_SHADER, VERTEX),
                (glow::FRAGMENT_SHADER, FRAGMENT),
            ] {
                let shader = gl.create_shader(kind)?;
                shaders.push(shader);
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            Ok(())
        })();
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if let Err(error) = result {
            gl.delete_program(program);
            return Err(error);
        }
        let vao = match gl.create_vertex_array() {
            Ok(value) => value,
            Err(error) => {
                gl.delete_program(program);
                return Err(error);
            }
        };
        let texture = match gl.create_texture() {
            Ok(value) => value,
            Err(error) => {
                gl.delete_program(program);
                gl.delete_vertex_array(vao);
                return Err(error);
            }
        };
        let framebuffer = match gl.create_framebuffer() {
            Ok(value) => value,
            Err(error) => {
                gl.delete_program(program);
                gl.delete_vertex_array(vao);
                gl.delete_texture(texture);
                return Err(error);
            }
        };
        Ok(Self {
            program,
            vao,
            texture,
            framebuffer,
            size: [0, 0],
            captured: false,
        })
    }

    unsafe fn capture(&mut self, gl: &glow::Context, screen: [u32; 2]) {
        self.captured = false;
        if screen.contains(&0) {
            return;
        }
        let size = [(screen[0] / 4).max(1) as i32, (screen[1] / 4).max(1) as i32];
        let draw = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        let scissor = gl.is_enabled(glow::SCISSOR_TEST);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        if size != self.size {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.size = size;
        }
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, draw);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.framebuffer));
        gl.framebuffer_texture_2d(
            glow::DRAW_FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(self.texture),
            0,
        );
        if gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE {
            gl.disable(glow::SCISSOR_TEST);
            gl.blit_framebuffer(
                0,
                0,
                screen[0] as i32,
                screen[1] as i32,
                0,
                0,
                size[0],
                size[1],
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );
            self.captured = true;
        }
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
        if scissor {
            gl.enable(glow::SCISSOR_TEST);
        }
    }
}

const VERTEX: &str = r#"#version 330 core
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"#version 330 core
uniform sampler2D scene;
uniform vec2 screen;
uniform vec4 panel;
uniform float radius;
uniform float opacity;
out vec4 color;
void main() {
    vec2 p = gl_FragCoord.xy - panel.xy;
    vec2 halfSize = panel.zw * 0.5;
    float r = min(radius, min(halfSize.x, halfSize.y));
    vec2 q = abs(p - halfSize) - (halfSize - r);
    float distance = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    float alpha = 1.0 - smoothstep(-1.0, 0.5, distance);
    if (alpha <= 0.0) discard;
    vec2 uv = gl_FragCoord.xy / screen;
    vec2 direction = (p - halfSize) / max(halfSize, vec2(1.0));
    float edge = pow(max(abs(direction.x), abs(direction.y)), 8.0);
    uv += direction * edge * 3.0 / screen;
    vec3 blurred = vec3(0.0);
    float weights[5] = float[5](1.0, 4.0, 6.0, 4.0, 1.0);
    vec2 stepSize = 1.5 / vec2(textureSize(scene, 0));
    for (int y = -2; y <= 2; y++)
        for (int x = -2; x <= 2; x++)
            blurred += texture(scene, uv + vec2(x, y) * stepSize).rgb * weights[x+2] * weights[y+2];
    color = vec4(blurred / 256.0 * alpha * opacity, alpha * opacity);
}
"#;
