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
}
