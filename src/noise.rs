// CPU copy of the GPU noise (sim.wgsl): used for the noise preview, force gizmos and particles.
// The hash and interpolation match the shader bit for bit, so previews show what the solver does.

pub type V3 = [f32; 3];

fn hash(p: [i32; 3]) -> f32 {
    let mut h = (p[0] as u32).wrapping_mul(73856093) ^ (p[1] as u32).wrapping_mul(19349663) ^ (p[2] as u32).wrapping_mul(83492791);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

pub fn vnoise(p: V3) -> f32 {
    let i = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let f = [p[0] - p[0].floor(), p[1] - p[1].floor(), p[2] - p[2].floor()];
    let u = f.map(|x| x * x * (3.0 - 2.0 * x));
    let h = |a: i32, b: i32, c: i32| hash([i[0] + a, i[1] + b, i[2] + c]);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = mix(h(0, 0, 0), h(1, 0, 0), u[0]);
    let x10 = mix(h(0, 1, 0), h(1, 1, 0), u[0]);
    let x01 = mix(h(0, 0, 1), h(1, 0, 1), u[0]);
    let x11 = mix(h(0, 1, 1), h(1, 1, 1), u[0]);
    mix(mix(x00, x10, u[1]), mix(x01, x11, u[1]), u[2])
}

pub fn vnoise3(p: V3) -> V3 {
    [
        vnoise(p) - 0.5,
        vnoise([p[0] + 31.4, p[1] + 7.1, p[2] + 3.3]) - 0.5,
        vnoise([p[0] + 5.9, p[1] + 71.7, p[2] + 13.1]) - 0.5,
    ]
}

pub fn curl(p: V3) -> V3 {
    let e = 0.15;
    let s = |dx: f32, dy: f32, dz: f32| vnoise3([p[0] + dx, p[1] + dy, p[2] + dz]);
    let (px0, px1) = (s(-e, 0.0, 0.0), s(e, 0.0, 0.0));
    let (py0, py1) = (s(0.0, -e, 0.0), s(0.0, e, 0.0));
    let (pz0, pz1) = (s(0.0, 0.0, -e), s(0.0, 0.0, e));
    [
        ((py1[2] - py0[2]) - (pz1[1] - pz0[1])) / (2.0 * e),
        ((pz1[0] - pz0[0]) - (px1[2] - px0[2])) / (2.0 * e),
        ((px1[1] - px0[1]) - (py1[0] - py0[0])) / (2.0 * e),
    ]
}

/// Offset in noise space for a seed (same formula as the shader).
pub fn seed_offset(seed: f32) -> V3 {
    let fr = |x: f32| x - x.floor();
    [fr(seed * 0.1234) * 1000.0, fr(seed * 0.3517) * 1000.0, fr(seed * 0.7311) * 1000.0]
}

/// Force: Noise settings in shader units.
#[derive(Clone, Copy, Debug)]
pub struct NoiseParams {
    pub freq: f32, // per metre
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub curl: bool,
    pub seed: f32,
    pub speed: f32,
}

/// Fractal noise vector at a position (metres) and time (s), roughly in -1..1 per axis.
pub fn fbm(np: &NoiseParams, pos: V3, t: f32) -> V3 {
    let o = seed_offset(np.seed);
    let mut q = [pos[0] * np.freq + o[0], pos[1] * np.freq - t * np.speed + o[1], pos[2] * np.freq + t * np.speed * 0.35 + o[2]];
    let (mut amp, mut sum, mut norm) = (1.0, [0.0f32; 3], 0.0);
    for _ in 0..np.octaves.clamp(1, 8) {
        let f = if np.curl { curl(q) } else { vnoise3(q).map(|x| x * 2.0) };
        for k in 0..3 {
            sum[k] += f[k] * amp;
        }
        norm += amp;
        amp *= np.gain;
        q = q.map(|x| x * np.lacunarity);
    }
    sum.map(|x| x / norm.max(1e-6))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noise_is_smooth_and_bounded() {
        let np = NoiseParams { freq: 4.0, octaves: 4, lacunarity: 2.0, gain: 0.5, curl: true, seed: 330.0, speed: 1.0 };
        let a = fbm(&np, [0.1, 0.2, 0.3], 0.0);
        let b = fbm(&np, [0.1001, 0.2, 0.3], 0.0);
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05), "neighbouring samples are close");
        assert!(a.iter().all(|x| x.abs() < 4.0));
    }
}
