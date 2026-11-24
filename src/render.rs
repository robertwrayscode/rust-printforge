//! OpenGL viewport renderer (runs inside egui via a paint callback).

use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3};
use std::sync::{Arc, Mutex};

pub const FLOATS_PER_VERT: usize = 10; // pos3 normal3 rgba4

#[derive(Clone, Copy, PartialEq)]
pub enum Pass {
    Opaque,
    Overlay,
    Transparent,
    Lines,
}

pub struct Batch {
    pub pass: Pass,
    pub data: Vec<f32>,
}

impl Batch {
    pub fn new(pass: Pass) -> Self {
        Batch { pass, data: Vec::new() }
    }
    pub fn vert(&mut self, p: Vec3, n: Vec3, c: [f32; 4]) {
        self.data.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, c[0], c[1], c[2], c[3]]);
    }
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [f32; 4]) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        self.vert(a, n, col);
        self.vert(b, n, col);
        self.vert(c, n, col);
    }
    pub fn line(&mut self, a: Vec3, b: Vec3, col: [f32; 4]) {
        self.vert(a, Vec3::Z, col);
        self.vert(b, Vec3::Z, col);
    }
}

struct GpuBatch {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
    pass: Pass,
}

#[derive(Default)]
pub struct Renderer {
    program: Option<glow::Program>,
    batches: Vec<GpuBatch>,
    pending: Option<Vec<Batch>>,
    pub view_proj: Mat4,
    pub eye: Vec3,
}

pub type SharedRenderer = Arc<Mutex<Renderer>>;

const VS: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in vec4 a_col;
uniform mat4 u_vp;
out vec3 v_nrm;
out vec4 v_col;
out vec3 v_pos;
void main() {
    v_nrm = a_nrm;
    v_col = a_col;
    v_pos = a_pos;
    gl_Position = u_vp * vec4(a_pos, 1.0);
}
"#;

const FS: &str = r#"
precision mediump float;
in vec3 v_nrm;
in vec4 v_col;
in vec3 v_pos;
uniform vec3 u_eye;
uniform int u_lit;
out vec4 o_col;
void main() {
    if (u_lit == 0) { o_col = v_col; return; }
    vec3 n = normalize(v_nrm);
    vec3 v = normalize(u_eye - v_pos);
    if (dot(n, v) < 0.0) n = -n;
    vec3 l1 = normalize(vec3(0.4, -0.6, 0.9));
    vec3 l2 = normalize(v);
    float d = 0.35 + 0.4 * max(dot(n, l1), 0.0) + 0.35 * max(dot(n, l2), 0.0);
    vec3 h = normalize(l2 + l1);
    float s = pow(max(dot(n, h), 0.0), 40.0) * 0.12;
    o_col = vec4(v_col.rgb * d + vec3(s), v_col.a);
}
"#;

impl Renderer {
    pub fn set_batches(&mut self, b: Vec<Batch>) {
        self.pending = Some(b);
    }

    unsafe fn init(&mut self, gl: &glow::Context) {
        if self.program.is_some() {
            return;
        }
        unsafe {
            let ver = "#version 150\n";
            let prog = gl.create_program().expect("program");
            let mut shaders = vec![];
            for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(ty).expect("shader");
                gl.shader_source(sh, &format!("{ver}{src}"));
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    eprintln!("shader error: {}", gl.get_shader_info_log(sh));
                }
                gl.attach_shader(prog, sh);
                shaders.push(sh);
            }
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.bind_attrib_location(prog, 1, "a_nrm");
            gl.bind_attrib_location(prog, 2, "a_col");
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) {
                eprintln!("link error: {}", gl.get_program_info_log(prog));
            }
            for s in shaders {
                gl.detach_shader(prog, s);
                gl.delete_shader(s);
            }
            self.program = Some(prog);
        }
    }

    unsafe fn upload(&mut self, gl: &glow::Context) {
        let Some(new) = self.pending.take() else { return };
        unsafe {
            for b in self.batches.drain(..) {
                gl.delete_vertex_array(b.vao);
                gl.delete_buffer(b.vbo);
            }
            for b in new {
                if b.data.is_empty() {
                    continue;
                }
                let vao = gl.create_vertex_array().unwrap();
                let vbo = gl.create_buffer().unwrap();
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let bytes: &[u8] = std::slice::from_raw_parts(b.data.as_ptr() as *const u8, b.data.len() * 4);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
                let stride = (FLOATS_PER_VERT * 4) as i32;
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
                gl.enable_vertex_attrib_array(1);
                gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);
                gl.enable_vertex_attrib_array(2);
                gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 24);
                gl.bind_vertex_array(None);
                self.batches.push(GpuBatch { vao, vbo, count: (b.data.len() / FLOATS_PER_VERT) as i32, pass: b.pass });
            }
        }
    }

    pub unsafe fn paint(&mut self, gl: &glow::Context) {
        unsafe {
            self.init(gl);
            self.upload(gl);
            let prog = self.program.unwrap();
            gl.clear_color(0.93, 0.94, 0.96, 1.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(prog));
            let vp = self.view_proj.to_cols_array();
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(prog, "u_vp").as_ref(), false, &vp);
            gl.uniform_3_f32(gl.get_uniform_location(prog, "u_eye").as_ref(), self.eye.x, self.eye.y, self.eye.z);
            let lit = gl.get_uniform_location(prog, "u_lit");
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::CULL_FACE);
            for pass in [Pass::Opaque, Pass::Lines, Pass::Overlay, Pass::Transparent] {
                match pass {
                    Pass::Opaque => {
                        gl.disable(glow::BLEND);
                        gl.depth_mask(true);
                        gl.uniform_1_i32(lit.as_ref(), 1);
                    }
                    Pass::Lines => {
                        gl.enable(glow::BLEND);
                        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                        gl.uniform_1_i32(lit.as_ref(), 0);
                    }
                    Pass::Overlay => {
                        gl.enable(glow::POLYGON_OFFSET_FILL);
                        gl.polygon_offset(-1.0, -2.0);
                        gl.uniform_1_i32(lit.as_ref(), 1);
                    }
                    Pass::Transparent => {
                        gl.disable(glow::POLYGON_OFFSET_FILL);
                        gl.depth_mask(false);
                        gl.uniform_1_i32(lit.as_ref(), 1);
                    }
                }
                for b in self.batches.iter().filter(|b| b.pass == pass) {
                    gl.bind_vertex_array(Some(b.vao));
                    let mode = if pass == Pass::Lines { glow::LINES } else { glow::TRIANGLES };
                    gl.draw_arrays(mode, 0, b.count);
                }
            }
            gl.bind_vertex_array(None);
            gl.depth_mask(true);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.use_program(None);
        }
    }
}
