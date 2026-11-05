// GPU fluid solver. Grid idx = x + nx*(y + ny*z), y up. Cell centres at integer coords.
// vel: staggered (u, v, w) in cells/frame. sca: (temperature kK above ambient, smoke, fuel, flames).

struct Emitter {
    pos: vec4f,    // centre (cells), shadow caster
    size: vec4f,   // half size per axis (cells), torus thickness
    rates: vec4f,  // fuel/frame, smoke/frame, temperature, gradient
    vel: vec4f,    // velocity (cells/frame), spread
    flags: vec4f,  // shape, fuel add, smoke add, temperature replace
    chaos: vec4f,  // strength, frequency (/cell), speed, pressure push
    extra: vec4f,  // hollow, fade, pressure noise intensity, pressure noise frequency (/cell)
    extra2: vec4f, // pressure noise seed, pressure noise speed, _, _
    r0: vec4f,     // world -> local rotation rows
    r1: vec4f,
    r2: vec4f,
}

struct Force {
    a: vec4f,     // type, strength, scale, speed
    pos: vec4f,   // position (cells), radius (cells)
    dir: vec4f,   // direction / axis, extra (tube radius cells or field mode)
    mpos: vec4f,  // mask centre (cells), mask on
    msize: vec4f, // mask half size (cells), mask shape
    mext: vec4f,  // mask torus thickness, softness, acts on volume (1/0), _
    mk: vec4f,    // field masks: constant, velocity, temperature, smoke
    mr: vec4f,    // temperature range min/max (kK), smoke range min/max
    nz: vec4f,    // octaves, lacunarity, gain, curl
    nb: vec4f,    // bias xyz, seed
    r0: vec4f,    // world -> local rotation rows
    r1: vec4f,
    r2: vec4f,
}

struct Coll {
    pos: vec4f,   // centre (cells), shape
    size: vec4f,  // half size (cells), torus thickness
}

struct Params {
    n: vec4u,   // nx, ny, nz, emitter count
    a: vec4f,   // dt, buoyancy, smoke weight, fuel weight
    b: vec4f,   // burn rate, ignition (kK), heat, smoke yield
    b2: vec4f,  // flames yield, combustion expansion, ignition softness (kK), max burn temperature (kK)
    c: vec4f,   // dissipation per frame: temperature, velocity, smoke, flames
    c2: vec4f,  // fuel dissipation, subtractive smoke, flames, fuel
    df: vec4f,  // diffusion blend: temperature, smoke, fuel, flames
    d: vec4f,   // force count, collider count, mode (0 combustion 1 smoke 2 fire), time (s)
    e: vec4f,   // cell size (m), res scale, velocity diffusion, tightness
    v1: vec4f,  // vorticity, large vorticity, vorticity ramp, _
    vm: vec4f,  // vorticity masks: velocity, temperature, smoke, constant
    vr: vec4f,  // vorticity temperature range (kK), smoke range
    w: vec4f,   // wind goal (cells/frame), wind chaos (cells/frame)
    sh: vec4f,  // shredding intensity, threshold (kK), width (kK), _
    mk: vec4f,  // domain mask shape, intensity, distance, solid walls (bits)
    ad: vec4f,  // advection offset (cells/frame), order (0..2)
    fl: vec4f,  // velocity cubic, voxel cubic, leak guard, _
    pr: vec4f,  // hourglass, collider removal, relaxation, _
    // written at render time:
    l: vec4f,   // light dir (xyz), shadow k
    l2: vec4f,  // shadow step (cells), shadow steps, ao on, glow radius (cells)
    l3: vec4f,  // glow remap min (kK), max (kK), ramp, glow occlusion
    pk: vec4f,  // sharpening: smoke, flames, temperature, on
    pp: vec4f,  // post process mode, strength, modulation channel (-1 off), modulation frequency (/cell)
    pm: vec4f,  // modulation intensity, speed, _, _
    em: array<Emitter, 32>,
    fo: array<Force, 8>,
    co: array<Coll, 8>,
}

@group(0) @binding(0) var<uniform> prm: Params;
// Lean layout for big grids: sca1 doubles as the new-velocity buffer, tmp as the curl buffer.
@group(0) @binding(1) var<storage, read_write> vel0: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> sca0: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> sca1: array<vec4f>;
@group(0) @binding(4) var<storage, read_write> tmp: array<vec4f>;
@group(0) @binding(5) var<storage, read_write> p0: array<f32>;
@group(0) @binding(6) var<storage, read_write> p1: array<f32>;
@group(0) @binding(7) var<storage, read_write> dvg: array<f32>;
@group(0) @binding(8) var<storage, read_write> light: array<vec4f>; // sun transmittance, ambient occlusion, glow, glow temperature
@group(0) @binding(9) var vol: texture_storage_3d<rgba16float, write>;  // temperature, smoke, sun, flames
@group(0) @binding(10) var<storage, read_write> occ: array<atomic<u32>>; // 8^3 bricks with content
@group(0) @binding(11) var vol2: texture_storage_3d<rgba16float, write>; // fuel, ambient occlusion, glow, glow temperature
@group(0) @binding(12) var<storage, read_write> hist: array<atomic<u32>>; // 64 temperature + 64 smoke bins

fn dims() -> vec3i { return vec3i(prm.n.xyz); }
fn idx(c: vec3i) -> u32 {
    let n = dims();
    let q = clamp(c, vec3i(0), n - 1);
    return u32(q.x + n.x * (q.y + n.y * q.z));
}
fn inside(g: vec3u) -> bool { return all(g < prm.n.xyz); }
fn wall(bit: u32) -> bool { return (u32(prm.mk.w) & bit) != 0u; }

struct Tri { i: array<u32, 8>, w: array<f32, 8> }
fn tri(p: vec3f) -> Tri {
    let q = clamp(p, vec3f(0.0), vec3f(dims() - 1));
    let b = vec3i(floor(q));
    let f = q - floor(q);
    var t: Tri;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3i(i32(k & 1u), i32((k >> 1u) & 1u), i32((k >> 2u) & 1u));
        t.i[k] = idx(b + o);
        let wv = select(1.0 - f, f, o == vec3i(1));
        t.w[k] = wv.x * wv.y * wv.z;
    }
    return t;
}
fn s_sca0(p: vec3f) -> vec4f {
    let t = tri(p);
    var r = vec4f(0.0);
    for (var k = 0u; k < 8u; k++) { r += sca0[t.i[k]] * t.w[k]; }
    return r;
}
fn s_sca1(p: vec3f) -> vec4f {
    let t = tri(p);
    var r = vec4f(0.0);
    for (var k = 0u; k < 8u; k++) { r += sca1[t.i[k]] * t.w[k]; }
    return r;
}

// Catmull-Rom weights for one axis
fn cr(t: f32) -> vec4f {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4f(-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2);
}
/// Cubic (Catmull-Rom) sample of the scalars, clamped to the surrounding values (no overshoot).
fn s_sca0_cubic(p: vec3f) -> vec4f {
    let q = clamp(p, vec3f(0.0), vec3f(dims() - 1));
    let b = vec3i(floor(q));
    let f = q - floor(q);
    let wx = cr(f.x);
    let wy = cr(f.y);
    let wz = cr(f.z);
    var r = vec4f(0.0);
    for (var k = 0; k < 4; k++) {
        for (var j = 0; j < 4; j++) {
            for (var i = 0; i < 4; i++) {
                r += sca0[idx(b + vec3i(i - 1, j - 1, k - 1))] * (wx[i] * wy[j] * wz[k]);
            }
        }
    }
    let t = tri(p);
    var lo = vec4f(1e9);
    var hi = vec4f(-1e9);
    for (var k = 0u; k < 8u; k++) {
        lo = min(lo, sca0[t.i[k]]);
        hi = max(hi, sca0[t.i[k]]);
    }
    return clamp(r, lo, hi);
}
fn adv_sca0(p: vec3f) -> vec4f {
    if (prm.fl.y > 0.5) { return s_sca0_cubic(p); }
    return s_sca0(p);
}

fn hash(p: vec3i) -> f32 {
    var h = (u32(p.x) * 73856093u) ^ (u32(p.y) * 19349663u) ^ (u32(p.z) * 83492791u);
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffu) / 65535.0;
}
fn vnoise(p: vec3f) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash(i), hash(i + vec3i(1, 0, 0)), u.x);
    let x10 = mix(hash(i + vec3i(0, 1, 0)), hash(i + vec3i(1, 1, 0)), u.x);
    let x01 = mix(hash(i + vec3i(0, 0, 1)), hash(i + vec3i(1, 0, 1)), u.x);
    let x11 = mix(hash(i + vec3i(0, 1, 1)), hash(i + vec3i(1, 1, 1)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}
fn vnoise3(p: vec3f) -> vec3f {
    return vec3f(vnoise(p), vnoise(p + vec3f(31.4, 7.1, 3.3)), vnoise(p + vec3f(5.9, 71.7, 13.1))) - 0.5;
}

/// Curl noise: a smooth, swirling, divergence-free vector field. It folds gas into sheets
/// instead of punching it full of holes the way plain random pushes do.
fn curl_noise(p: vec3f) -> vec3f {
    let e = 0.15;
    let dx = vec3f(e, 0.0, 0.0);
    let dy = vec3f(0.0, e, 0.0);
    let dz = vec3f(0.0, 0.0, e);
    let px0 = vnoise3(p - dx); let px1 = vnoise3(p + dx);
    let py0 = vnoise3(p - dy); let py1 = vnoise3(p + dy);
    let pz0 = vnoise3(p - dz); let pz1 = vnoise3(p + dz);
    return vec3f(
        (py1.z - py0.z) - (pz1.y - pz0.y),
        (pz1.x - pz0.x) - (px1.z - px0.z),
        (px1.y - px0.y) - (py1.x - py0.x)) / (2.0 * e);
}

fn seed_offset(seed: f32) -> vec3f {
    return vec3f(fract(seed * 0.1234), fract(seed * 0.3517), fract(seed * 0.7311)) * 1000.0;
}

/// Fractal noise force (same as noise.rs on the CPU).
fn fbm_force(wp: vec3f, fo: Force, t: f32) -> vec3f {
    let freq = fo.a.z;
    var q = wp * freq + seed_offset(fo.nb.w) + vec3f(0.0, -t * fo.a.w, t * fo.a.w * 0.35);
    var amp = 1.0;
    var sum = vec3f(0.0);
    var norm = 0.0;
    let oct = i32(clamp(fo.nz.x, 1.0, 8.0));
    for (var o = 0; o < oct; o++) {
        var f: vec3f;
        if (fo.nz.w > 0.5) { f = curl_noise(q); } else { f = vnoise3(q) * 2.0; }
        sum += f * amp;
        norm += amp;
        amp *= fo.nz.z;
        q *= fo.nz.y;
    }
    return sum / max(norm, 1e-6);
}

/// Normalised distance for a shape (d = offset / half size): < 1 inside.
fn shape_dist(d: vec3f, kind: f32, torus: f32) -> f32 {
    if (kind > 2.5) {
        let t = clamp(torus, 0.02, 0.98);
        let q = vec2f(length(d.xz) - (1.0 - t), d.y);
        return length(q) / t;
    }
    if (kind > 1.5) { return max(length(d.xz), abs(d.y)); }
    if (kind > 0.5) { return max(abs(d.x), max(abs(d.y), abs(d.z))); }
    return length(d);
}

/// Inside a collider?
fn solid(pc: vec3f) -> bool {
    for (var k = 0u; k < u32(prm.d.y); k++) {
        let c = prm.co[k];
        if (shape_dist((pc - c.pos.xyz) / max(c.size.xyz, vec3f(0.5)), c.pos.w, c.size.w) < 1.0) { return true; }
    }
    return false;
}

/// Smoothed fields around a cell (sca1 still holds last frame's scalars during emit_forces).
fn smooth_fields(pc: vec3f) -> vec4f {
    let r = 3.0;
    var acc = vec4f(0.0);
    for (var k = 0u; k < 7u; k++) {
        var o = vec3f(0.0);
        if (k == 1u) { o.x = r; } else if (k == 2u) { o.x = -r; } else if (k == 3u) { o.y = r; }
        else if (k == 4u) { o.y = -r; } else if (k == 5u) { o.z = r; } else if (k == 6u) { o.z = -r; }
        acc += s_sca1(pc + o);
    }
    return acc / 7.0;
}

fn in_range(x: f32, lo: f32, hi: f32, soft: f32) -> f32 {
    let w = max(soft, 1e-4);
    return smoothstep(lo - w, lo + w, x) * (1.0 - smoothstep(hi - w, hi + w, x));
}

/// Sum of all Force nodes at a cell (cells/frame per frame, before dt and scale).
fn forces_at(pc: vec3f, wp: vec3f, t: f32, vel: vec3f) -> vec3f {
    var total = vec3f(0.0);
    var have_smooth = false;
    var sm = vec4f(0.0);
    for (var k = 0u; k < u32(prm.d.x); k++) {
        let fo = prm.fo[k];
        if (fo.mext.z < 0.5) { continue; } // particles-only force
        let ty = fo.a.x;
        // field masks; temperature and smoke use smoothed fields so the mask edge is soft
        var w = fo.mk.x;
        if (fo.mk.y > 0.0) { w += fo.mk.y * clamp(length(vel) / 0.5, 0.0, 1.0); }
        if (fo.mk.z > 0.0 || fo.mk.w > 0.0) {
            if (!have_smooth) { sm = smooth_fields(pc); have_smooth = true; }
            w += fo.mk.z * smoothstep(0.0, 0.8, sm.x + sm.z * 4.0) * in_range(sm.x, fo.mr.x, fo.mr.y, 0.2);
            w += fo.mk.w * smoothstep(0.0, 0.2, sm.y) * in_range(sm.y, fo.mr.z, fo.mr.w, 0.05);
        }
        w = clamp(w, 0.0, 1.0);
        if (w <= 0.0) { continue; }
        if (fo.mpos.w > 0.5) {
            // soft-edged mask shape: forces fade out smoothly (sharp edges shred the gas)
            let md = shape_dist((pc - fo.mpos.xyz) / max(fo.msize.xyz, vec3f(0.5)), fo.msize.w, fo.mext.x);
            w *= 1.0 - smoothstep(1.0 - max(fo.mext.y, 0.02), 1.0, md);
            if (w <= 0.0) { continue; }
        }
        let rw = pc - fo.pos.xyz;
        let r = vec3f(dot(fo.r0.xyz, rw), dot(fo.r1.xyz, rw), dot(fo.r2.xyz, rw)); // local
        let rl = max(length(r), 1e-3);
        let rad = max(fo.pos.w, 1.0);
        var f = vec3f(0.0);
        if (ty < 0.5) {
            f = fo.dir.xyz * 0.6;
        } else if (ty < 1.5) {
            f = r / rl * exp(-(rl * rl) / (rad * rad)) * 2.0;
        } else if (ty < 2.5) {
            let ax = fo.dir.xyz;
            let h = dot(r, ax);
            let rp = r - h * ax;
            let rpl = max(length(rp), 1e-3);
            let radial = rp / rpl;
            let tube = max(fo.dir.w, 1.0);
            let dv = vec2f(rpl - rad, h);
            let dl = max(length(dv), 1e-3);
            f = (-dv.y * radial + dv.x * ax) / dl * exp(-(dl * dl) / (tube * tube)) * 2.0;
        } else if (ty < 3.5) {
            let lp = vec3f(dot(fo.r0.xyz, wp), dot(fo.r1.xyz, wp), dot(fo.r2.xyz, wp));
            f = (fbm_force(lp, fo, t) + fo.nb.xyz * 0.05) * 4.0;
        } else if (ty < 4.5) {
            let m = fo.dir.w;
            let ax = fo.dir.xyz;
            let fall = exp(-(rl * rl) / (rad * rad));
            if (m < 0.5) { f = r / rl * fall; }
            else if (m < 1.5) { f = -r / rl * fall; }
            else if (m < 2.5) { f = cross(ax, r) / rl * fall; }
            else { f = ax * fall; }
            f *= 2.0;
        } else {
            let sz = max(fo.a.z, 1.0);
            for (var j = 0; j < 8; j++) {
                let hj = vec3f(hash(vec3i(j, 7, 1)), hash(vec3i(j, 3, 9)), hash(vec3i(j, 5, 4)));
                var cpos = (hj - 0.5) * rad * 2.0;
                cpos.y = -rad + fract(hj.y + t * fo.a.w * 0.15) * rad * 2.0;
                let ax = normalize(vec3f(hash(vec3i(j, 1, 2)), hash(vec3i(j, 2, 3)), hash(vec3i(j, 3, 4))) - 0.5 + vec3f(1e-3));
                let rr = r - cpos;
                f += cross(ax, rr) / sz * exp(-dot(rr, rr) / (sz * sz)) * 2.0;
            }
        }
        // back to world axes
        let fw = vec3f(fo.r0.x, fo.r1.x, fo.r2.x) * f.x + vec3f(fo.r0.y, fo.r1.y, fo.r2.y) * f.y + vec3f(fo.r0.z, fo.r1.z, fo.r2.z) * f.z;
        total += fw * fo.a.y * w;
    }
    return total;
}

@compute @workgroup_size(4, 4, 4)
fn emit_forces(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let i = idx(vec3i(g));
    let pc = vec3f(g);
    if (solid(pc)) {
        vel0[i] = vec4f(0.0);
        sca0[i] = vec4f(0.0);
        return;
    }
    var v = vel0[i];
    var s = sca0[i];
    let dt = prm.a.x;
    let scale = prm.e.y;
    let t = prm.d.w;
    let wp = pc * prm.e.x; // metres
    for (var k = 0u; k < prm.n.w; k++) {
        let e = prm.em[k];
        let dw = pc - e.pos.xyz;
        let dl = vec3f(dot(e.r0.xyz, dw), dot(e.r1.xyz, dw), dot(e.r2.xyz, dw));
        let d = dl / max(e.size.xyz, vec3f(0.6));
        let dist = shape_dist(d, e.flags.x, e.size.w);
        if (dist >= 1.0) { continue; }
        let hollow = e.extra.x;
        if (hollow > 0.0 && dist < 1.0 - hollow) { continue; }
        var m = min(pow(1.0 - dist, max(e.rates.w, 0.01) / 15.0), 1.0);
        if (hollow > 0.0) { m = 1.0 - abs(dist - (1.0 - hollow * 0.5)) / (hollow * 0.5); }
        if (e.chaos.x > 0.0) {
            let nz = vnoise(pc * e.chaos.y + vec3f(0.0, -t * e.chaos.z * 3.0, t * e.chaos.z));
            m *= clamp(1.0 - e.chaos.x * (1.4 * nz - 0.2), 0.0, 1.0);
        }
        m *= e.extra.y;
        if (m <= 0.0) { continue; }
        if (e.flags.y > 0.5) { s.z += e.rates.x * m * dt; } else { s.z = max(s.z, e.rates.x * m * 20.0); }
        if (e.flags.z > 0.5) { s.y += e.rates.y * m * dt; } else { s.y = max(s.y, e.rates.y * m * 20.0); }
        if (e.flags.w > 0.5) { s.x = max(s.x, e.rates.z * m); } else { s.x += e.rates.z * m * 0.05 * dt; }
        let jitter = curl_noise(wp * 5.0 + vec3f(t * 3.0, -t * 4.0, t * 2.0)) * 0.15 * e.vel.w * scale;
        v = vec4f(v.xyz + (e.vel.xyz * m + jitter) * 0.1 * dt, v.w);
        // pressure: push gas outward, with optional noise on the push
        var push = e.chaos.w;
        if (push > 0.0 && e.extra.z > 0.0) {
            let pn = vnoise(pc * e.extra.w + seed_offset(e.extra2.x) + vec3f(t * e.extra2.y));
            push *= max(1.0 + e.extra.z * (pn * 2.0 - 1.0), 0.0);
        }
        if (push > 0.0) {
            v = vec4f(v.xyz + normalize(dw + vec3f(1e-4)) * push * 0.05 * m * scale, v.w);
        }
    }
    // buoyancy and weight
    v.y += (prm.a.y * 0.12 * s.x - prm.a.z * 0.05 * s.y - prm.a.w * 0.05 * s.z) * dt * scale;
    // force tightness: damp motion that no force keeps up
    v = vec4f(v.xyz * (1.0 - prm.e.w * dt), v.w);
    // wind: pull the air along the wind direction up to the wind speed (doesn't fight buoyancy)
    let goal = prm.w.xyz;
    let gl = length(goal);
    if (gl > 1e-5) {
        let gd = goal / gl;
        let along = dot(v.xyz, gd);
        v = vec4f(v.xyz + gd * max(gl - along, 0.0) * 0.03 * dt, v.w);
    }
    if (prm.w.w > 0.0) {
        v = vec4f(v.xyz + curl_noise(wp * 1.5 + vec3f(t * 0.7, 0.0, -t * 0.5)) * prm.w.w * 0.03 * dt, v.w);
    }
    // shredding: break up gas around a temperature band
    if (prm.sh.x > 0.0) {
        let band = exp(-pow((s.x - prm.sh.y) / max(prm.sh.z, 1e-3), 2.0));
        if (band > 0.01) {
            v = vec4f(v.xyz + curl_noise(wp * 18.0 + vec3f(0.0, -t * 3.0, t)) * prm.sh.x * band * 0.04 * dt * scale, v.w);
        }
    }
    if (prm.d.x > 0.5) {
        v = vec4f(v.xyz + forces_at(pc, wp, t, v.xyz) * 0.02 * dt * scale, v.w);
    }
    vel0[i] = v;
    sca0[i] = s;
}

// ---- Staggered (MAC) velocity: vel0[cell].x = u on the cell's -x face, .y = v on its -y face,
// .z = w on its -z face. Each side is open (gas leaves) or solid, per the Solid Wall settings.

fn face_u(c: vec3i) -> f32 {
    if (c.x <= 0 && wall(4u)) { return 0.0; }
    if (c.x >= dims().x && wall(8u)) { return 0.0; }
    return vel0[idx(c)].x; // idx clamps: an open side copies the edge face (outflow)
}
fn face_v(c: vec3i) -> f32 {
    if (c.y <= 0) {
        if (wall(1u)) { return 0.0; }
        return vel0[idx(c)].y;
    }
    if (c.y >= dims().y) {
        if (wall(2u)) { return 0.0; }
        return vel0[idx(c - vec3i(0, 1, 0))].y;
    }
    return vel0[idx(c)].y;
}
fn face_w(c: vec3i) -> f32 {
    if (c.z <= 0 && wall(16u)) { return 0.0; }
    if (c.z >= dims().z && wall(32u)) { return 0.0; }
    return vel0[idx(c)].z;
}

fn cvel(c: vec3i) -> vec3f {
    return 0.5 * vec3f(
        face_u(c) + face_u(c + vec3i(1, 0, 0)),
        face_v(c) + face_v(c + vec3i(0, 1, 0)),
        face_w(c) + face_w(c + vec3i(0, 0, 1)));
}

fn s_comp(q: vec3f, k: u32) -> f32 {
    let t = tri(q);
    var r = 0.0;
    for (var j = 0u; j < 8u; j++) { r += vel0[t.i[j]][k] * t.w[j]; }
    return r;
}
fn s_comp_cubic(p: vec3f, k: u32) -> f32 {
    let q = clamp(p, vec3f(0.0), vec3f(dims() - 1));
    let b = vec3i(floor(q));
    let f = q - floor(q);
    let wx = cr(f.x);
    let wy = cr(f.y);
    let wz = cr(f.z);
    var r = 0.0;
    var lo = 1e9;
    var hi = -1e9;
    for (var z = 0; z < 4; z++) {
        for (var y = 0; y < 4; y++) {
            for (var x = 0; x < 4; x++) {
                let v = vel0[idx(b + vec3i(x - 1, y - 1, z - 1))][k];
                r += v * (wx[x] * wy[y] * wz[z]);
                if (x >= 1 && x <= 2 && y >= 1 && y <= 2 && z >= 1 && z <= 2) { lo = min(lo, v); hi = max(hi, v); }
            }
        }
    }
    return clamp(r, lo, hi);
}

fn samp_vel(p: vec3f) -> vec3f {
    return vec3f(
        s_comp(p + vec3f(0.5, 0.0, 0.0), 0u),
        s_comp(p + vec3f(0.0, 0.5, 0.0), 1u),
        s_comp(p + vec3f(0.0, 0.0, 0.5), 2u)) + prm.ad.xyz;
}

/// Back-trace: where the gas at p came from dt ago, at the chosen accuracy
/// (1st order straight step, 2nd order midpoint, 3rd order Runge-Kutta).
fn trace(p: vec3f, dt: f32) -> vec3f {
    let order = prm.ad.w;
    let k1 = samp_vel(p);
    if (order < 0.5) { return p - dt * k1; }
    let k2 = samp_vel(p - 0.5 * dt * k1);
    if (order < 1.5) { return p - dt * k2; }
    let k3 = samp_vel(p - 0.75 * dt * k2);
    return p - dt * (2.0 * k1 + 3.0 * k2 + 4.0 * k3) / 9.0;
}

@compute @workgroup_size(4, 4, 4)
fn curl_pass(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let dx = vec3i(1, 0, 0);
    let dy = vec3i(0, 1, 0);
    let dz = vec3i(0, 0, 1);
    let w = 0.5 * vec3f(
        (cvel(c + dy).z - cvel(c - dy).z) - (cvel(c + dz).y - cvel(c - dz).y),
        (cvel(c + dz).x - cvel(c - dz).x) - (cvel(c + dx).z - cvel(c - dx).z),
        (cvel(c + dx).y - cvel(c - dx).y) - (cvel(c + dy).x - cvel(c - dy).x));
    tmp[idx(c)] = vec4f(w, length(w)); // tmp holds the curl until scalar advection
}

@compute @workgroup_size(4, 4, 4)
fn confine(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    // where vorticity applies: velocity, temperature, smoke and constant masks
    let s = sca0[i];
    let vm = prm.vm;
    let soft = max(prm.v1.z, 1e-3);
    var mask = vm.w;
    mask += vm.x * clamp(length(cvel(c)) / 0.5, 0.0, 1.0);
    mask += vm.y * in_range(s.x, prm.vr.x, prm.vr.y, soft);
    mask += vm.z * smoothstep(0.0, 0.05, s.y) * in_range(s.y, prm.vr.z, prm.vr.w, soft);
    mask = clamp(mask, 0.0, 1.0);
    var f = vec3f(0.0);
    let gr = 0.5 * vec3f(
        tmp[idx(c + vec3i(1, 0, 0))].w - tmp[idx(c - vec3i(1, 0, 0))].w,
        tmp[idx(c + vec3i(0, 1, 0))].w - tmp[idx(c - vec3i(0, 1, 0))].w,
        tmp[idx(c + vec3i(0, 0, 1))].w - tmp[idx(c - vec3i(0, 0, 1))].w);
    let l = length(gr);
    if (l > 1e-6) { f += cross(gr / l, tmp[i].xyz) * prm.v1.x; }
    if (prm.v1.y > 0.0) {
        // large-scale vorticity from the curl four cells away
        let r = 4;
        let wl = (tmp[idx(c + vec3i(r, 0, 0))].xyz + tmp[idx(c - vec3i(r, 0, 0))].xyz + tmp[idx(c + vec3i(0, r, 0))].xyz
            + tmp[idx(c - vec3i(0, r, 0))].xyz + tmp[idx(c + vec3i(0, 0, r))].xyz + tmp[idx(c - vec3i(0, 0, r))].xyz) / 6.0;
        let gl = vec3f(
            tmp[idx(c + vec3i(r, 0, 0))].w - tmp[idx(c - vec3i(r, 0, 0))].w,
            tmp[idx(c + vec3i(0, r, 0))].w - tmp[idx(c - vec3i(0, r, 0))].w,
            tmp[idx(c + vec3i(0, 0, r))].w - tmp[idx(c - vec3i(0, 0, r))].w) / (2.0 * f32(r));
        let ll = length(gl);
        if (ll > 1e-6) { f += cross(gl / ll, wl) * prm.v1.y * 2.0; }
    }
    vel0[i] = vec4f(vel0[i].xyz + f * mask * prm.a.x, 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn divergence(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let div = (face_u(c + vec3i(1, 0, 0)) - face_u(c))
        + (face_v(c + vec3i(0, 1, 0)) - face_v(c))
        + (face_w(c + vec3i(0, 0, 1)) - face_w(c));
    // burning gas expands (Combustion expansion)
    dvg[idx(c)] = div - prm.b2.y * sca0[idx(c)].w * 0.05;
}

fn p_at(c: vec3i, own: f32, b: bool) -> f32 {
    let n = dims();
    if (c.x < 0) { return select(0.0, own, wall(4u)); }
    if (c.x >= n.x) { return select(0.0, own, wall(8u)); }
    if (c.z < 0) { return select(0.0, own, wall(16u)); }
    if (c.z >= n.z) { return select(0.0, own, wall(32u)); }
    if (c.y < 0) { return select(0.0, own, wall(1u)); }
    if (c.y >= n.y) { return select(0.0, own, wall(2u)); }
    let i = idx(c);
    return select(p0[i], p1[i], b);
}

fn jacobi(g: vec3u, from_b: bool) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    let own = select(p0[i], p1[i], from_b);
    let sum = p_at(c + vec3i(1, 0, 0), own, from_b) + p_at(c - vec3i(1, 0, 0), own, from_b)
        + p_at(c + vec3i(0, 1, 0), own, from_b) + p_at(c - vec3i(0, 1, 0), own, from_b)
        + p_at(c + vec3i(0, 0, 1), own, from_b) + p_at(c - vec3i(0, 0, 1), own, from_b);
    let p = (sum - dvg[i]) / 6.0;
    if (from_b) { p0[i] = p; } else { p1[i] = p; }
}
// Red-black successive over-relaxation: updates in place, half the cells per pass.
fn sor(g: vec3u, parity: u32, w: f32) {
    if (!inside(g) || ((g.x + g.y + g.z) & 1u) != parity) { return; }
    let c = vec3i(g);
    let i = idx(c);
    let own = p0[i];
    let sum = p_at(c + vec3i(1, 0, 0), own, false) + p_at(c - vec3i(1, 0, 0), own, false)
        + p_at(c + vec3i(0, 1, 0), own, false) + p_at(c - vec3i(0, 1, 0), own, false)
        + p_at(c + vec3i(0, 0, 1), own, false) + p_at(c - vec3i(0, 0, 1), own, false);
    p0[i] = mix(own, (sum - dvg[i]) / 6.0, w);
}
@compute @workgroup_size(4, 4, 4)
fn sor_red(@builtin(global_invocation_id) g: vec3u) { sor(g, 0u, prm.pr.z); }
@compute @workgroup_size(4, 4, 4)
fn sor_black(@builtin(global_invocation_id) g: vec3u) { sor(g, 1u, prm.pr.z); }
@compute @workgroup_size(4, 4, 4)
fn gs_red(@builtin(global_invocation_id) g: vec3u) { sor(g, 0u, 1.0); }
@compute @workgroup_size(4, 4, 4)
fn gs_black(@builtin(global_invocation_id) g: vec3u) { sor(g, 1u, 1.0); }
@compute @workgroup_size(4, 4, 4)
fn jacobi_a(@builtin(global_invocation_id) g: vec3u) { jacobi(g, false); }
@compute @workgroup_size(4, 4, 4)
fn jacobi_b(@builtin(global_invocation_id) g: vec3u) { jacobi(g, true); }

// Hourglass filter: blends out the cell-to-cell zigzag a pressure solve can leave behind.
@compute @workgroup_size(4, 4, 4)
fn hourglass(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    let own = p0[i];
    let avg = (p_at(c + vec3i(1, 0, 0), own, false) + p_at(c - vec3i(1, 0, 0), own, false)
        + p_at(c + vec3i(0, 1, 0), own, false) + p_at(c - vec3i(0, 1, 0), own, false)
        + p_at(c + vec3i(0, 0, 1), own, false) + p_at(c - vec3i(0, 0, 1), own, false)) / 6.0;
    p1[i] = mix(own, avg, prm.pr.x * 0.5);
}

@compute @workgroup_size(4, 4, 4)
fn project(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    let p = p0[i];
    var v = vel0[i];
    v.x = v.x - (p - p_at(c - vec3i(1, 0, 0), p, false));
    v.y = v.y - (p - p_at(c - vec3i(0, 1, 0), p, false));
    v.z = v.z - (p - p_at(c - vec3i(0, 0, 1), p, false));
    if (c.x == 0 && wall(4u)) { v.x = 0.0; }
    if (c.y == 0 && wall(1u)) { v.y = 0.0; }
    if (c.z == 0 && wall(16u)) { v.z = 0.0; }
    if (solid(vec3f(g))) { v = vec4f(0.0); }
    if (prm.fl.z > 0.5) {
        // prevent leakage: no flow through faces shared with a collider
        if (solid(vec3f(g) - vec3f(1.0, 0.0, 0.0))) { v.x = 0.0; }
        if (solid(vec3f(g) - vec3f(0.0, 1.0, 0.0))) { v.y = 0.0; }
        if (solid(vec3f(g) - vec3f(0.0, 0.0, 1.0))) { v.z = 0.0; }
    }
    vel0[i] = v;
}

fn vel_comp(q: vec3f, k: u32) -> f32 {
    if (prm.fl.x > 0.5) { return s_comp_cubic(q, k); }
    return s_comp(q, k);
}

@compute @workgroup_size(4, 4, 4)
fn advect_vel(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3f(g);
    let dt = prm.a.x;
    let pu = c - vec3f(0.5, 0.0, 0.0);
    let pv = c - vec3f(0.0, 0.5, 0.0);
    let pw = c - vec3f(0.0, 0.0, 0.5);
    let tu = trace(pu, dt) + vec3f(0.5, 0.0, 0.0);
    let tv = trace(pv, dt) + vec3f(0.0, 0.5, 0.0);
    let tw = trace(pw, dt) + vec3f(0.0, 0.0, 0.5);
    var vel = vec3f(vel_comp(tu, 0u), vel_comp(tv, 1u), vel_comp(tw, 2u));
    let dv = prm.e.z;
    if (dv > 0.0) {
        // viscosity: blend with the neighbourhood
        let o = vec3f(1.0, 0.0, 0.0);
        let p = vec3f(0.0, 1.0, 0.0);
        let q = vec3f(0.0, 0.0, 1.0);
        let nu = (s_comp(tu + o, 0u) + s_comp(tu - o, 0u) + s_comp(tu + p, 0u) + s_comp(tu - p, 0u) + s_comp(tu + q, 0u) + s_comp(tu - q, 0u)) / 6.0;
        let nv = (s_comp(tv + o, 1u) + s_comp(tv - o, 1u) + s_comp(tv + p, 1u) + s_comp(tv - p, 1u) + s_comp(tv + q, 1u) + s_comp(tv - q, 1u)) / 6.0;
        let nw = (s_comp(tw + o, 2u) + s_comp(tw - o, 2u) + s_comp(tw + p, 2u) + s_comp(tw - p, 2u) + s_comp(tw + q, 2u) + s_comp(tw - q, 2u)) / 6.0;
        vel = mix(vel, vec3f(nu, nv, nw), dv);
    }
    vel *= 1.0 - prm.c.y;
    if (g.y == 0u && wall(1u)) { vel.y = 0.0; }
    sca1[idx(vec3i(g))] = vec4f(vel, 0.0); // copied back into vel0 afterwards
}

@compute @workgroup_size(4, 4, 4)
fn advect_fwd(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let back = trace(vec3f(g), prm.a.x);
    var v = adv_sca0(back);
    // diffusion per channel (temperature, smoke, fuel, flames)
    let dif = vec4f(prm.df.x, prm.df.y, prm.df.z, prm.df.w);
    if (any(dif > vec4f(0.0))) {
        let nb = (s_sca0(back + vec3f(1.0, 0.0, 0.0)) + s_sca0(back - vec3f(1.0, 0.0, 0.0))
            + s_sca0(back + vec3f(0.0, 1.0, 0.0)) + s_sca0(back - vec3f(0.0, 1.0, 0.0))
            + s_sca0(back + vec3f(0.0, 0.0, 1.0)) + s_sca0(back - vec3f(0.0, 0.0, 1.0))) / 6.0;
        // stored order is (temperature, smoke, fuel, flames)
        v = mix(v, nb, vec4f(dif.x, dif.y, dif.z, dif.w));
    }
    sca1[idx(c)] = v;
}

@compute @workgroup_size(4, 4, 4)
fn advect_back(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    tmp[idx(c)] = s_sca1(trace(vec3f(g), -prm.a.x));
}

fn near_solid(pc: vec3f) -> bool {
    return solid(pc + vec3f(1.0, 0.0, 0.0)) || solid(pc - vec3f(1.0, 0.0, 0.0)) || solid(pc + vec3f(0.0, 1.0, 0.0))
        || solid(pc - vec3f(0.0, 1.0, 0.0)) || solid(pc + vec3f(0.0, 0.0, 1.0)) || solid(pc - vec3f(0.0, 0.0, 1.0));
}

// MacCormack correction (sharper detail) + combustion + dissipation, result in sca1.
@compute @workgroup_size(4, 4, 4)
fn correct(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let i = idx(vec3i(g));
    let dt = prm.a.x;
    var s = sca1[i];
    if (prm.ad.w > 0.5) {
        let back = trace(vec3f(g), dt);
        let t = tri(back);
        var lo = vec4f(1e9);
        var hi = vec4f(-1e9);
        for (var k = 0u; k < 8u; k++) {
            lo = min(lo, sca0[t.i[k]]);
            hi = max(hi, sca0[t.i[k]]);
        }
        // where the sharpened value would overshoot its neighbours, keep the smooth result
        let sharp = s + 0.25 * (sca0[i] - tmp[i]);
        let bad = (sharp < lo) | (sharp > hi);
        s = select(sharp, s, bad);
    }
    // combustion: fuel above the ignition temperature burns into heat, smoke and flames
    let mode = prm.d.z;
    if (mode < 0.5 || mode > 1.5) {
        let ignite = smoothstep(prm.b.y, prm.b.y + prm.b2.z, s.x);
        if (ignite > 0.0 && s.z > 0.0) {
            let burned = s.z * min(prm.b.x * 0.15 * dt * ignite, 1.0);
            s.z -= burned;
            s.x = min(s.x + burned * prm.b.z * 10.0, max(prm.b2.w, s.x));
            if (mode < 0.5) { s.y += burned * prm.b.w; }
            s.w += burned * prm.b2.x * 8.0;
        }
    }
    // dissipation: multiplicative, then subtractive; flames also cool with the gas
    s.x -= s.x * min(prm.c.x * dt, 1.0);
    s.y *= max(1.0 - prm.c.z * dt, 0.0);
    s.w *= max(1.0 - (prm.c.w + prm.c.x * 1.2) * dt, 0.0);
    s.z *= max(1.0 - prm.c2.x * dt, 0.0) * 0.995;
    s.y = max(s.y - prm.c2.y * dt, 0.0);
    s.w = max(s.w - prm.c2.z * dt, 0.0);
    s.z = max(s.z - prm.c2.w * dt, 0.0);
    // colliders remove gas that touches them
    if (prm.d.y > 0.5 && prm.pr.y > 0.0 && near_solid(vec3f(g))) {
        s *= 1.0 - prm.pr.y * 0.25;
    }
    // fade out softly near open sides, so nothing ends in a hard edge
    let n = dims();
    let gi = vec3i(g);
    var edge = 1e9;
    if (!wall(4u)) { edge = min(edge, f32(gi.x)); }
    if (!wall(8u)) { edge = min(edge, f32(n.x - 1 - gi.x)); }
    if (!wall(16u)) { edge = min(edge, f32(gi.z)); }
    if (!wall(32u)) { edge = min(edge, f32(n.z - 1 - gi.z)); }
    if (!wall(2u)) { edge = min(edge, f32(n.y - 1 - gi.y)); }
    if (!wall(1u)) { edge = min(edge, f32(gi.y)); }
    let fade = smoothstep(0.0, 6.0, edge);
    s = vec4f(s.xyz * mix(0.8, 1.0, fade), s.w * mix(0.8, 1.0, fade));
    // domain mask: fade everything toward the edge of a sphere or box inside the domain
    if (prm.mk.x > 0.5 && prm.mk.y > 0.0) {
        let u = (vec3f(g) + 0.5) / vec3f(n) * 2.0 - 1.0;
        var md = length(u);
        if (prm.mk.x > 1.5) { md = max(abs(u.x), max(abs(u.y), abs(u.z))); }
        let m = smoothstep(1.0 - prm.mk.z, 1.0, md);
        s *= 1.0 - m * prm.mk.y * 0.2;
    }
    sca1[i] = max(s, vec4f(0.0));
}

// t: temperature in kK above ambient (same units as the remap range)
fn emission_weight(t: f32) -> f32 {
    let k = clamp((t - prm.l3.x) / max(prm.l3.y - prm.l3.x, 1e-3), 0.0, 1.0);
    return pow(pow(k, prm.l3.z), 3.0);
}

fn in_caster(p: vec3f) -> bool {
    for (var k = 0u; k < prm.n.w; k++) {
        let e = prm.em[k];
        if (e.pos.w < 0.5) { continue; }
        let dw = p - e.pos.xyz;
        let dl = vec3f(dot(e.r0.xyz, dw), dot(e.r1.xyz, dw), dot(e.r2.xyz, dw));
        if (shape_dist(dl / max(e.size.xyz, vec3f(0.6)), e.flags.x, e.size.w) < 1.0) { return true; }
    }
    return false;
}

// Light per voxel: sun transmittance, ambient occlusion, and glow from nearby flames.
@compute @workgroup_size(4, 4, 4)
fn light_pass(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let i = idx(vec3i(g));
    let here = sca0[i];
    // light only matters where there is smoke (or fuel) to light
    if (here.y < 1e-4 && here.z < 1e-4) { light[i] = vec4f(1.0, 1.0, 0.0, 0.0); return; }
    let n = vec3f(dims() - 1);
    // sun
    var sun_t = 1.0;
    if (prm.l.w > 0.0) {
        var p = vec3f(g);
        let stepc = prm.l2.x;
        var acc = 0.0;
        for (var k = 0; k < i32(prm.l2.y); k++) {
            p += prm.l.xyz * stepc;
            if (any(p < vec3f(0.0)) || any(p > n)) { break; }
            if (in_caster(p)) { acc += 50.0; break; }
            acc += s_sca0(p).y * (stepc / 1.5);
        }
        sun_t = exp(-acc * prm.l.w);
    }
    // ambient occlusion: smoke above and around
    var ao = 1.0;
    if (prm.l2.z > 0.0) {
        var acc = 0.0;
        for (var k = 1; k <= 6; k++) {
            let o = f32(k) * 2.0;
            acc += s_sca0(vec3f(g) + vec3f(0.0, o, 0.0)).y + 0.5 * (s_sca0(vec3f(g) + vec3f(o, 0.0, 0.0)).y + s_sca0(vec3f(g) - vec3f(o, 0.0, 0.0)).y
                + s_sca0(vec3f(g) + vec3f(0.0, 0.0, o)).y + s_sca0(vec3f(g) - vec3f(0.0, 0.0, o)).y) * 0.5;
        }
        ao = exp(-acc * 0.6);
    }
    // glow: emission of flames within the scattering radius, dimmed by smoke in between
    var glow = 0.0;
    var gk = 0.0;
    let rad = prm.l2.w;
    if (rad > 0.5) {
        for (var d = 0; d < 6; d++) {
            var dir = vec3f(0.0);
            if (d == 0) { dir.x = 1.0; } else if (d == 1) { dir.x = -1.0; } else if (d == 2) { dir.y = 1.0; }
            else if (d == 3) { dir.y = -1.0; } else if (d == 4) { dir.z = 1.0; } else { dir.z = -1.0; }
            var occl = 0.0;
            for (var k = 1; k <= 4; k++) {
                let dist = rad * f32(k) / 4.0;
                let q = s_sca0(vec3f(g) + dir * dist);
                let w = emission_weight(q.x) * exp(-f32(k) * 0.5) * exp(-occl * prm.l3.w);
                glow += w;
                gk += w * q.x;
                occl += q.y * dist / 4.0;
            }
        }
        gk = gk / max(glow, 1e-5);
        glow /= 6.0;
    }
    light[i] = vec4f(sun_t, ao, glow, gk);
}

// Copy the fields into the 3D textures the renderer samples, with sharpening and post-processing.
@compute @workgroup_size(4, 4, 4)
fn pack(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    var s = sca0[i];
    let lt = light[i];
    let mode = prm.pp.x;
    if (prm.pk.w > 0.5 || mode > 0.5) {
        var nb = vec4f(0.0);
        var mx = s;
        for (var k = 0; k < 6; k++) {
            var o = vec3i(0);
            if (k == 0) { o.x = 1; } else if (k == 1) { o.x = -1; } else if (k == 2) { o.y = 1; }
            else if (k == 3) { o.y = -1; } else if (k == 4) { o.z = 1; } else { o.z = -1; }
            let q = sca0[idx(c + o)];
            nb += q;
            mx = max(mx, q);
        }
        nb /= 6.0;
        if (prm.pk.w > 0.5) {
            // raymarch sharpening: unsharp mask per channel (temperature, smoke, flames)
            let a = vec3f(prm.pk.z, prm.pk.x, prm.pk.y);
            let sh = clamp(s.xyw + (s.xyw - nb.xyw) * a, vec3f(0.0), s.xyw * 2.0 + 0.05);
            s = vec4f(sh.x, sh.y, s.z, sh.z);
        }
        if (mode > 0.5 && mode < 1.5) {
            // motion blur: smear along the velocity
            let v = cvel(c);
            var acc = s;
            for (var k = 1; k <= 3; k++) { acc += s_sca0(vec3f(g) - v * f32(k) * 1.5 * prm.pp.y); }
            s = mix(s, acc / 4.0, prm.pp.y);
        } else if (mode > 1.5 && mode < 2.5) {
            s = max(s + (s - nb) * prm.pp.y * 2.0, vec4f(0.0));
        } else if (mode > 2.5) {
            s = mix(s, mx, prm.pp.y);
        }
    }
    // post modulation: noise on smoke or flames
    if (prm.pp.z > -0.5) {
        let q = vec3f(g) * prm.pp.w + vec3f(0.0, -prm.d.w * prm.pm.y, 0.0);
        let nz = vnoise(q) * 0.6 + vnoise(q * 2.3) * 0.4;
        let m = 1.0 - prm.pm.x * (1.0 - nz * 1.4);
        if (prm.pp.z < 0.5) { s.y *= max(m, 0.0); } else { s.w *= max(m, 0.0); }
    }
    textureStore(vol, c, vec4f(s.x, s.y, lt.x, s.w));
    textureStore(vol2, c, vec4f(s.z, lt.y, lt.z, lt.w));
    if (s.x > 0.02 || s.y > 0.002 || s.z > 0.002) {
        let b = g / 8u;
        let nbk = (prm.n.xyz + 7u) / 8u;
        atomicStore(&occ[b.x + nbk.x * (b.y + nbk.y * b.z)], 1u);
    }
}

// Histograms for the Shading node (temperature 0..10000 K, smoke 0..2), sampled every 2nd voxel.
@compute @workgroup_size(4, 4, 4)
fn histogram(@builtin(global_invocation_id) g: vec3u) {
    let c = g * 2u;
    if (!all(c < prm.n.xyz)) { return; }
    let s = sca0[idx(vec3i(c))];
    if (s.x > 0.01) {
        let tb = u32(clamp((s.x * 1000.0 + 300.0) / 10000.0 * 64.0, 0.0, 63.0));
        atomicAdd(&hist[tb], 1u);
    }
    if (s.y > 0.001) {
        let sb = u32(clamp(s.y / 2.0 * 64.0, 0.0, 63.0));
        atomicAdd(&hist[64u + sb], 1u);
    }
}
