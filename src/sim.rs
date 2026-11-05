// Simulation settings shared by the UI and the GPU solver (gpu.rs + sim.wgsl).
// Units: temperature in kK above ambient, positions in metres (sim coords: x, up, z).

pub const AMBIENT_K: f32 = 300.0;
pub const FPS: f32 = 60.0;

pub type V3 = [f32; 3];

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    Sphere,
    Box,
    Cylinder,
    Torus,
}

#[derive(Clone)]
pub struct Chaos {
    pub strength: f32,
    pub scale: f32,
    pub speed: f32,
}

/// Expanding shell (Shape: Burst).
#[derive(Clone)]
pub struct Burst {
    pub radius: f32,
    pub duration: f32,
    pub thickness: f32,
    pub start: f32,
}

/// A region of space produced by the Shape nodes.
#[derive(Clone)]
pub struct ShapeInst {
    pub kind: Shape,
    pub center: V3,
    pub half: V3, // half size per axis (m)
    pub torus: f32,
    pub hollow: f32,
    pub gradient: Option<f32>,
    pub noise: Option<Chaos>,
    pub burst: Option<Burst>,
    pub wander: f32, // m/s of random drift (Shape: Particles)
    pub seed: f32,
}

impl ShapeInst {
    pub fn sphere(center: V3, r: f32) -> Self {
        ShapeInst { kind: Shape::Sphere, center, half: [r; 3], torus: 0.3, hollow: 0.0, gradient: None, noise: None, burst: None, wander: 0.0, seed: 0.0 }
    }
}

/// Emitter: Particles settings; particles are evaluated over time on the CPU.
#[derive(Clone)]
pub struct Particles {
    pub count: usize,
    pub lifetime: f32,
    pub speed: f32,
    pub spread_deg: f32,
    pub dir: V3,
    pub gravity: f32,
    pub size: f32,
    pub seed: f32,
    pub sparks: bool,
    pub brightness: f32,
}

#[derive(Clone)]
pub struct Emitter {
    pub shapes: Vec<ShapeInst>,
    pub particles: Option<Particles>,
    pub fuel: f32,
    pub smoke: f32,
    pub temp_k: f32,
    pub gradient: f32,
    pub expansion: f32,
    pub vel: V3,
    pub spread: f32,
    pub fuel_add: bool,
    pub smoke_add: bool,
    pub temp_replace: bool,
    pub burst: f32, // seconds of emission per cycle (0 = continuous)
    pub gap: f32,   // seconds between bursts
    pub chaos: Option<Chaos>,
}

/// Force nodes. `kind`: 0 line, 1 point, 2 toroidal, 3 noise, 4 vector field, 5 vorticles.
#[derive(Clone)]
pub struct Force {
    pub kind: u32,
    pub strength: f32,
    pub scale: f32,
    pub speed: f32,
    pub pos: V3,
    pub radius: f32,
    pub dir: V3,
    pub extra: f32,
    pub mask: Option<ShapeInst>,
    pub softness: f32,
}

#[derive(Clone)]
pub struct SimParams {
    pub time_scale: f32,
    pub buoyancy: f32,
    pub smoke_weight: f32,
    pub vorticity: f32,
    pub burn_rate: f32,
    pub ignition_k: f32,
    pub heat: f32,
    pub smoke_yield: f32,
    pub cooling: f32,
    pub smoke_fade: f32,
    pub wind: [f32; 2],
    pub iterations: usize,
    pub substeps: usize,
    pub forces: Vec<Force>,
    pub colliders: Vec<ShapeInst>,
    pub emitters: Vec<Emitter>,
}

/// Grid size for Low / Medium / High / Ultra (1.5 times taller than wide).
pub fn dims(res: usize) -> [usize; 3] {
    match res {
        0 => [96, 144, 96],
        2 => [224, 336, 224],
        3 => [288, 432, 288],
        _ => [160, 240, 160],
    }
}

fn hash(a: f32, b: f32) -> f32 {
    let h = ((a * 127.1 + b * 311.7).sin() * 43758.547).fract();
    h.abs()
}

/// A particle at time t: (position, velocity, life fraction 0..1).
pub fn particle(p: &Particles, origin: V3, k: usize, t: f32) -> (V3, V3, f32) {
    let life = p.lifetime.max(0.05);
    let phase = k as f32 / p.count.max(1) as f32 * life;
    let age = (t + phase) % life;
    let birth = ((t + phase) / life).floor();
    let r = |i: f32| hash(p.seed + k as f32 * 13.7 + i, birth);
    // direction within a cone around `dir`
    let d = {
        let l = (p.dir[0] * p.dir[0] + p.dir[1] * p.dir[1] + p.dir[2] * p.dir[2]).sqrt().max(1e-4);
        [p.dir[0] / l, p.dir[1] / l, p.dir[2] / l]
    };
    let ang = p.spread_deg.to_radians() * r(1.0).sqrt();
    let rot = r(2.0) * std::f32::consts::TAU;
    let helper = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let cross = |a: V3, b: V3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let u = cross(d, helper);
    let ul = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt().max(1e-4);
    let u = [u[0] / ul, u[1] / ul, u[2] / ul];
    let w = cross(d, u);
    let dir: V3 = std::array::from_fn(|i| d[i] * ang.cos() + (u[i] * rot.cos() + w[i] * rot.sin()) * ang.sin());
    let speed = p.speed * (0.7 + 0.6 * r(3.0));
    let v0: V3 = std::array::from_fn(|i| dir[i] * speed);
    let mut pos: V3 = std::array::from_fn(|i| origin[i] + v0[i] * age);
    pos[1] += 0.5 * p.gravity * age * age;
    let mut vel = v0;
    vel[1] += p.gravity * age;
    (pos, vel, age / life)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn particles_fly_and_fall() {
        let p = Particles { count: 4, lifetime: 1.0, speed: 2.0, spread_deg: 0.0, dir: [0.0, 1.0, 0.0], gravity: -10.0, size: 0.05, seed: 1.0, sparks: true, brightness: 1.0 };
        let (a, _, la) = particle(&p, [0.0; 3], 0, 0.1);
        let (b, vb, lb) = particle(&p, [0.0; 3], 0, 0.5);
        assert!(lb > la && b[1] != a[1]);
        assert!(vb[1] < 2.0, "gravity slows the climb");
    }
}
