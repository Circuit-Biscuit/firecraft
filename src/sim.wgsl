// GPU fluid solver. Grid idx = x + nx*(y + ny*z), y up. Cell centres at integer coords.
// vel: (u, v, w, _) in cells/frame. sca: (temperature kK above ambient, smoke, fuel, flame).

struct Emitter {
    pos: vec4f,   // centre (cells), _
    size: vec4f,  // half size per axis (cells), torus thickness
    rates: vec4f, // fuel/frame, smoke/frame, temperature, gradient
    vel: vec4f,   // velocity (cells/frame), spread
    flags: vec4f, // shape, fuel add, smoke add, temperature replace
    chaos: vec4f, // strength, scale, speed, expansion
    extra: vec4f, // hollow, fade, _, _
}

struct Force {
    a: vec4f,     // type, strength, scale (cells or noise scale), speed
    pos: vec4f,   // position (cells), radius (cells)
    dir: vec4f,   // direction / axis, extra (tube radius cells or field mode)
    mpos: vec4f,  // mask centre (cells), mask on
    msize: vec4f, // mask half size (cells), mask shape
    mext: vec4f,  // mask torus thickness, softness, _, _
}

struct Coll {
    pos: vec4f,   // centre (cells), shape
    size: vec4f,  // half size (cells), torus thickness
}

struct Params {
    n: vec4u,  // nx, ny, nz, emitter count
    a: vec4f,  // dt, buoyancy, smoke weight, vorticity
    b: vec4f,  // burn rate, ignition, heat, smoke yield
    c: vec4f,  // cooling, smoke fade, wind x, wind z
    d: vec4f,  // force count, collider count, _, time (s)
    e: vec4f,  // cell size (m), res scale, _, _
    l: vec4f,  // light dir (xyz), shadow k
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
@group(0) @binding(8) var<storage, read_write> shadow: array<f32>;
@group(0) @binding(9) var vol: texture_storage_3d<rgba16float, write>;
@group(0) @binding(10) var<storage, read_write> occ: array<atomic<u32>>; // 8^3 bricks with content

fn dims() -> vec3i { return vec3i(prm.n.xyz); }
fn idx(c: vec3i) -> u32 {
    let n = dims();
    let q = clamp(c, vec3i(0), n - 1);
    return u32(q.x + n.x * (q.y + n.y * q.z));
}
fn inside(g: vec3u) -> bool { return all(g < prm.n.xyz); }

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

/// Heat around a cell, smoothed over a few cells (sca1 still holds last frame's scalars here).
fn heat_mask(pc: vec3f) -> f32 {
    let r = 3.0;
    var heat = 0.0;
    for (var k = 0u; k < 7u; k++) {
        var o = vec3f(0.0);
        if (k == 1u) { o.x = r; } else if (k == 2u) { o.x = -r; } else if (k == 3u) { o.y = r; }
        else if (k == 4u) { o.y = -r; } else if (k == 5u) { o.z = r; } else if (k == 6u) { o.z = -r; }
        let q = s_sca1(pc + o);
        heat += q.x + q.z * 4.0 + q.y;
    }
    return smoothstep(0.0, 0.8, heat / 7.0);
}

/// Sum of all Force nodes at a cell (cells/frame per frame, before dt and scale).
fn forces_at(pc: vec3f, wp: vec3f, t: f32) -> vec3f {
    var total = vec3f(0.0);
    var hm = -1.0;
    for (var k = 0u; k < u32(prm.d.x); k++) {
        let fo = prm.fo[k];
        let ty = fo.a.x;
        var w = 1.0;
        if (fo.mpos.w > 0.5) {
            // soft-edged mask shape: forces fade out smoothly (sharp edges shred the gas)
            let md = shape_dist((pc - fo.mpos.xyz) / max(fo.msize.xyz, vec3f(0.5)), fo.msize.w, fo.mext.x);
            w = 1.0 - smoothstep(1.0 - max(fo.mext.y, 0.02), 1.0, md);
            if (w <= 0.0) { continue; }
        }
        let r = pc - fo.pos.xyz;
        let rl = max(length(r), 1e-3);
        let rad = max(fo.pos.w, 1.0);
        var f = vec3f(0.0);
        if (ty < 0.5) {
            // line: constant push along a direction
            f = fo.dir.xyz * 0.6;
        } else if (ty < 1.5) {
            // point: push out (positive) or pull in (negative), fading with distance
            f = r / rl * exp(-(rl * rl) / (rad * rad)) * 2.0;
        } else if (ty < 2.5) {
            // toroidal: gas circulates around a ring (smoke rings, shockwaves)
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
            // noise: swirling curl noise, kept near the effect unless a mask shape is given
            let q = wp * fo.a.z * 4.0 + vec3f(0.0, -t * fo.a.w * 2.0, t * fo.a.w * 0.7);
            f = curl_noise(q) * 0.08 / 0.02;
            if (fo.mpos.w < 0.5) {
                if (hm < 0.0) { hm = heat_mask(pc); }
                w *= hm;
            }
        } else if (ty < 4.5) {
            // vector field: push out, pull in, swirl around an axis, or updraft
            let m = fo.dir.w;
            let ax = fo.dir.xyz;
            let fall = exp(-(rl * rl) / (rad * rad));
            if (m < 0.5) { f = r / rl * fall; }
            else if (m < 1.5) { f = -r / rl * fall; }
            else if (m < 2.5) { f = cross(ax, r) / rl * fall; }
            else { f = ax * fall; }
            f *= 2.0;
        } else {
            // vorticles: a handful of drifting little vortices
            let sz = max(fo.a.z, 1.0);
            for (var j = 0; j < 8; j++) {
                let hj = vec3f(hash(vec3i(j, 7, 1)), hash(vec3i(j, 3, 9)), hash(vec3i(j, 5, 4)));
                var cpos = fo.pos.xyz + (hj - 0.5) * rad * 2.0;
                cpos.y = fo.pos.y - rad + fract(hj.y + t * fo.a.w * 0.15) * rad * 2.0;
                let ax = normalize(vec3f(hash(vec3i(j, 1, 2)), hash(vec3i(j, 2, 3)), hash(vec3i(j, 3, 4))) - 0.5 + vec3f(1e-3));
                let rr = pc - cpos;
                f += cross(ax, rr) / sz * exp(-dot(rr, rr) / (sz * sz)) * 2.0;
            }
        }
        total += f * fo.a.y * w;
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
        let d = (pc - e.pos.xyz) / max(e.size.xyz, vec3f(0.6));
        let dist = shape_dist(d, e.flags.x, e.size.w);
        if (dist >= 1.0) { continue; }
        let hollow = e.extra.x;
        if (hollow > 0.0 && dist < 1.0 - hollow) { continue; }
        var m = min(pow(1.0 - dist, max(e.rates.w, 0.01) / 15.0), 1.0);
        if (hollow > 0.0) {
            // shell: strongest in the middle of the wall
            m = 1.0 - abs(dist - (1.0 - hollow * 0.5)) / (hollow * 0.5);
        }
        if (e.chaos.x > 0.0) {
            let nz = vnoise(wp * e.chaos.y * 6.0 + vec3f(0.0, -t * e.chaos.z * 3.0, t * e.chaos.z));
            m *= clamp(1.0 - e.chaos.x * (1.4 * nz - 0.2), 0.0, 1.0);
        }
        m *= e.extra.y; // fade (particles near the end of their life)
        if (m <= 0.0) { continue; }
        if (e.flags.y > 0.5) { s.z += e.rates.x * m * dt; } else { s.z = max(s.z, e.rates.x * m * 20.0); }
        if (e.flags.z > 0.5) { s.y += e.rates.y * m * dt; } else { s.y = max(s.y, e.rates.y * m * 20.0); }
        if (e.flags.w > 0.5) { s.x = max(s.x, e.rates.z * m); } else { s.x += e.rates.z * m * 0.05 * dt; }
        let jitter = curl_noise(wp * 5.0 + vec3f(t * 3.0, -t * 4.0, t * 2.0)) * 0.15 * e.vel.w * scale;
        v = vec4f(v.xyz + (e.vel.xyz * m + jitter) * 0.1 * dt, v.w);
        if (e.chaos.w > 0.0) {
            v = vec4f(v.xyz + normalize(d + vec3f(1e-4)) * e.chaos.w * 0.05 * m * scale, v.w);
        }
    }
    // buoyancy and wind
    v.y += (prm.a.y * 0.12 * s.x - prm.a.z * 0.05 * s.y) * dt * scale;
    // wind: pull the air toward the wind speed (m/s to cells/frame), not accelerate forever
    if (prm.c.z != 0.0 || prm.c.w != 0.0) {
        let goal = vec2f(prm.c.z, prm.c.w) * scale * 0.53;
        v.x += (goal.x - v.x) * 0.03 * dt;
        v.z += (goal.y - v.z) * 0.03 * dt;
    }
    // Force nodes
    if (prm.d.x > 0.5) {
        v = vec4f(v.xyz + forces_at(pc, wp, t) * 0.02 * dt * scale, v.w);
    }
    vel0[i] = v;
    sca0[i] = s;
}

// ---- Staggered (MAC) velocity: vel0[cell].x = u on the cell's -x face, .y = v on its -y face,
// .z = w on its -z face. Projection on this layout has no odd/even checkerboard noise.

// Sides and top are open (gas can leave); only the floor is solid. Closed sides made smoke
// pile up into a hard-edged box at the edge of the simulated space.
fn face_u(c: vec3i) -> f32 {
    return vel0[idx(c)].x; // idx clamps: outside faces copy the edge face (outflow)
}
fn face_v(c: vec3i) -> f32 {
    if (c.y <= 0) { return 0.0; }                     // floor
    if (c.y >= dims().y) { return vel0[idx(c - vec3i(0, 1, 0))].y; } // open top: outflow
    return vel0[idx(c)].y;
}
fn face_w(c: vec3i) -> f32 {
    return vel0[idx(c)].z;
}

/// Velocity at a cell centre (average of its two faces per axis).
fn cvel(c: vec3i) -> vec3f {
    return 0.5 * vec3f(
        face_u(c) + face_u(c + vec3i(1, 0, 0)),
        face_v(c) + face_v(c + vec3i(0, 1, 0)),
        face_w(c) + face_w(c + vec3i(0, 0, 1)));
}

/// Trilinear sample of one stored component k at index-space position q.
fn s_comp(q: vec3f, k: u32) -> f32 {
    let t = tri(q);
    var r = 0.0;
    for (var j = 0u; j < 8u; j++) { r += vel0[t.i[j]][k] * t.w[j]; }
    return r;
}

/// Velocity anywhere (p in cell-centre coordinates).
fn samp_vel(p: vec3f) -> vec3f {
    return vec3f(
        s_comp(p + vec3f(0.5, 0.0, 0.0), 0u),
        s_comp(p + vec3f(0.0, 0.5, 0.0), 1u),
        s_comp(p + vec3f(0.0, 0.0, 0.5), 2u));
}

/// Second-order (midpoint) back-trace: where the gas at p came from dt ago. A single straight
/// step is exact only in straight flow; in swirling flow it tears sharp edges into fingers.
fn trace(p: vec3f, dt: f32) -> vec3f {
    let mid = p - 0.5 * dt * samp_vel(p);
    return p - dt * samp_vel(mid);
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
    let gr = 0.5 * vec3f(
        tmp[idx(c + vec3i(1, 0, 0))].w - tmp[idx(c - vec3i(1, 0, 0))].w,
        tmp[idx(c + vec3i(0, 1, 0))].w - tmp[idx(c - vec3i(0, 1, 0))].w,
        tmp[idx(c + vec3i(0, 0, 1))].w - tmp[idx(c - vec3i(0, 0, 1))].w);
    let l = length(gr);
    if (l < 1e-6) { return; }
    let f = cross(gr / l, tmp[i].xyz) * prm.a.w * prm.a.x;
    vel0[i] = vec4f(vel0[i].xyz + f, 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn divergence(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let div = (face_u(c + vec3i(1, 0, 0)) - face_u(c))
        + (face_v(c + vec3i(0, 1, 0)) - face_v(c))
        + (face_w(c + vec3i(0, 0, 1)) - face_w(c));
    dvg[idx(c)] = div; // pressure keeps last frame's answer as the starting guess
}

fn p_at(c: vec3i, own: f32, b: bool) -> f32 {
    let n = dims();
    if (c.y >= n.y || c.x < 0 || c.z < 0 || c.x >= n.x || c.z >= n.z) { return 0.0; } // open sides and top
    if (c.y < 0) { return own; } // solid floor
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
// Red-black successive over-relaxation: updates in place, half the cells per pass, and
// converges many times faster than Jacobi (an accurate pressure keeps the gas smooth).
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
fn sor_red(@builtin(global_invocation_id) g: vec3u) { sor(g, 0u, 1.8); }
@compute @workgroup_size(4, 4, 4)
fn sor_black(@builtin(global_invocation_id) g: vec3u) { sor(g, 1u, 1.8); }
// plain Gauss-Seidel: removes the cell-sized pressure errors that show up as speckle
@compute @workgroup_size(4, 4, 4)
fn gs_red(@builtin(global_invocation_id) g: vec3u) { sor(g, 0u, 1.0); }
@compute @workgroup_size(4, 4, 4)
fn gs_black(@builtin(global_invocation_id) g: vec3u) { sor(g, 1u, 1.0); }

@compute @workgroup_size(4, 4, 4)
fn jacobi_a(@builtin(global_invocation_id) g: vec3u) { jacobi(g, false); }
@compute @workgroup_size(4, 4, 4)
fn jacobi_b(@builtin(global_invocation_id) g: vec3u) { jacobi(g, true); }

@compute @workgroup_size(4, 4, 4)
fn project(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let i = idx(c);
    let p = p0[i];
    var v = vel0[i];
    v.x = v.x - (p - p_at(c - vec3i(1, 0, 0), p, false));
    v.y = select(v.y - (p - p0[idx(c - vec3i(0, 1, 0))]), 0.0, c.y == 0);
    v.z = v.z - (p - p_at(c - vec3i(0, 0, 1), p, false));
    if (solid(vec3f(g))) { v = vec4f(0.0); }
    vel0[i] = v;
}

@compute @workgroup_size(4, 4, 4)
fn advect_vel(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3f(g);
    let dt = prm.a.x;
    let pu = c - vec3f(0.5, 0.0, 0.0);
    let pv = c - vec3f(0.0, 0.5, 0.0);
    let pw = c - vec3f(0.0, 0.0, 0.5);
    var u = s_comp(trace(pu, dt) + vec3f(0.5, 0.0, 0.0), 0u);
    var v = s_comp(trace(pv, dt) + vec3f(0.0, 0.5, 0.0), 1u);
    var w = s_comp(trace(pw, dt) + vec3f(0.0, 0.0, 0.5), 2u);
    if (g.y == 0u) { v = 0.0; }
    sca1[idx(vec3i(g))] = vec4f(u, v, w, 0.0); // copied back into vel0 afterwards
}

@compute @workgroup_size(4, 4, 4)
fn advect_fwd(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    let back = trace(vec3f(g), prm.a.x);
    var v = s_sca0(back);
    // a little diffusion of heat and fuel: real flames have it, and it stops the burning
    // front from breaking into cell-sized fingers (the grain)
    let d = 0.0; // ponytail: diffusion off; the midpoint trace fixed the fingers it was meant to damp
    if (d > 0.0) {
        let nb = s_sca0(back + vec3f(1.0, 0.0, 0.0)) + s_sca0(back - vec3f(1.0, 0.0, 0.0))
            + s_sca0(back + vec3f(0.0, 1.0, 0.0)) + s_sca0(back - vec3f(0.0, 1.0, 0.0))
            + s_sca0(back + vec3f(0.0, 0.0, 1.0)) + s_sca0(back - vec3f(0.0, 0.0, 1.0));
        let blurred = mix(v, nb / 6.0, d);
        v = vec4f(blurred.x, v.y, blurred.z, v.w); // smoke keeps its crisp detail
    }
    sca1[idx(c)] = v;
}

@compute @workgroup_size(4, 4, 4)
fn advect_back(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let c = vec3i(g);
    tmp[idx(c)] = s_sca1(trace(vec3f(g), -prm.a.x));
}

// MacCormack correction (sharper detail) + combustion + dissipation, result in sca1.
@compute @workgroup_size(4, 4, 4)
fn correct(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let i = idx(vec3i(g));
    let dt = prm.a.x;
    let back = trace(vec3f(g), dt);
    let t = tri(back);
    var lo = vec4f(1e9);
    var hi = vec4f(-1e9);
    for (var k = 0u; k < 8u; k++) {
        lo = min(lo, sca0[t.i[k]]);
        hi = max(hi, sca0[t.i[k]]);
    }
    // MacCormack with revert: where the sharpened value would overshoot its neighbours, use the
    // plain (smooth) result instead of clamping, which is what leaves salt-and-pepper speckle.
    let hat = sca1[i];
    let sharp = hat + 0.25 * (sca0[i] - tmp[i]);
    let bad = (sharp < lo) | (sharp > hi);
    var s = select(sharp, hat, bad);
    s.w = 0.0;
    // smooth ignition (a hard on/off threshold makes per-voxel sparkle)
    let ignite = smoothstep(prm.b.y, prm.b.y + 0.4, s.x);
    if (ignite > 0.0 && s.z > 0.0) {
        let burned = s.z * min(prm.b.x * 0.15 * dt * ignite, 1.0);
        s.z -= burned;
        s.x += burned * prm.b.z * 10.0;
        s.y += burned * prm.b.w;
        s.w = burned;
    }
    s.x -= s.x * min(prm.c.x * 0.1 * dt, 1.0);
    s.y *= max(1.0 - prm.c.y * 0.01 * dt, 0.0);
    s.z *= 0.995;
    // fade out softly near the open sides and top, so nothing ends in a hard edge
    let n = dims();
    let gi = vec3i(g);
    let edge = f32(min(min(min(gi.x, n.x - 1 - gi.x), min(gi.z, n.z - 1 - gi.z)), n.y - 1 - gi.y));
    let fade = smoothstep(0.0, 6.0, edge);
    s = vec4f(s.xyz * mix(0.8, 1.0, fade), s.w);
    sca1[i] = max(s, vec4f(0.0));
}

// Light transmittance towards the sun.
@compute @workgroup_size(4, 4, 4)
fn shadow_pass(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    // light only matters where there is smoke to light (keeps this pass cheap on big grids)
    if (sca0[idx(vec3i(g))].y < 1e-4) { shadow[idx(vec3i(g))] = 1.0; return; }
    var p = vec3f(g);
    let n = vec3f(dims() - 1);
    let stepc = max(1.5, f32(dims().x) / 40.0); // reach across the grid at any resolution
    var acc = 0.0;
    for (var k = 0; k < 64; k++) {
        p += prm.l.xyz * stepc;
        if (any(p < vec3f(0.0)) || any(p > n)) { break; }
        acc += s_sca0(p).y * (stepc / 1.5);
    }
    shadow[idx(vec3i(g))] = exp(-acc * prm.l.w);
}

// Copy (temperature, smoke, light, fuel) into the 3D texture the renderer samples.
@compute @workgroup_size(4, 4, 4)
fn pack(@builtin(global_invocation_id) g: vec3u) {
    if (!inside(g)) { return; }
    let i = idx(vec3i(g));
    let s = sca0[i];
    textureStore(vol, vec3i(g), vec4f(s.x, s.y, shadow[i], s.z));
    if (s.x > 0.02 || s.y > 0.002) {
        let b = g / 8u;
        let nb = (prm.n.xyz + 7u) / 8u;
        atomicStore(&occ[b.x + nb.x * (b.y + nb.y * b.z)], 1u);
    }
}

// ---- Histograms for the Shading node (added after 0.4; reads the grid, changes nothing) ----
@group(0) @binding(11) var<storage, read_write> hist: array<atomic<u32>>;

@compute @workgroup_size(4, 4, 4)
fn histogram(@builtin(global_invocation_id) g: vec3u) {
    let c = vec3i(g * 2u);
    if (any(c >= dims())) { return; }
    let s = sca0[idx(c)];
    if (s.x > 0.01) {
        atomicAdd(&hist[u32(clamp((s.x * 1000.0 + 300.0) / 10000.0 * 64.0, 0.0, 63.0))], 1u);
    }
    if (s.y > 0.001) {
        atomicAdd(&hist[64u + u32(clamp(s.y / 2.0 * 64.0, 0.0, 63.0))], 1u);
    }
}
