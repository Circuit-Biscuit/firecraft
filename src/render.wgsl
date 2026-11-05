// GPU raymarcher: emission / absorption / scattering through the simulated volume.
// Writes the volume colour (premultiplied) + coverage into `accum` and the background into `bg`;
// post.wgsl finishes the picture (bloom, tonemapping, style, alpha).

struct R {
    n: vec4u,       // nx, ny, nz, seed
    img: vec4u,     // width, height, flags, pass
    eye: vec4f,     // xyz, tan(fov/2)
    fwd: vec4f,     // xyz, aspect
    right: vec4f,   // xyz, ortho half-height
    up: vec4f,      // xyz, time (s)
    origin: vec4f,  // domain min (m), cell size (m)
    size: vec4f,    // domain size (m), step (cells)
    ambient: vec4f, // colour, ambient occlusion strength
    sun: vec4f,     // colour * intensity, anisotropy g
    sun_dir: vec4f, // xyz, direct light contribution
    extc: vec4f,    // extinction colour, shadow intensity
    fl: vec4f,      // remap min K, remap max K, ramp, colouring mode
    fl2: vec4f,     // shaping by flames, shaping by density, flames min limit, flames scale
    fl3: vec4f,     // fire colour, flames opacity
    sm: vec4f,      // smoke colour, density scale
    sm2: vec4f,     // smoke remap min, max, ramp, colouring mode
    sm3: vec4f,     // smoke min limit, smoke clamp, history weight, ground height
    fu: vec4f,      // fuel colour, fuel scale
    sc: vec4f,      // flames light, particles light, _, detail (cells)
    em: vec4f,      // emissive masking: reference, width, ramp, intensity
    em2: vec4f,     // low, medium, high frequencies, fast detail
    sl0: vec4f,     // slicing x-: offset, width; x+: offset, width (fractions)
    sl1: vec4f,     // z-, z+
    sl2: vec4f,     // y- (ground), y+ (roof)
    sl3: vec4f,     // mask ramp, _, _, _
    sky_top: vec4f,
    sky_bot: vec4f,
    ground: vec4f,
    cnt: vec4u,     // lights, shapes, sparks, shape flags (bits 0-1 render mode, 2-3 masking, 4 invert, 5 sparks HQ)
    lights: array<vec4f, 8>,  // 4 x (position + range, colour)
    shapes: array<vec4f, 48>, // 8 x (centre + kind, half + torus, colour, rot rows x3)
    sp: array<vec4f, 32>,     // spark position (m), radius (m)
    scol: array<vec4f, 32>,   // spark colour * brightness
}

@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var vol: texture_3d<f32>;   // temperature, smoke, sun light, flames
@group(0) @binding(2) var vol_s: sampler;
@group(0) @binding(3) var vol2: texture_3d<f32>;  // fuel, ambient occlusion, glow, glow temperature
@group(0) @binding(4) var<storage, read_write> accum: array<vec4f>;
@group(0) @binding(5) var<storage, read> occ: array<u32>;
@group(0) @binding(6) var noise: texture_3d<f32>;
@group(0) @binding(7) var noise_s: sampler;
@group(0) @binding(8) var<storage, read> lut: array<vec4f, 256>;
@group(0) @binding(9) var<storage, read_write> bgbuf: array<vec4f>;

fn flag(b: u32) -> bool { return (r.img.z & b) != 0u; }

fn brick_empty(uvw: vec3f) -> bool {
    let n = r.n.xyz;
    let c = vec3u(clamp(uvw * vec3f(n), vec3f(0.0), vec3f(n - 1u)));
    let b = c / 8u;
    let nb = (n + 7u) / 8u;
    return occ[b.x + nb.x * (b.y + nb.y * b.z)] == 0u;
}

fn gradient(k: f32) -> vec3f {
    let x = clamp(k, 0.0, 1.0) * 255.0;
    let i = u32(floor(x));
    return mix(lut[i].xyz, lut[min(i + 1u, 255u)].xyz, fract(x));
}

fn blackbody(tk: f32) -> vec3f {
    let t = clamp(tk / 100.0, 10.0, 400.0);
    var c: vec3f;
    if (t <= 66.0) {
        c = vec3f(1.0, clamp((99.47 * log(t) - 161.1) / 255.0, 0.0, 1.0), select(clamp((138.5 * log(t - 10.0) - 305.0) / 255.0, 0.0, 1.0), 0.0, t <= 19.0));
    } else {
        c = vec3f(clamp(329.7 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0), clamp(288.1 * pow(t - 60.0, -0.0755) / 255.0, 0.0, 1.0), 1.0);
    }
    return pow(c, vec3f(2.2));
}

// reference voxel size (m) for per-metre density and glow
const H_REF: f32 = 0.0044643;

fn ign(p: vec2u, f: u32) -> f32 {
    let q = vec2f(p) + 5.588238 * f32(f % 64u);
    return fract(52.9829189 * fract(dot(q, vec2f(0.06711056, 0.00583715))));
}

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

/// Inside shape k? (0 = outside, 1 = inside)
fn in_shape(p: vec3f, k: u32) -> bool {
    let c = r.shapes[k * 6u];
    let hs = r.shapes[k * 6u + 1u];
    let dw = p - c.xyz;
    let dl = vec3f(dot(r.shapes[k * 6u + 3u].xyz, dw), dot(r.shapes[k * 6u + 4u].xyz, dw), dot(r.shapes[k * 6u + 5u].xyz, dw));
    return shape_dist(dl / max(hs.xyz, vec3f(1e-4)), c.w, hs.w) < 1.0;
}

fn slice_mask(uvw: vec3f) -> f32 {
    var m = 1.0;
    let side = array<vec2f, 6>(r.sl0.xy, r.sl0.zw, r.sl1.xy, r.sl1.zw, r.sl2.xy, r.sl2.zw);
    let dist = array<f32, 6>(uvw.x, 1.0 - uvw.x, uvw.z, 1.0 - uvw.z, uvw.y, 1.0 - uvw.y);
    for (var k = 0; k < 6; k++) {
        let s = side[k];
        if (s.x > 0.0 || s.y > 0.0) { m *= smoothstep(s.x, s.x + max(s.y, 1e-4), dist[k]); }
    }
    return pow(m, r.sl3.x);
}

fn hg(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5);
}

struct Spark { t: f32, c: vec3f, a: f32 }

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let w = r.img.x;
    let h = r.img.y;
    if (gid.x >= w || gid.y >= h) { return; }
    let pass_ = r.img.w;
    let x = ((f32(gid.x) + 0.5) / f32(w) * 2.0 - 1.0) * r.fwd.w;
    let y = 1.0 - (f32(gid.y) + 0.5) / f32(h) * 2.0;
    var ro = r.eye.xyz;
    var rd = normalize(r.fwd.xyz + r.right.xyz * x * r.eye.w + r.up.xyz * y * r.eye.w);
    if (flag(2u)) {
        ro = r.eye.xyz + r.right.xyz * x * r.right.w + r.up.xyz * y * r.right.w;
        rd = r.fwd.xyz;
    }
    let time = r.up.w;

    // background
    var bg = vec3f(0.0);
    if (flag(4u)) { bg = mix(r.sky_bot.xyz, r.sky_top.xyz, clamp(rd.y * 0.5 + 0.5, 0.0, 1.0)); }
    var t_end = 1e9;
    if (flag(8u) && rd.y < -1e-4) {
        let tg = (r.sm3.w - ro.y) / rd.y;
        if (tg > 0.0) {
            t_end = tg;
            var lg = r.ambient.xyz;
            if (flag(16u)) { lg += r.sun.xyz * max(r.sun_dir.y, 0.0); }
            let fade = exp(-tg * 0.08);
            bg = mix(bg, r.ground.xyz * lg, fade);
        }
    }

    // sparks near this ray, sorted by distance along it
    var sparks: array<Spark, 8>;
    var nsp = 0u;
    let hq = (r.cnt.w & 32u) != 0u;
    if (flag(8192u)) {
        for (var k = 0u; k < r.cnt.z; k++) {
            let sp = r.sp[k];
            let tc = dot(sp.xyz - ro, rd);
            if (tc <= 0.0 || tc > t_end) { continue; }
            let d = ro + rd * tc - sp.xyz;
            let rr = max(sp.w, 1e-4);
            let d2 = dot(d, d) / (rr * rr);
            var a = exp(-d2);
            if (hq) { a += 0.25 * exp(-d2 / 9.0); }
            if (a < 0.002 || nsp >= 8u) { continue; }
            var j = nsp;
            while (j > 0u && sparks[j - 1u].t > tc) { sparks[j] = sparks[j - 1u]; j--; }
            sparks[j] = Spark(tc, r.scol[k].xyz * a * 3.0, min(a, 1.0));
            nsp++;
        }
    }
    var next_spark = 0u;

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
    var holdout = false;
    var tmax = 0.0;
    var depth = 1.0;
    let step = r.size.w;
    let ds = cell * step;
    // density and glow add up per metre, not per voxel: the same look on any voxel size
    // (calibrated to 0.4.0 High, 224 voxels per metre)
    let stepm = step * cell / H_REF;
    let nshapes = r.cnt.y;
    let shape_render = r.cnt.w & 3u;     // 0 holdout, 1 visible, 2 hidden
    let shape_mask = (r.cnt.w >> 2u) & 3u; // 0 none, 1 inside, 2 outside
    let shape_inv = (r.cnt.w & 16u) != 0u;
    if (t1 > t0 && flag(4096u)) {
        var t = t0 + ds * ign(gid.xy, r.n.x + r.n.w);
        let lit = flag(1u);
        let has_sun = flag(16u);
        let brick = cell * 8.0;
        let detail = r.sc.w * cell;
        let cos_t = dot(r.sun_dir.xyz, rd);
        let phase = hg(cos_t, r.sun.w);
        loop {
            if (t >= t1 || trans < 0.004) { break; }
            let p = ro + rd * t;
            // sparks between the last step and here
            while (next_spark < nsp && sparks[next_spark].t <= t) {
                let s = sparks[next_spark];
                if (pass_ == 0u || pass_ == 2u) { col += s.c * select(1.0, trans, flag(16384u)); }
                if (flag(32768u)) { trans *= 1.0 - s.a * 0.8; }
                next_spark++;
            }
            // shapes: visible / holdout / masking
            var smask = 1.0;
            if (nshapes > 0u) {
                var inside_any = false;
                var hit = 0u;
                for (var k = 0u; k < nshapes; k++) {
                    if (in_shape(p, k)) { inside_any = true; hit = k; break; }
                }
                if (inside_any != shape_inv && shape_render != 2u) {
                    if (shape_render == 1u) {
                        let sc = r.shapes[hit * 6u + 2u].xyz;
                        var lg = r.ambient.xyz + vec3f(0.15);
                        if (has_sun) { lg += r.sun.xyz * 0.6; }
                        if (pass_ == 0u || pass_ == 3u) { col += trans * sc * lg; }
                    } else {
                        holdout = true;
                    }
                    trans = 0.0;
                    depth = min(depth, (t - t0) / length(r.size.xyz));
                    break;
                }
                if (shape_mask == 1u && inside_any) { smask = 0.0; }
                if (shape_mask == 2u && !inside_any) { smask = 0.0; }
            }
            var uvw = (p - o) / r.size.xyz;
            if (brick_empty(uvw)) {
                t += max(brick * 0.5, ds);
                continue;
            }
            if (detail > 0.0) {
                let q = p * (3.0 / cell) * 0.07 + vec3f(0.0, -time * 0.6, 0.0);
                var nz = textureSampleLevel(noise, noise_s, q, 0.0).xyz - 0.5;
                if (r.em2.w < 0.5) { nz += 0.5 * (textureSampleLevel(noise, noise_s, q * 2.7, 0.0).xyz - 0.5); }
                uvw += nz * detail / r.size.xyz;
            }
            let s = textureSampleLevel(vol, vol_s, uvw, 0.0);
            let s2 = textureSampleLevel(vol2, vol_s, uvw, 0.0);
            let m = slice_mask(uvw) * smask;
            // smoke
            let sm_raw = max(s.y, 0.0);
            var sm = 0.0;
            if (flag(64u) && sm_raw > r.sm3.x) { sm = min(sm_raw, r.sm3.y) * r.sm.w * 0.9 * m; }
            // flames
            let tk = s.x * 1000.0 + 300.0;
            var k = max((tk - r.fl.x) / max(r.fl.y - r.fl.x, 1.0), 0.0);
            if (flag(256u)) { k = min(k, 1.0); }
            k = pow(k, r.fl.z);
            var emit = vec3f(0.0);
            var fire_sigma = 0.0;
            if (flag(32u) && k > r.fl2.z) {
                var fw = 1.0;
                let sf = r.fl2.x;
                if (sf > 0.0) { fw = mix(1.0, clamp(max(s.w, 0.0) * sf * 0.1, 0.0, 1.0), min(sf, 1.0)); }
                let sd = r.fl2.y;
                if (sd > 0.0) { fw *= mix(1.0, clamp(sm_raw * sd, 0.0, 1.0), min(sd, 1.0)); }
                let ramp = pow(k, 3.0) * 6.0;
                let mode = r.fl.w;
                if (mode < 0.5) { emit = gradient(k) * ramp; }
                else if (mode < 1.5) { emit = gradient(k) * select(0.0, 1.5, k > 0.02); }
                else if (mode < 2.5) { emit = blackbody(tk) * ramp; }
                else { emit = r.fl3.xyz * ramp; }
                emit *= r.fl2.w * fw * m;
                if (flag(1024u)) {
                    // emissive masking: fire is hidden inside thick smoke, with a noisy edge
                    var mm = smoothstep(r.em.x - r.em.y * 0.5, r.em.x + r.em.y * 0.5, sm_raw);
                    mm = pow(mm, r.em.z);
                    let q = p / cell;
                    let n1 = textureSampleLevel(noise, noise_s, q * 0.02, 0.0).x;
                    let n2 = textureSampleLevel(noise, noise_s, q * 0.07, 0.0).y;
                    let n3 = textureSampleLevel(noise, noise_s, q * 0.25, 0.0).z;
                    let wsum = max(r.em2.x + r.em2.y + r.em2.z, 1e-3);
                    let nz = (r.em2.x * n1 + r.em2.y * n2 + r.em2.z * n3) / wsum;
                    emit *= 1.0 - r.em.w * clamp(mm * (0.4 + 1.2 * nz), 0.0, 1.0);
                }
                if (!flag(512u)) { fire_sigma = min(k, 1.0) * min(k, 1.0) * 0.08 * m; }
                fire_sigma += r.fl3.w * min(k, 1.0) * 1.5 * m;
            }
            tmax = max(tmax, min(k, 1.0) * m);
            // fuel
            var fsig = 0.0;
            if (flag(128u)) { fsig = max(s2.x, 0.0) * r.fu.w * 0.5 * m; }
            let sigma = sm + fire_sigma + fsig;
            if (sigma > 1e-5 || dot(emit, emit) > 0.0) {
                let a = 1.0 - exp(-sigma * stepm);
                // light reaching this point
                var lg = r.ambient.xyz * mix(1.0, s2.y, r.ambient.w);
                if (has_sun) {
                    let sh = select(1.0, s.z, lit);
                    let shadowed = pow(max(sh, 1e-4), r.extc.w);
                    let tint = mix(r.extc.xyz, vec3f(1.0), sh);
                    lg += r.sun.xyz * shadowed * tint * phase * r.sun_dir.w;
                }
                for (var li = 0u; li < r.cnt.x; li++) {
                    let lp = r.lights[li * 2u];
                    let dl = p - lp.xyz;
                    lg += r.lights[li * 2u + 1u].xyz / (1.0 + dot(dl, dl) / max(lp.w * lp.w, 1e-4));
                }
                if (r.sc.x > 0.0 && s2.z > 0.0) {
                    let gk = max((s2.w * 1000.0 + 300.0 - r.fl.x) / max(r.fl.y - r.fl.x, 1.0), 0.0);
                    lg += gradient(min(gk, 1.0)) * s2.z * r.sc.x * 6.0;
                }
                if (r.sc.y > 0.0) {
                    for (var k2 = 0u; k2 < min(r.cnt.z, 8u); k2++) {
                        let dl = p - r.sp[k2].xyz;
                        lg += r.scol[k2].xyz * r.sc.y * 0.02 / (0.01 + dot(dl, dl));
                    }
                }
                var albedo = r.sm.xyz;
                if (r.sm2.w > 0.5) {
                    let v = select(sm_raw, tk, flag(2048u));
                    let kk = pow(clamp((v - r.sm2.x) / max(r.sm2.y - r.sm2.x, 1e-4), 0.0, 1.0), r.sm2.z);
                    albedo = gradient(kk);
                }
                let scat = (albedo * sm + r.fu.xyz * fsig) / max(sigma, 1e-5) * lg;
                if (pass_ == 0u) { col += trans * (scat * a + emit * stepm * 0.5); }
                else if (pass_ == 2u) { col += trans * emit * stepm * 0.5; }
                else if (pass_ == 3u) { col += trans * scat * a; }
                else if (pass_ == 6u) { col += trans * vec3f(1.0 - exp(-fsig * stepm)); }
                trans *= 1.0 - a;
                if (trans < 0.5) { depth = min(depth, (t - t0) / length(r.size.xyz)); }
            }
            t += ds;
        }
    }
    // sparks after the volume
    while (next_spark < nsp) {
        let s = sparks[next_spark];
        if (pass_ == 0u || pass_ == 2u) { col += s.c * select(1.0, trans, flag(16384u)); }
        if (flag(32768u)) { trans *= 1.0 - s.a * 0.8; }
        next_spark++;
    }
    var alpha = 1.0 - trans;
    if (pass_ == 1u || pass_ == 5u) { col = vec3f(alpha); }
    else if (pass_ == 4u) { col = vec3f(tmax); }
    else if (pass_ == 7u) { col = vec3f(select(1.0, depth, alpha > 0.0)); }
    if (holdout) { bg = vec3f(0.0); }
    // smoothing over frames / progressive refinement (weight 1 = fresh image)
    let ai = gid.y * w + gid.x;
    var outc = vec4f(col, alpha);
    var outb = vec4f(bg, select(1.0, 0.0, holdout));
    let hw = r.sm3.z;
    if (hw < 0.999) {
        outc = mix(accum[ai], outc, hw);
        outb = mix(bgbuf[ai], outb, hw);
    }
    accum[ai] = outc;
    bgbuf[ai] = outb;
}
