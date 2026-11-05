// Camera and shading settings for the GPU raymarcher (render.wgsl), plus small vector helpers.

pub type V3 = [f32; 3];
pub fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
pub fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
pub fn norm(a: V3) -> V3 { mul(a, 1.0 / dot(a, a).sqrt().max(1e-9)) }

#[derive(Clone)]
pub struct Camera {
    pub yaw: f32,   // degrees
    pub pitch: f32, // degrees
    pub dist: f32,
    pub fov: f32, // degrees
    pub target: V3,
    pub ortho: bool,
}

impl Camera {
    /// (eye, forward, right, up)
    pub fn basis(&self) -> (V3, V3, V3, V3) {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        let eye = add(self.target, mul([p.cos() * y.sin(), p.sin(), p.cos() * y.cos()], self.dist));
        let f = norm(sub(self.target, eye));
        let r = norm(cross(f, [0.0, 1.0, 0.0]));
        let u = cross(r, f);
        (eye, f, r, u)
    }

    /// World point -> normalised screen coords (0..1, y down), None if behind.
    pub fn project(&self, p: V3, aspect: f32) -> Option<[f32; 2]> {
        let (eye, f, r, u) = self.basis();
        let d = sub(p, eye);
        let z = dot(d, f);
        if z <= 0.01 {
            return None;
        }
        let tanf = (self.fov.to_radians() * 0.5).tan();
        let s = if self.ortho { self.dist * tanf } else { z * tanf };
        let x = dot(d, r) / (s * aspect);
        let y = dot(d, u) / s;
        Some([(x + 1.0) * 0.5, (1.0 - y) * 0.5])
    }
}

#[derive(Clone)]
pub struct Shade {
    pub lit: bool,
    pub ambient: V3,
    pub sun: Option<(V3, V3)>, // (colour * intensity, direction towards light)
    /// Fire colour lookup table (256 entries) and how it is applied:
    /// 0 exposed gradient, 1 gradient, 2 blackbody, 3 single colour.
    pub lut: Vec<[f32; 3]>,
    pub coloring: u32,
    pub fire_color: V3,
    /// Coloring remap range (K) and ramp; clamp to the range
    pub remap: [f32; 2],
    pub remap_ramp: f32,
    pub clamp_remap: bool,
    pub shaping_flames: f32,
    pub shaping_density: f32,
    pub flames_min: f32,
    pub flames_scale: f32,
    pub flames_opacity: f32,
    pub flames_translucent: bool,
    pub render_flames: bool,
    pub render_smoke: bool,
    pub render_fuel: bool,
    pub smoke_color: V3,
    pub smoke_scale: f32,
    pub smoke_min: f32,
    pub smoke_clamp: f32,
    /// smoke colouring: 0 single colour, 1 gradient; remap range/ramp; remap by temperature
    pub smoke_coloring: u32,
    pub smoke_remap: [f32; 2],
    pub smoke_remap_ramp: f32,
    pub smoke_by_temp: bool,
    pub fuel_color: V3,
    pub fuel_scale: f32,
    // lighting
    pub shadow_density: f32,
    pub shadow_intensity: f32,
    pub shadow_sharpness: f32,
    pub shadow_steps: f32,
    pub shadow_reach: f32,
    pub anisotropy: f32,
    pub ao: f32,
    pub extinction: V3,
    // scattering
    pub direct: f32,
    pub flames_light: f32,
    pub particles_light: f32,
    pub scatter_radius: f32,
    pub scatter_occlusion: f32,
    /// emissive masking: active, reference, width, ramp, intensity, low/mid/high
    pub emask: [f32; 8],
    /// raymarch sharpening (smoke, flames, temperature) or None
    pub sharpen: Option<[f32; 3]>,
    /// volume post process: mode, strength; post modulation: channel (-1 off), freq /m, intensity, speed
    pub post: [f32; 6],
    /// slicing mask: X-, X+, Y-, Y+, Z-, Z+ (offset, width as fractions) and ramp
    pub slicing: [f32; 13],
    pub points: Vec<(V3, V3, f32)>, // point lights: position, colour*intensity, range
    /// shapes drawn in the render: (shape, colour)
    pub shapes: Vec<(crate::sim::ShapeInst, V3)>,
    /// 0 holdout, 1 visible, 2 hidden; masking 0 none, 1 inside, 2 outside; invert
    pub shape_mode: [u32; 3],
    pub sparks: bool,
    pub sparks_hq: bool,
    pub spark_group: u32,
    pub render_volume: bool,
    pub render_particles: bool,
    pub volume_occludes_particles: bool,
    pub particles_occlude_volume: bool,
    pub detail: f32, // sub-cell detail noise, in cells (Upscaling)
    pub fast_detail: bool,
    pub sky: Option<(V3, V3)>, // (top, bottom)
    pub ground: Option<(V3, f32)>,
    pub step: f32, // cells per march step
    /// what to output: 0 all, 1 alpha, 2 emission, 3 scattering, 4 temperature, 5 density, 6 fuel, 7 depth
    pub pass: u32,
}

/// Picture finishing (Scene node) and alpha handling (Render node).
#[derive(Clone)]
pub struct Post {
    pub gamma: f32,
    pub exposure: f32,
    pub vibrance: f32,
    pub tonemap: u32,
    pub dither: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub swizzle: u32,
    pub bloom: f32,
    pub bloom_radius: f32,
    /// width px, offset px, roundness 0..1, intensity 0..1
    pub vignette: [f32; 4],
    pub style: u32,
    pub style_size: f32,
    /// RGB, hue, value, saturation levels (0 = off)
    pub quant: [f32; 4],
    pub quant_dither: f32,
    pub dither_mode: u32,
    pub alpha_limit: f32,
    pub alpha_sharpen: f32,
    pub remap_style: u32,
    /// 0 premultiplied, 1 straight, 2 opaque
    pub alpha_mode: u32,
    pub invert_alpha: bool,
}

impl Default for Post {
    fn default() -> Self {
        Post {
            gamma: 2.2,
            exposure: 1.0,
            vibrance: 0.307,
            tonemap: 4,
            dither: 0.0,
            contrast: 0.0,
            brightness: 0.0,
            saturation: 0.05,
            swizzle: 0,
            bloom: 1.5,
            bloom_radius: 0.2,
            vignette: [300.0, 100.0, 0.5, 0.0],
            style: 0,
            style_size: 4.0,
            quant: [0.0; 4],
            quant_dither: 0.0,
            dither_mode: 0,
            alpha_limit: 0.0,
            alpha_sharpen: 0.0,
            remap_style: 0,
            alpha_mode: 2,
            invert_alpha: false,
        }
    }
}
