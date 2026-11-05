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
    pub grad: [V3; 3],
    pub t_min_k: f32,
    pub t_max_k: f32,
    pub brightness: f32,
    pub smoke_color: V3,
    pub absorption: f32,
    pub shadow: f32,
    pub detail: f32, // sub-cell detail noise, in cells
    pub point: Option<(V3, V3, f32)>, // point light: position, colour*intensity, range
    pub sparks: bool,
    pub density: f32,
    pub temp_mul: f32,
    pub sky: Option<(V3, V3)>, // (top, bottom)
    pub ground: Option<(V3, f32)>,
    pub step: f32, // cells per march step
    /// Everything added after 0.4 (Shading, Scene, Volume Processing, Render settings).
    /// `Look::default()` draws exactly the 0.4 picture.
    pub look: Look,
}

/// Rendering settings added after 0.4. Every default leaves the 0.4 picture unchanged.
#[derive(Clone)]
pub struct Look {
    // Fire Color Gradient: 256 colours when it is more than 0.4's three stops (None = 0.4 colours)
    pub lut: Option<Vec<V3>>,
    // Shading › Flames
    pub render_flames: bool,
    pub remap_ramp: f32,
    pub clamp_remap: bool,
    pub shaping_flames: f32,
    pub shaping_density: f32,
    pub flames_coloring: u32, // 0 exposed gradient, 1 gradient, 2 blackbody, 3 single colour
    pub fire_color: V3,
    pub translucent: bool,
    pub flames_min: f32,
    pub flames_opacity: f32,
    // Shading › Smoke
    pub render_smoke: bool,
    pub smoke_remap: Option<(bool, f32, f32, f32)>, // by temperature?, min, max, ramp (gradient colouring)
    pub smoke_min: f32,
    pub smoke_clamp: f32, // 10 = no clamp
    // Shading › Fuel
    pub fuel: Option<(V3, f32)>,
    // Shading › Lighting / Scattering
    pub shadow_sharpness: f32, // 0.75 = 0.4
    pub anisotropy: f32,       // 0 = 0.4 (isotropic)
    pub ao: f32,
    pub extinction: V3,
    pub shadow_gamma: f32, // 1 = 0.4
    pub shadow_steps: f32, // 64 = 0.4
    pub shadow_reach: f32, // 1 = 0.4
    pub direct: f32,       // 1 = 0.4
    pub flames_light: f32,
    pub particles_light: f32,
    pub scatter_radius: f32,
    pub scatter_occlusion: f32,
    // Shading › Emissive masking: density ref, width, ramp, intensity, low, mid, high
    pub emask: Option<[f32; 7]>,
    // Shading › Rendering: smoke, flames, temperature sharpening (1 = none)
    pub sharpen: Option<V3>,
    // Volume Processing
    pub post: u32, // 0 none, 1 motion blur, 2 sharpen, 3 dilate
    pub post_strength: f32,
    pub modulation: Option<(u32, f32, f32, f32)>, // channel, noise scale (m), intensity, speed
    pub slice: [f32; 12],                          // X-, X+, Y-, Y+, Z-, Z+: offset, width (0..1)
    pub slice_ramp: f32,
    // Render › Render Settings
    pub render_volume: bool,
    pub render_particles: bool,
    pub vol_occludes_particles: bool,
    pub particles_occlude_vol: bool,
    pub shapes: Vec<(crate::sim::ShapeInst, V3)>, // shown emitter shapes and their colours
    pub shapes_render: u32,                       // 0 holdout, 1 visible, 2 hidden
    pub shapes_mask: u32,                         // 0 none, 1 mask inside, 2 mask outside
    pub shapes_invert: bool,
    // Export: render pass (0 = the picture), alpha (0 opaque, 1 premultiplied, 2 straight), inverted alpha
    pub pass: u32,
    pub alpha: u32,
    pub invert_alpha: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            lut: None,
            render_flames: true,
            remap_ramp: 1.0,
            clamp_remap: true,
            shaping_flames: 0.0,
            shaping_density: 0.0,
            flames_coloring: 0,
            fire_color: [1.0, 0.45, 0.12],
            translucent: false,
            flames_min: 0.0,
            flames_opacity: 0.0,
            render_smoke: true,
            smoke_remap: None,
            smoke_min: 0.0,
            smoke_clamp: 10.0,
            fuel: None,
            shadow_sharpness: 0.75,
            anisotropy: 0.0,
            ao: 0.0,
            extinction: [1.0; 3],
            shadow_gamma: 1.0,
            shadow_steps: 64.0,
            shadow_reach: 1.0,
            direct: 1.0,
            flames_light: 0.0,
            particles_light: 0.0,
            scatter_radius: 0.15,
            scatter_occlusion: 0.75,
            emask: None,
            sharpen: None,
            post: 0,
            post_strength: 0.5,
            modulation: None,
            slice: [0.0; 12],
            slice_ramp: 1.0,
            render_volume: true,
            render_particles: true,
            vol_occludes_particles: false,
            particles_occlude_vol: false,
            shapes: vec![],
            shapes_render: 0,
            shapes_mask: 0,
            shapes_invert: false,
            pass: 0,
            alpha: 0,
            invert_alpha: false,
        }
    }
}

/// Scene node picture finishing (added after 0.4). `Finish::default()` = 0.4: ACES, gamma 2.2.
#[derive(Clone, PartialEq)]
pub struct Finish {
    pub gamma: f32,
    pub exposure: f32,
    pub vibrance: f32,
    pub tonemap: u32, // 0 none, 1 Reinhard, 2 ACES, 3 Filmic, 4 Uchimura, 5 AgX
    pub dither: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub swizzle: u32,
    pub bloom: f32,
    pub bloom_radius: f32,
    pub vignette: [f32; 4], // width px, offset px, roundness, intensity
    pub style: u32,
    pub style_size: f32,
    pub quant: [f32; 4], // rgb, hue, value, saturation levels (0 = off)
    pub quant_dither: f32,
    pub dither_mode: u32,
    pub translucency_limit: f32,
    pub translucency_sharpen: f32,
    pub remap_style: u32,
}

impl Default for Finish {
    fn default() -> Self {
        Finish {
            gamma: 2.2,
            exposure: 1.0,
            vibrance: 0.0,
            tonemap: 2,
            dither: 0.0,
            contrast: 0.0,
            brightness: 0.0,
            saturation: 0.0,
            swizzle: 0,
            bloom: 0.0,
            bloom_radius: 0.2,
            vignette: [300.0, 100.0, 0.5, 0.0],
            style: 0,
            style_size: 4.0,
            quant: [0.0; 4],
            quant_dither: 0.0,
            dither_mode: 0,
            translucency_limit: 0.0,
            translucency_sharpen: 0.0,
            remap_style: 0,
        }
    }
}

impl Finish {
    /// The 0.4 picture: no finishing pass needed.
    pub fn is_04(&self) -> bool {
        let mut d = Finish::default();
        d.bloom_radius = self.bloom_radius;
        d.vignette[0..3].copy_from_slice(&self.vignette[0..3]);
        d.style_size = self.style_size;
        d.dither_mode = self.dither_mode;
        *self == d
    }
}
