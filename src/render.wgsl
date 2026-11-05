// GPU raymarcher: emission/absorption volume with sun shadows, written to a storage texture.
//
// This is the 0.4.0 raymarcher. Everything added since (the Shading, Volume Processing and Render
// settings) sits behind a feature bit in `r.e.x` or a pass/alpha code in `r.img.w`; with all of
// them off, only 0.4's own lines run, so the default picture stays exactly 0.4's.

struct R {
    n: vec4u,       // nx, ny, nz, frame
    img: vec4u,     // width, height, flags (1 lit, 2 ortho, 4 sky, 8 ground, 16 sun), export pass/alpha
    eye: vec4f,     // xyz, tan(fov/2)
    fwd: vec4f,     // xyz, aspect
    right: vec4f,   // xyz, ortho half-height
    up: vec4f,
    origin: vec4f,  // domain min (m), cell size (m)
    size: vec4f,    // domain size (m), step (cells)
    ambient: vec4f,
    sun: vec4f,     // colour * intensity
    sun_dir: vec4f, // xyz, time (s)
    g0: vec4f,
    g1: vec4f,
    g2: vec4f,
    tk: vec4f,      // t min K, t max K, brightness, temperature multiplier
    smoke: vec4f,   // colour, absorption
    misc: vec4f,    // density, history weight, ground height, fine detail
    sky_top: vec4f,
    sky_bot: vec4f,
    ground: vec4f,
    plight: vec4f,  // point light position (m), on
    pcol: vec4f,    // point light colour * intensity, range (m)
    ns: vec4u,      // spark count
    sp: array<vec4f, 32>, // spark position (m), radius (m)
    sc: array<vec4f, 32>, // spark colour * brightness
    // ---- added after 0.4 ----
    e: vec4u,              // feature bits, flames colouring, shape count, shapes render | mask << 2 | invert << 4
    x: array<vec4f, 16>,   // feature settings (see `gpu.rs`)
    lut: array<vec4f, 256>, // Fire Color Gradient, 256 shades (feature bit 1)
    shp: array<vec4f, 48>, // shown shapes: centre + kind, half + torus, colour, world->local rows
}

@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var vol: texture_3d<f32>;   // temperature, smoke, light, fuel
@group(0) @binding(2) var vol_s: sampler;
@group(0) @binding(3) var out_img: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var<storage, read_write> accum: array<vec4f>;
@group(0) @binding(5) var<storage, read> occ: array<u32>;   // 8^3 bricks that contain anything
@group(0) @binding(6) var noise: texture_3d<f32>;           // tileable random values
@group(0) @binding(7) var noise_s: sampler;
@group(0) @binding(8) var vol2: texture_3d<f32>;  // flames (added after 0.4)

// feature bits (r.e.x)
const F_LUT: u32 = 1u;
const F_NOCLAMP: u32 = 2u;
const F_RAMP: u32 = 4u;
const F_NOFLAMES: u32 = 8u;
const F_TRANSLUCENT: u32 = 16u;
const F_NOSMOKE: u32 = 32u;
const F_SMOKE_REMAP: u32 = 64u;
const F_REMAP_TEMP: u32 = 128u;
const F_FUEL: u32 = 256u;
const F_SUN: u32 = 512u;
const F_AO: u32 = 1024u;
const F_GLOW: u32 = 2048u;
const F_SPARK_LIGHT: u32 = 4096u;
const F_EMASK: u32 = 8192u;
const F_SHARPEN: u32 = 16384u;
const F_POST: u32 = 32768u;
const F_MOD: u32 = 65536u;
const F_SLICE: u32 = 131072u;
const F_NOVOLUME: u32 = 262144u;
const F_NOPARTICLES: u32 = 524288u;
const F_VOL_OCC_P: u32 = 1048576u;
const F_P_OCC_VOL: u32 = 2097152u;
const F_SMOKE_LIM: u32 = 4194304u;
const F_FLAME_LIM: u32 = 8388608u;
const F_SHAPING: u32 = 16777216u;
const F_COLORING: u32 = 33554432u;

fn on(bit: u32) -> bool { return (r.e.x & bit) != 0u; }

fn brick_empty(uvw: vec3f) -> bool {
    let n = r.n.xyz;
    let c = vec3u(clamp(uvw * vec3f(n), vec3f(0.0), vec3f(n - 1u)));
    let b = c / 8u;
    let nb = (n + 7u) / 8u;
    return occ[b.x + nb.x * (b.y + nb.y * b.z)] == 0u;
}

// Hardware trilinear sample at a domain-relative position (0..1).
fn sample(uvw: vec3f) -> vec4f {
    return textureSampleLevel(vol, vol_s, uvw, 0.0);
}

fn gradient(k: f32) -> vec3f {
    if (on(F_LUT)) {
        let f = clamp(k, 0.0, 1.0) * 255.0;
        let i = u32(floor(f));
        return mix(r.lut[i].xyz, r.lut[min(i + 1u, 255u)].xyz, f - floor(f));
    }
    if (k < 0.5) { return mix(r.g0.xyz, r.g1.xyz, k * 2.0); }
    return mix(r.g1.xyz, r.g2.xyz, (k - 0.5) * 2.0);
}

fn blackbody(tk: f32) -> vec3f {
    let t = clamp(tk / 100.0, 10.0, 400.0);
    var c = vec3f(1.0);
    if (t > 66.0) { c.x = clamp(329.7 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0); }
    if (t <= 66.0) { c.y = (99.47 * log(t) - 161.1) / 255.0; } else { c.y = 288.1 * pow(t - 60.0, -0.0755) / 255.0; }
    if (t < 66.0) { c.z = select((138.5 * log(max(t - 10.0, 1e-3)) - 305.0) / 255.0, 0.0, t <= 19.0); }
    return pow(clamp(c, vec3f(0.0), vec3f(1.0)), vec3f(2.2));
}

fn aces(x: vec3f) -> vec3f {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3f(0.0), vec3f(1.0));
}

// Interleaved gradient noise: an even, low-discrepancy pattern (no clumpy grain),
// shifted every frame so the smoothing pass averages it away.
fn ign(p: vec2u, f: u32) -> f32 {
    let q = vec2f(p) + 5.588238 * f32(f % 64u);
    return fract(52.9829189 * fract(dot(q, vec2f(0.06711056, 0.00583715))));
}

// Henyey-Greenstein phase, relative to isotropic (1 = no change).
fn phase(cos_t: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_t;
    return (1.0 - g * g) / max(pow(d, 1.5), 1e-3);
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

fn in_shape(p: vec3f, k: u32) -> bool {
    let c = r.shp[k * 6u];
    let h = r.shp[k * 6u + 1u];
    let dw = p - c.xyz;
    let dl = vec3f(dot(r.shp[k * 6u + 3u].xyz, dw), dot(r.shp[k * 6u + 4u].xyz, dw), dot(r.shp[k * 6u + 5u].xyz, dw));
    return shape_dist(dl / max(h.xyz, vec3f(1e-4)), c.w, h.w) < 1.0;
}

/// Volume Processing slicing mask: fades the volume towards the domain walls.
fn slice_mask(uvw: vec3f) -> f32 {
    var m = 1.0;
    let ramp = max(r.x[10].w, 0.01);
    for (var a = 0u; a < 3u; a++) {
        let s = r.x[11u + a];
        let u = uvw[a];
        if (s.x > 0.0 || s.y > 0.0) { m *= pow(smoothstep(s.x, s.x + max(s.y, 1e-4), u), ramp); }
        if (s.z > 0.0 || s.w > 0.0) { m *= pow(smoothstep(s.z, s.z + max(s.w, 1e-4), 1.0 - u), ramp); }
    }
    return m;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let w = r.img.x;
    let h = r.img.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let flags = r.img.z;
    let x = ((f32(gid.x) + 0.5) / f32(w) * 2.0 - 1.0) * r.fwd.w;
    let y = 1.0 - (f32(gid.y) + 0.5) / f32(h) * 2.0;
    var ro = r.eye.xyz;
    var rd = normalize(r.fwd.xyz + r.right.xyz * x * r.eye.w + r.up.xyz * y * r.eye.w);
    if ((flags & 2u) != 0u) {
        ro = r.eye.xyz + r.right.xyz * x * r.right.w + r.up.xyz * y * r.right.w;
        rd = r.fwd.xyz;
    }

    // background
    var bg = vec3f(0.0);
    if ((flags & 4u) != 0u) {
        bg = mix(r.sky_bot.xyz, r.sky_top.xyz, clamp(rd.y * 0.5 + 0.5, 0.0, 1.0));
    }
    var t_end = 1e9;
    if ((flags & 8u) != 0u && rd.y < -1e-4) {
        let tg = (r.misc.z - ro.y) / rd.y;
        if (tg > 0.0) {
            t_end = tg;
            var light = r.ambient.xyz;
            if ((flags & 16u) != 0u) { light += r.sun.xyz * max(r.sun_dir.y, 0.0); }
            let fade = exp(-tg * 0.08);
            bg = mix(bg, r.ground.xyz * light, fade);
        }
    }

    // volume
    let o = r.origin.xyz;
    let cell = r.origin.w;
    let inv = 1.0 / select(rd, vec3f(1e-8), abs(rd) < vec3f(1e-8));
    let ta = (o - ro) * inv;
    let tb = (o + r.size.xyz - ro) * inv;
    let t0 = max(max(min(ta.x, tb.x), min(ta.y, tb.y)), max(min(ta.z, tb.z), 0.0));
    let t1 = min(min(max(ta.x, tb.x), max(ta.y, tb.y)), min(max(ta.z, tb.z), t_end));
    var col = vec3f(0.0);
    var trans = 1.0;
    // added after 0.4: export passes and shown shapes
    var ce = vec3f(0.0);
    var cs = vec3f(0.0);
    var kmax = 0.0;
    var fuel_a = 0.0;
    var depth = 1.0;
    var holdout = false;
    let nshapes = r.e.z;
    let step = r.size.w;
    let ds = cell * step;
    if (t1 > t0 && !on(F_NOVOLUME)) {
        var t = t0 + ds * ign(gid.xy, r.n.w);
        let t_first = t;
        let lit = (flags & 1u) != 0u;
        let has_sun = (flags & 16u) != 0u;
        let brick = cell * 8.0;
        let detail = r.misc.w * cell;
        let time = r.sun_dir.w;
        loop {
            if (t >= t1 || trans < 0.005) { break; }
            let p = ro + rd * t;
            var uvw = (p - o) / r.size.xyz;
            // shown shapes (Emitter › Show emitter) and Render › Shapes Rendering / Masking
            var smask = 1.0;
            if (nshapes > 0u) {
                var hit = 0u;
                var inside_any = false;
                for (var k = 0u; k < nshapes; k++) {
                    if (in_shape(p, k)) { inside_any = true; hit = k; break; }
                }
                let srender = r.e.w & 3u;
                let smaskm = (r.e.w >> 2u) & 3u;
                let sinv = (r.e.w & 16u) != 0u;
                if (inside_any && srender != 2u) {
                    if (srender == 1u) {
                        var lg = r.ambient.xyz + vec3f(0.15);
                        if (has_sun) { lg += r.sun.xyz * 0.6; }
                        col += trans * r.shp[hit * 6u + 2u].xyz * lg;
                    } else {
                        holdout = !sinv;
                    }
                    trans = 0.0;
                    depth = min(depth, (t - t0) / length(r.size.xyz));
                    break;
                }
                if (smaskm == 1u && inside_any) { smask = 0.0; }
                if (smaskm == 2u && !inside_any) { smask = 0.0; }
            }
            // skip empty space a brick at a time
            if (brick_empty(uvw)) {
                // jump to where the ray leaves this empty brick, landing on the ray's own step
                // pattern (0.4 jumped half a brick, which could land past the start of the next,
                // filled brick and cut its edge off)
                let bc = floor(clamp(uvw * vec3f(r.n.xyz), vec3f(0.0), vec3f(r.n.xyz - 1u)) / 8.0);
                let bmin = o + bc * brick;
                let tb0 = (bmin - ro) * inv;
                let tb1 = (bmin + vec3f(brick) - ro) * inv;
                let texit = min(min(max(tb0.x, tb1.x), max(tb0.y, tb1.y)), max(tb0.z, tb1.z));
                t = max(t + ds, t_first + ceil((texit - t_first) / ds) * ds);
                continue;
            }
            // fine detail: nudge the lookup by small animated noise (finer than one cell)
            if (detail > 0.0) {
                let q = p * (3.0 / cell) * 0.07 + vec3f(0.0, -time * 0.6, 0.0);
                let nz = textureSampleLevel(noise, noise_s, q, 0.0).xyz + 0.5 * textureSampleLevel(noise, noise_s, q * 2.7, 0.0).xyz - 0.75;
                uvw += nz * detail / r.size.xyz;
            }
            var s = sample(uvw);
            // Volume Processing post process (sharpen / dilate) and Shading › Raymarch sharpening
            if (on(F_POST) || on(F_SHARPEN)) {
                let du = vec3f(1.0) / vec3f(r.n.xyz);
                let n6 = (sample(uvw + vec3f(du.x, 0.0, 0.0)) + sample(uvw - vec3f(du.x, 0.0, 0.0)) + sample(uvw + vec3f(0.0, du.y, 0.0))
                    + sample(uvw - vec3f(0.0, du.y, 0.0)) + sample(uvw + vec3f(0.0, 0.0, du.z)) + sample(uvw - vec3f(0.0, 0.0, du.z))) / 6.0;
                let mode = r.x[8].w;
                let st = r.x[6].w;
                if (on(F_POST) && mode > 1.5 && mode < 2.5) { s = vec4f(max(s.xy + (s.xy - n6.xy) * st * 2.0, vec2f(0.0)), s.zw); }
                // dilate: grow into empty neighbours without brightening what is already there
                if (on(F_POST) && mode > 2.5) { s = vec4f(mix(s.xy, max(s.xy, n6.xy), st), s.zw); }
                if (on(F_SHARPEN)) {
                    // unsharp mask: crisper edges, same overall density and heat (100% = off)
                    s.y = max(s.y + (s.y - n6.y) * (r.x[9].x - 1.0) * 0.5, 0.0);
                    s.x = max(s.x + (s.x - n6.x) * (r.x[9].z - 1.0) * 0.5, 0.0);
                }
            }
            let sm_raw = max(s.y, 0.0);
            var sm = sm_raw * r.misc.x;
            if (on(F_SMOKE_LIM)) {
                if (sm_raw < r.x[2].w) { sm = 0.0; }
                sm = min(sm, r.x[3].x * r.misc.x);
            }
            if (on(F_NOSMOKE)) { sm = 0.0; }
            // Volume Processing: slicing mask and noise modulation
            var vm = smask;
            if (on(F_SLICE)) { vm *= slice_mask(uvw); }
            var kmod = 1.0;
            if (on(F_MOD)) {
                let q = p / max(r.x[10].x, 1e-3) * 0.05 + vec3f(0.0, -time * r.x[10].z * 0.2, 0.0);
                let nz = textureSampleLevel(noise, noise_s, q, 0.0).x;
                let mm = 1.0 - r.x[10].y * nz;
                if (r.x[9].w < 0.5) { sm *= mm; } else { kmod = mm; }
            }
            sm *= vm;
            let tk = s.x * r.tk.w * 1000.0 + 300.0;
            var k = clamp((tk - r.tk.x) / max(r.tk.y - r.tk.x, 1.0), 0.0, 1.0);
            if (on(F_NOCLAMP)) { k = max((tk - r.tk.x) / max(r.tk.y - r.tk.x, 1.0), 0.0); }
            if (on(F_RAMP)) { k = pow(k, r.x[0].x); }
            k *= kmod * vm;
            if (on(F_NOFLAMES)) { k = 0.0; }
            kmax = max(kmax, k);
            // hot gas glows and absorbs a little, which gives the flame body its dark folds
            var fire_sigma = k * k * 0.08;
            if (on(F_TRANSLUCENT)) { fire_sigma = 0.0; }
            if (on(F_FLAME_LIM)) {
                if (k < r.x[0].y) { k = 0.0; fire_sigma = 0.0; }
                fire_sigma += r.x[0].z * min(k, 1.0) * 1.5;
            }
            var sigma = sm * r.smoke.w * 0.3 + fire_sigma;
            var fsig = 0.0;
            if (on(F_FUEL)) { fsig = max(s.w, 0.0) * r.x[5].w * 0.5 * vm; sigma += fsig; }
            if (sigma > 1e-5 || k > 0.0) {
                let a = 1.0 - exp(-sigma * step);
                var light = r.ambient.xyz;
                if (on(F_AO)) {
                    let up = sample(uvw + vec3f(0.0, 4.0 / f32(r.n.y), 0.0)).y + sample(uvw + vec3f(0.0, 8.0 / f32(r.n.y), 0.0)).y;
                    light *= mix(1.0, exp(-up * 1.5), r.x[3].y);
                }
                if (has_sun) {
                    if (on(F_SUN)) {
                        // Shading › Lighting: shadow intensity, extinction colour, anisotropy, direct light
                        let sh = select(1.0, s.z, lit);
                        let tint = mix(r.x[4].xyz, vec3f(1.0), sh);
                        light += r.sun.xyz * pow(max(sh, 1e-4), r.x[3].z) * tint * phase(dot(r.sun_dir.xyz, rd), r.x[4].w) * r.x[3].w;
                    } else {
                        light += r.sun.xyz * select(1.0, s.z, lit);
                    }
                }
                if (r.plight.w > 0.5) {
                    let dl = p - r.plight.xyz;
                    light += r.pcol.xyz / (1.0 + dot(dl, dl) / max(r.pcol.w * r.pcol.w, 1e-4));
                }
                if (on(F_GLOW)) {
                    // Shading › Scattering › Flames contribution: nearby flames light the smoke
                    let rr = r.x[6].x / r.size.xyz;
                    var gl = vec3f(0.0);
                    for (var d = 0u; d < 6u; d++) {
                        var dir = vec3f(0.0);
                        dir[d / 2u] = select(-1.0, 1.0, (d & 1u) == 0u);
                        let q = sample(uvw + dir * rr);
                        let gk = clamp((q.x * r.tk.w * 1000.0 + 300.0 - r.tk.x) / max(r.tk.y - r.tk.x, 1.0), 0.0, 1.0);
                        gl += gradient(gk) * pow(gk, 3.0) * exp(-max(q.y, 0.0) * r.x[6].z * 4.0);
                    }
                    light += gl / 6.0 * r.x[6].y * 6.0 * r.tk.z;
                }
                if (on(F_SPARK_LIGHT)) {
                    for (var k2 = 0u; k2 < min(r.ns.x, 8u); k2++) {
                        let dl = p - r.sp[k2].xyz;
                        light += r.sc[k2].xyz * r.x[14].x * 0.02 / (0.01 + dot(dl, dl));
                    }
                }
                var albedo = r.smoke.xyz;
                if (on(F_SMOKE_REMAP)) {
                    let v = select(sm_raw, tk, on(F_REMAP_TEMP));
                    albedo = gradient(pow(clamp((v - r.x[2].x) / max(r.x[2].y - r.x[2].x, 1e-4), 0.0, 1.0), r.x[2].z));
                }
                var scat = albedo * light * min(sm * r.smoke.w * 0.3 / max(sigma, 1e-5), 1.0);
                if (on(F_FUEL)) { scat += r.x[5].xyz * light * min(fsig / max(sigma, 1e-5), 1.0); }
                var emit = gradient(k) * pow(k, 3.0) * r.tk.z * 6.0;
                if (on(F_COLORING)) {
                    let mode = r.e.y;
                    if (mode == 1u) { emit = gradient(k) * select(0.0, 1.5, k > 0.02) * r.tk.z; }
                    else if (mode == 2u) { emit = blackbody(tk) * pow(k, 3.0) * r.tk.z * 6.0; }
                    else if (mode == 3u) { emit = r.x[1].xyz * pow(k, 3.0) * r.tk.z * 6.0; }
                }
                if (on(F_SHAPING)) {
                    // Shading › Flames: emission only where fuel burns (flames) or where there is smoke
                    let sf = r.x[0].w;
                    if (sf > 0.0) { emit *= mix(1.0, clamp(max(textureSampleLevel(vol2, vol_s, uvw, 0.0).x, 0.0) * sf * 20.0, 0.0, 1.0), min(sf, 1.0)); }
                    let sd = r.x[1].w;
                    if (sd > 0.0) { emit *= mix(1.0, clamp(sm_raw * sd, 0.0, 1.0), min(sd, 1.0)); }
                }
                if (on(F_EMASK)) {
                    // Shading › Emissive masking: fire hidden inside thick smoke, with a noisy edge
                    var mm = smoothstep(r.x[7].x - r.x[7].y * 0.5, r.x[7].x + r.x[7].y * 0.5, sm_raw);
                    mm = pow(mm, r.x[7].z);
                    let q = p / cell;
                    let n1 = textureSampleLevel(noise, noise_s, q * 0.02, 0.0).x;
                    let n2 = textureSampleLevel(noise, noise_s, q * 0.07, 0.0).y;
                    let n3 = textureSampleLevel(noise, noise_s, q * 0.25, 0.0).z;
                    let ws = max(r.x[8].x + r.x[8].y + r.x[8].z, 1e-3);
                    let nz = (r.x[8].x * n1 + r.x[8].y * n2 + r.x[8].z * n3) / ws;
                    emit *= 1.0 - r.x[7].w * clamp(mm * (0.4 + 1.2 * nz), 0.0, 1.0);
                }
                col += trans * (scat * a + emit * step * 0.5);
                ce += trans * emit * step * 0.5;
                cs += trans * scat * a;
                fuel_a += trans * (1.0 - exp(-max(s.w, 0.0) * step));
                trans *= 1.0 - a;
                if (trans < 0.5 && depth >= 1.0) { depth = (t - t0) / length(r.size.xyz); }
            }
            t += ds;
        }
    }
    let alpha = 1.0 - trans;
    let vcol = col;
    if (holdout) { bg = vec3f(0.0); }
    col += bg * trans;
    var spk = vec3f(0.0);
    // glowing particles (sparks)
    if (!on(F_NOPARTICLES)) {
        for (var k = 0u; k < r.ns.x; k++) {
            let sp = r.sp[k];
            let tc = dot(sp.xyz - ro, rd);
            if (tc <= 0.0 || tc > t_end) { continue; }
            let d = ro + rd * tc - sp.xyz;
            let rr = max(sp.w, 1e-4);
            if (on(F_VOL_OCC_P) || on(F_P_OCC_VOL)) {
                // Render settings: the volume hides sparks behind it, and/or sparks cover the volume
                let glow = r.sc[k].xyz * exp(-dot(d, d) / (rr * rr)) * 3.0;
                let cover = clamp(exp(-dot(d, d) / (rr * rr)), 0.0, 1.0);
                var gg = glow;
                if (on(F_VOL_OCC_P)) { gg *= trans; }
                if (on(F_P_OCC_VOL)) { col = col * (1.0 - cover) + gg; } else { col += gg; }
                spk += gg;
            } else {
                col += r.sc[k].xyz * exp(-dot(d, d) / (rr * rr)) * 3.0;
                spk += r.sc[k].xyz * exp(-dot(d, d) / (rr * rr)) * 3.0;
            }
        }
    }
    // smoothing over frames / progressive refinement (weight 1 = fresh image)
    let ai = gid.y * w + gid.x;
    var hw = r.misc.y;
    if (on(F_POST) && r.x[8].w > 0.5 && r.x[8].w < 1.5) {
        // Volume Processing › Motion Blur: keep more of the previous frames
        hw = hw * (1.0 - r.x[6].w * 0.85);
    }
    let pass_ = r.img.w & 15u;
    let transparent = (r.img.w & 16u) != 0u;
    if (pass_ == 0u && !transparent) {
        // the 0.4 image
        if (hw < 0.999) { col = mix(accum[ai].xyz, col, hw); }
        accum[ai] = vec4f(col, 1.0);
        let c = pow(aces(col), vec3f(1.0 / 2.2));
        textureStore(out_img, vec2i(gid.xy), vec4f(c, 1.0));
        return;
    }
    // export passes: 0 render all (transparent), 1 alpha, 2 emission, 3 scattering,
    // 4 temperature, 5 density, 6 fuel, 7 depth
    var v = vec4f(vcol + spk, alpha);
    if (!transparent) { v = vec4f(col, 1.0); }
    if (pass_ == 1u || pass_ == 5u) { v = vec4f(vec3f(alpha), 1.0); }
    else if (pass_ == 2u) { v = vec4f(ce + spk, alpha); }
    else if (pass_ == 3u) { v = vec4f(cs, alpha); }
    else if (pass_ == 4u) { v = vec4f(vec3f(kmax), 1.0); }
    else if (pass_ == 6u) { v = vec4f(vec3f(fuel_a), 1.0); }
    else if (pass_ == 7u) { v = vec4f(vec3f(select(1.0, depth, alpha > 0.0)), 1.0); }
    if (hw < 0.999) { v = mix(accum[ai], v, hw); }
    accum[ai] = v;
    var rgb = v.xyz;
    if (pass_ == 0u || pass_ == 2u || pass_ == 3u) {
        if ((r.img.w & 64u) != 0u && v.w > 1e-4) { rgb = rgb / v.w; } // straight alpha
        rgb = pow(aces(rgb), vec3f(1.0 / 2.2));
    }
    var a = v.w;
    if ((r.img.w & 32u) != 0u) { a = 1.0 - a; }
    textureStore(out_img, vec2i(gid.xy), vec4f(rgb, select(1.0, a, transparent)));
}
