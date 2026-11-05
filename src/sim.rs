// Simulation settings shared by the UI and the GPU solver (gpu.rs + sim.wgsl).
// Units: temperature in kK above ambient, positions in metres (sim coords: x, up, z).

pub const AMBIENT_K: f32 = 300.0;
pub const FPS: f32 = 60.0;

pub type V3 = [f32; 3];
/// Rotation matrix, rows. `mat_mul(R, v)` turns a local direction into a world one.
pub type M3 = [[f32; 3]; 3];
pub const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mat_mul(m: &M3, v: V3) -> V3 {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}
pub fn transpose(m: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| m[j][i]))
}
fn mm(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// Rotation from the panel's X/Y/Z degrees (Z-up axes) expressed in sim axes (Y-up).
pub fn rotation_ui(deg: V3) -> M3 {
    let [x, y, z] = deg.map(|d| d.to_radians());
    // UI X = sim x, UI Y = sim -z, UI Z = sim y
    let rx: M3 = [[1.0, 0.0, 0.0], [0.0, x.cos(), x.sin()], [0.0, -x.sin(), x.cos()]];
    let ry: M3 = [[y.cos(), -y.sin(), 0.0], [y.sin(), y.cos(), 0.0], [0.0, 0.0, 1.0]];
    let rz: M3 = [[z.cos(), 0.0, z.sin()], [0.0, 1.0, 0.0], [-z.sin(), 0.0, z.cos()]];
    mm(&rz, &mm(&ry, &rx))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    Sphere,
    Box,
    Cylinder,
    Torus,
}

impl Shape {
    pub fn id(self) -> f32 {
        match self {
            Shape::Sphere => 0.0,
            Shape::Box => 1.0,
            Shape::Cylinder => 2.0,
            Shape::Torus => 3.0,
        }
    }
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
    pub rot: M3,
}

impl ShapeInst {
    pub fn sphere(center: V3, r: f32) -> Self {
        ShapeInst { kind: Shape::Sphere, center, half: [r; 3], torus: 0.3, hollow: 0.0, gradient: None, noise: None, burst: None, wander: 0.0, seed: 0.0, rot: IDENTITY }
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
    /// Pressure noise: intensity (0..1), feature size (m), seed, speed.
    pub pressure_noise: [f32; 4],
    pub shadow_caster: bool,
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
    /// Field masks: constant, velocity, temperature, smoke (0..1).
    pub masks: [f32; 4],
    /// Temperature range (K) and smoke range (fraction) for the masks.
    pub ranges: [f32; 4],
    /// Noise: octaves, lacunarity, gain, curl (1/0).
    pub noise: [f32; 4],
    pub bias: V3,
    pub seed: f32,
    /// 0 volume, 1 particles, 2 both
    pub mode: u32,
    pub rot: M3,
}

#[derive(Clone)]
pub struct SimParams {
    /// Simulated time per frame, relative to the 60 Hz default (60 / Timestep).
    pub time_scale: f32,
    pub substeps: usize,
    pub freeze: bool,
    /// 0 combustion, 1 smoke only, 2 fire only
    pub mode: u32,
    pub vel_cubic: bool,
    pub vox_cubic: bool,
    /// 0 first, 1 second, 2 third order
    pub order: u32,
    pub adv_offset: V3, // m/s, sim axes
    pub hourglass: f32,
    pub reuse_pressure: bool,
    pub leak_guard: bool,
    pub collider_removal: f32,
    pub iterations: usize,
    pub sor_share: f32,
    pub omega: f32,
    // combustion (fractions, 1 = the old defaults)
    pub burn_rate: f32,
    pub ignition_k: f32,
    pub ignition_soft_k: f32,
    pub max_burn_k: f32,
    pub heat: f32,
    pub smoke_yield: f32,
    pub flames_yield: f32,
    pub expansion: f32,
    /// Dissipation per frame: temperature, velocity, smoke, flames, fuel (multiplicative)
    pub diss: [f32; 5],
    /// Subtractive dissipation per frame: smoke, flames, fuel
    pub sub: [f32; 3],
    /// Diffusion blend per frame: temperature, smoke, fuel, flames, velocity
    pub diffusion: [f32; 5],
    pub vorticity: f32,
    pub vorticity_large: f32,
    /// Vorticity masks: velocity, temperature, smoke, constant
    pub vort_masks: [f32; 4],
    /// temperature min/max (K), smoke min/max (fraction)
    pub vort_ranges: [f32; 4],
    pub vort_ramp: f32,
    pub tightness: f32,
    pub buoyancy: f32,
    pub smoke_weight: f32,
    pub fuel_weight: f32,
    /// Solid sides: bit 1 ground, 2 ceiling, 4 -x, 8 +x, 16 -z, 32 +z (sim axes)
    pub walls: u32,
    /// Wind direction (sim axes, unnormalised), direction randomisation, speed m/s, chaos m/s
    pub wind_dir: V3,
    pub wind_rand: V3,
    pub wind_speed: f32,
    pub wind_chaos: f32,
    /// Shredding: intensity, threshold K, width K
    pub shred: [f32; 3],
    /// Domain mask: shape (0 none, 1 sphere, 2 box), intensity, distance
    pub mask: [f32; 3],
    pub forces: Vec<Force>,
    pub colliders: Vec<ShapeInst>,
    pub emitters: Vec<Emitter>,
}

/// Grid size, cell size (m) and domain origin (m, sim axes) from the Simulation node.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Domain {
    pub n: [usize; 3],
    pub h: f32,
    pub origin: V3,
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
    #[test]
    fn rotations_are_orthonormal() {
        let r = rotation_ui([30.0, -45.0, 60.0]);
        let v = mat_mul(&r, [0.3, -0.4, 0.5]);
        let back = mat_mul(&transpose(&r), v);
        assert!((back[0] - 0.3).abs() < 1e-5 && (back[1] + 0.4).abs() < 1e-5 && (back[2] - 0.5).abs() < 1e-5);
    }
}
