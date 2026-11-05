// GPU raymarcher: emission/absorption volume with sun shadows, written to a storage texture.

struct R {
    n: vec4u,       // nx, ny, nz, frame
    img: vec4u,     // width, height, flags (1 lit, 2 ortho, 4 sky, 8 ground, 16 sun), HQ sample index
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
}

@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var vol: texture_3d<f32>;   // temperature, smoke, light, fuel
@group(0) @binding(2) var vol_s: sampler;
@group(0) @binding(3) var out_img: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var<storage, read_write> accum: array<vec4f>;
@group(0) @binding(5) var<storage, read> occ: array<u32>;   // 8^3 bricks that contain anything
@group(0) @binding(6) var noise: texture_3d<f32>;           // tileable random values
@group(0) @binding(7) var noise_s: sampler;

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
    if (k < 0.5) { return mix(r.g0.xyz, r.g1.xyz, k * 2.0); }
    return mix(r.g1.xyz, r.g2.xyz, (k - 0.5) * 2.0);
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
    let step = r.size.w;
    let ds = cell * step;
    if (t1 > t0) {
        var t = t0 + ds * ign(gid.xy, r.n.w);
        let lit = (flags & 1u) != 0u;
        let has_sun = (flags & 16u) != 0u;
        let brick = cell * 8.0;
        let detail = r.misc.w * cell;
        let time = r.sun_dir.w;
        loop {
            if (t >= t1 || trans < 0.005) { break; }
            let p = ro + rd * t;
            var uvw = (p - o) / r.size.xyz;
            // skip empty space a brick at a time
            if (brick_empty(uvw)) {
                t += max(brick * 0.5, ds);
                continue;
            }
            // fine detail: nudge the lookup by small animated noise (finer than one cell)
            if (detail > 0.0) {
                let q = p * (3.0 / cell) * 0.07 + vec3f(0.0, -time * 0.6, 0.0);
                let nz = textureSampleLevel(noise, noise_s, q, 0.0).xyz + 0.5 * textureSampleLevel(noise, noise_s, q * 2.7, 0.0).xyz - 0.75;
                uvw += nz * detail / r.size.xyz;
            }
            let s = sample(uvw);
            let sm = max(s.y, 0.0) * r.misc.x;
            let tk = s.x * r.tk.w * 1000.0 + 300.0;
            let k = clamp((tk - r.tk.x) / max(r.tk.y - r.tk.x, 1.0), 0.0, 1.0);
            // hot gas glows and absorbs a little, which gives the flame body its dark folds
            let fire_sigma = k * k * 0.08;
            let sigma = sm * r.smoke.w * 0.3 + fire_sigma;
            if (sigma > 1e-5 || k > 0.0) {
                let a = 1.0 - exp(-sigma * step);
                var light = r.ambient.xyz;
                if (has_sun) { light += r.sun.xyz * select(1.0, s.z, lit); }
                if (r.plight.w > 0.5) {
                    let dl = p - r.plight.xyz;
                    light += r.pcol.xyz / (1.0 + dot(dl, dl) / max(r.pcol.w * r.pcol.w, 1e-4));
                }
                let scat = r.smoke.xyz * light * min(sm * r.smoke.w * 0.3 / max(sigma, 1e-5), 1.0);
                let emit = gradient(k) * pow(k, 3.0) * r.tk.z * 6.0;
                col += trans * (scat * a + emit * step * 0.5);
                trans *= 1.0 - a;
            }
            t += ds;
        }
    }
    col += bg * trans;
    // glowing particles (sparks)
    for (var k = 0u; k < r.ns.x; k++) {
        let sp = r.sp[k];
        let tc = dot(sp.xyz - ro, rd);
        if (tc <= 0.0 || tc > t_end) { continue; }
        let d = ro + rd * tc - sp.xyz;
        let rr = max(sp.w, 1e-4);
        col += r.sc[k].xyz * exp(-dot(d, d) / (rr * rr)) * 3.0;
    }
    // smoothing over frames / progressive refinement (weight 1 = fresh image)
    let ai = gid.y * w + gid.x;
    let hw = r.misc.y;
    if (hw < 0.999) { col = mix(accum[ai].xyz, col, hw); }
    accum[ai] = vec4f(col, 1.0);
    let c = pow(aces(col), vec3f(1.0 / 2.2));
    textureStore(out_img, vec2i(gid.xy), vec4f(c, 1.0));
}
