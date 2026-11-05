// Picture finishing (Scene node + Render settings): bloom, tonemapping, grading, vignette,
// rendering styles, quantization and alpha. Reads the raymarcher's accum/bg buffers.

struct P {
    img: vec4u,  // width, height, style, tonemapper
    a: vec4f,    // gamma, exposure, vibrance, tonemap dithering
    b: vec4f,    // contrast, brightness, saturation, bloom intensity
    c: vec4f,    // bloom radius (bloom px), swizzle, style size (px), remapping style
    vg: vec4f,   // vignette width px, offset px, roundness, intensity
    q: vec4f,    // quantization levels: rgb, hue, value, saturation
    q2: vec4f,   // quantization dither, dither mode, translucency limit, translucency sharpening
    al: vec4u,   // alpha mode (0 premultiplied 1 straight 2 opaque), invert alpha, bloom width, bloom height
    fr: vec4u,   // seed, pass, _, _
}

@group(0) @binding(0) var<uniform> pp: P;
@group(0) @binding(1) var<storage, read> accum: array<vec4f>;
@group(0) @binding(2) var<storage, read> bgbuf: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> bloom_a: array<vec4f>;
@group(0) @binding(4) var<storage, read_write> bloom_b: array<vec4f>;
@group(0) @binding(5) var out_img: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var<storage, read_write> hdr: array<vec4f>;

fn opaque() -> bool { return pp.al.x == 2u; }

/// Linear colour of a pixel: volume over background (opaque) or the volume alone (with alpha).
fn src(x: i32, y: i32) -> vec4f {
    let w = i32(pp.img.x);
    let h = i32(pp.img.y);
    let i = u32(clamp(y, 0, h - 1) * w + clamp(x, 0, w - 1));
    let v = accum[i];
    if (opaque()) { return vec4f(v.xyz + bgbuf[i].xyz * (1.0 - v.w), 1.0); }
    return v;
}

fn lum(c: vec3f) -> f32 { return dot(c, vec3f(0.2126, 0.7152, 0.0722)); }

// ---------- bloom ----------

@compute @workgroup_size(8, 8, 1)
fn bloom_down(@builtin(global_invocation_id) g: vec3u) {
    let bw = pp.al.z;
    let bh = pp.al.w;
    if (g.x >= bw || g.y >= bh) { return; }
    var acc = vec3f(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = src(i32(g.x) * 4 + i, i32(g.y) * 4 + j).xyz * pp.a.y;
            acc += c * smoothstep(0.5, 2.0, lum(c));
        }
    }
    bloom_a[g.y * bw + g.x] = vec4f(acc / 16.0, 1.0);
}

fn blur(g: vec3u, horizontal: bool) {
    let bw = i32(pp.al.z);
    let bh = i32(pp.al.w);
    if (i32(g.x) >= bw || i32(g.y) >= bh) { return; }
    let sigma = max(pp.c.x, 0.5);
    let stride = max(sigma / 6.0, 1.0);
    var acc = vec3f(0.0);
    var wsum = 0.0;
    for (var k = -12; k <= 12; k++) {
        let off = f32(k) * stride;
        let wgt = exp(-off * off / (2.0 * sigma * sigma));
        var x = i32(g.x);
        var y = i32(g.y);
        if (horizontal) { x = clamp(x + i32(round(off)), 0, bw - 1); } else { y = clamp(y + i32(round(off)), 0, bh - 1); }
        let v = select(bloom_b[y * bw + x].xyz, bloom_a[y * bw + x].xyz, horizontal);
        acc += v * wgt;
        wsum += wgt;
    }
    let o = u32(i32(g.y) * bw + i32(g.x));
    if (horizontal) { bloom_b[o] = vec4f(acc / wsum, 1.0); } else { bloom_a[o] = vec4f(acc / wsum, 1.0); }
}
@compute @workgroup_size(8, 8, 1)
fn bloom_h(@builtin(global_invocation_id) g: vec3u) { blur(g, true); }
@compute @workgroup_size(8, 8, 1)
fn bloom_v(@builtin(global_invocation_id) g: vec3u) { blur(g, false); }

fn bloom_at(x: f32, y: f32) -> vec3f {
    let bw = i32(pp.al.z);
    let bh = i32(pp.al.w);
    let fx = clamp(x / 4.0 - 0.5, 0.0, f32(bw - 1));
    let fy = clamp(y / 4.0 - 0.5, 0.0, f32(bh - 1));
    let x0 = i32(fx);
    let y0 = i32(fy);
    let x1 = min(x0 + 1, bw - 1);
    let y1 = min(y0 + 1, bh - 1);
    let tx = fract(fx);
    let ty = fract(fy);
    let a = mix(bloom_a[y0 * bw + x0].xyz, bloom_a[y0 * bw + x1].xyz, tx);
    let b = mix(bloom_a[y1 * bw + x0].xyz, bloom_a[y1 * bw + x1].xyz, tx);
    return mix(a, b, ty);
}

// ---------- tonemappers ----------

fn aces(x: vec3f) -> vec3f {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3f(0.0), vec3f(1.0));
}
fn hable(x: vec3f) -> vec3f {
    let a = 0.15; let b = 0.5; let c = 0.1; let d = 0.2; let e = 0.02; let f = 0.3;
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}
fn filmic(x: vec3f) -> vec3f {
    let w = 11.2;
    return hable(x * 2.0) / hable(vec3f(w));
}
// Uchimura "GT" tonemapper
fn uchimura(x: vec3f) -> vec3f {
    let P = 1.0; let a = 1.0; let m = 0.22; let l = 0.4; let c = 1.33; let b = 0.0;
    let l0 = ((P - m) * l) / a;
    let S0 = m + l0;
    let S1 = m + a * l0;
    let C2 = (a * P) / (P - S1);
    let CP = -C2 / P;
    let w0 = vec3f(1.0) - smoothstep(vec3f(0.0), vec3f(m), x);
    let w2 = step(vec3f(m + l0), x);
    let w1 = vec3f(1.0) - w0 - w2;
    let T = m * pow(x / m, vec3f(c)) + b;
    let S = P - (P - S1) * exp(CP * (x - S0));
    let L = m + a * (x - m);
    return T * w0 + L * w1 + S * w2;
}
fn agx(x: vec3f) -> vec3f {
    // compact AgX approximation
    let m = mat3x3f(vec3f(0.842, 0.0423, 0.0424), vec3f(0.0784, 0.878, 0.0784), vec3f(0.0792, 0.0792, 0.879));
    var v = m * max(x, vec3f(1e-10));
    v = clamp((log2(v) + 12.47) / 16.5, vec3f(0.0), vec3f(1.0));
    let v2 = v * v;
    let v4 = v2 * v2;
    v = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v - 0.00232;
    let inv = mat3x3f(vec3f(1.196, -0.0529, -0.0530), vec3f(-0.0980, 1.152, -0.0980), vec3f(-0.0990, -0.0990, 1.151));
    return pow(clamp(inv * v, vec3f(0.0), vec3f(1.0)), vec3f(2.2));
}
fn tonemap(x: vec3f) -> vec3f {
    let t = pp.img.w;
    if (t == 0u) { return clamp(x, vec3f(0.0), vec3f(1.0)); }
    if (t == 1u) { return x / (1.0 + x); }
    if (t == 2u) { return aces(x); }
    if (t == 3u) { return clamp(filmic(x), vec3f(0.0), vec3f(1.0)); }
    if (t == 4u) { return clamp(uchimura(x), vec3f(0.0), vec3f(1.0)); }
    return agx(x);
}

// ---------- colour ----------

fn rgb2hsv(c: vec3f) -> vec3f {
    let K = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4f(c.bg, K.wz), vec4f(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + 1e-10)), d / (q.x + 1e-10), q.x);
}
fn hsv2rgb(c: vec3f) -> vec3f {
    let p = abs(fract(c.xxx + vec3f(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3f(1.0), clamp(p - 1.0, vec3f(0.0), vec3f(1.0)), c.y);
}

fn bayer(x: u32, y: u32) -> f32 {
    let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return m[(y % 4u) * 4u + (x % 4u)] / 16.0 - 0.5;
}
fn rnd(x: u32, y: u32) -> f32 {
    var h = x * 374761393u + y * 668265263u + pp.fr.x * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    return f32((h ^ (h >> 16u)) & 0xffffu) / 65535.0 - 0.5;
}
fn dither_value(x: u32, y: u32) -> f32 {
    let mode = pp.q2.y;
    if (mode < 0.5) { return bayer(x, y); }
    if (mode < 1.5) { return rnd(x, y); }
    return 0.0;
}
fn quant(v: f32, levels: f32, d: f32) -> f32 {
    if (levels < 1.5) { return v; }
    return clamp(floor(v * (levels - 1.0) + 0.5 + d) / (levels - 1.0), 0.0, 1.0);
}

// ---------- styles ----------

fn kuwahara(x: i32, y: i32, rad: i32) -> vec3f {
    var best = vec3f(0.0);
    var bestv = 1e9;
    for (var q = 0; q < 4; q++) {
        let sx = select(-rad, 0, (q & 1) == 1);
        let sy = select(-rad, 0, (q & 2) == 2);
        var m = vec3f(0.0);
        var m2 = vec3f(0.0);
        for (var j = 0; j <= rad; j++) {
            for (var i = 0; i <= rad; i++) {
                let c = src(x + sx + i, y + sy + j).xyz;
                m += c;
                m2 += c * c;
            }
        }
        let n = f32((rad + 1) * (rad + 1));
        m /= n;
        let v = m2 / n - m * m;
        let vs = v.x + v.y + v.z;
        if (vs < bestv) { bestv = vs; best = m; }
    }
    return best;
}

fn aniso_kuwahara(x: i32, y: i32, rad: i32) -> vec3f {
    // 8 sectors, weighted by how uniform each one is (generalised Kuwahara)
    var acc = vec3f(0.0);
    var wsum = 0.0;
    for (var s = 0; s < 8; s++) {
        let a0 = f32(s) * 0.785398;
        var m = vec3f(0.0);
        var m2 = vec3f(0.0);
        var n = 0.0;
        for (var rr = 1; rr <= rad; rr++) {
            for (var k = 0; k < 3; k++) {
                let a = a0 + f32(k) * 0.26;
                let c = src(x + i32(round(cos(a) * f32(rr))), y + i32(round(sin(a) * f32(rr)))).xyz;
                m += c;
                m2 += c * c;
                n += 1.0;
            }
        }
        m /= n;
        let v = m2 / n - m * m;
        let wgt = 1.0 / (1.0 + 400.0 * (v.x + v.y + v.z));
        acc += m * wgt;
        wsum += wgt;
    }
    return acc / wsum;
}

fn strokes(x: i32, y: i32, len: i32, colored: bool, contrast: bool) -> vec3f {
    // smear along the isophote (perpendicular to the brightness gradient), with a stroke texture
    let gx = lum(src(x + 1, y).xyz) - lum(src(x - 1, y).xyz);
    let gy = lum(src(x, y + 1).xyz) - lum(src(x, y - 1).xyz);
    var dir = normalize(vec2f(-gy, gx) + vec2f(1e-4, 0.0));
    var acc = vec3f(0.0);
    for (var k = -len; k <= len; k++) {
        acc += src(x + i32(round(dir.x * f32(k))), y + i32(round(dir.y * f32(k)))).xyz;
    }
    acc /= f32(2 * len + 1);
    let tex = 0.75 + 0.5 * fract(sin(dot(floor(vec2f(f32(x), f32(y)) / vec2f(1.0, f32(len))), vec2f(12.9898, 78.233))) * 43758.547);
    var c = acc * tex;
    if (!colored) { c = vec3f(lum(c)); }
    if (contrast) { c = smoothstep(vec3f(0.1), vec3f(0.6), c) * 1.4; }
    return c;
}

@compute @workgroup_size(8, 8, 1)
fn finish(@builtin(global_invocation_id) g: vec3u) {
    let w = pp.img.x;
    let h = pp.img.y;
    if (g.x >= w || g.y >= h) { return; }
    let x = i32(g.x);
    let y = i32(g.y);
    let i = g.y * w + g.x;
    let base = src(x, y);
    var a = base.w;
    if (pp.fr.y != 0u) {
        // data passes (alpha, temperature, density, depth...): no grading
        let v = accum[i];
        hdr[i] = v;
        textureStore(out_img, vec2i(g.xy), vec4f(v.xyz, select(v.w, 1.0, opaque())));
        return;
    }
    var c = base.xyz;
    let style = pp.img.z;
    let sz = i32(max(pp.c.z, 1.0));
    if (style == 1u) {
        let bx = (x / sz) * sz + sz / 2;
        let by = (y / sz) * sz + sz / 2;
        let s = src(bx, by);
        c = s.xyz;
        a = s.w;
    } else if (style == 2u) {
        let l = array<f32, 9>(lum(src(x - 1, y - 1).xyz), lum(src(x, y - 1).xyz), lum(src(x + 1, y - 1).xyz), lum(src(x - 1, y).xyz), 0.0,
            lum(src(x + 1, y).xyz), lum(src(x - 1, y + 1).xyz), lum(src(x, y + 1).xyz), lum(src(x + 1, y + 1).xyz));
        let ex = (l[2] + 2.0 * l[5] + l[8]) - (l[0] + 2.0 * l[3] + l[6]);
        let ey = (l[6] + 2.0 * l[7] + l[8]) - (l[0] + 2.0 * l[1] + l[2]);
        c = c * 0.15 + normalize(c + vec3f(1e-4)) * sqrt(ex * ex + ey * ey) * 4.0;
    } else if (style == 3u) {
        let ctr = vec2f(f32(w), f32(h)) * 0.5;
        let d = (vec2f(f32(x), f32(y)) - ctr) / f32(h);
        let o = d * dot(d, d) * f32(sz) * 8.0;
        c = vec3f(src(x + i32(o.x), y + i32(o.y)).x, c.y, src(x - i32(o.x), y - i32(o.y)).z);
    } else if (style == 4u) {
        c = kuwahara(x, y, min(sz, 8));
    } else if (style == 5u) {
        c = aniso_kuwahara(x, y, min(sz * 2, 16));
    } else if (style == 6u) {
        c = strokes(x, y, max(sz, 2), true, false);
    } else if (style == 7u) {
        c = strokes(x, y, max(sz * 3, 4), false, false);
    } else if (style == 8u) {
        c = strokes(x, y, max(sz * 2, 3), false, true);
    } else if (style == 9u) {
        c = strokes(x, y, max(sz * 3, 4), true, false);
    }
    // bloom and exposure
    c = c * pp.a.y + bloom_at(f32(x), f32(y)) * pp.b.w * 0.08;
    hdr[i] = vec4f(c, a);
    c = tonemap(c);
    // grading
    c = (c - 0.5) * (1.0 + pp.b.x) + 0.5 + pp.b.y;
    let l = lum(c);
    c = mix(vec3f(l), c, 1.0 + pp.b.z);
    let sat = max(c.x, max(c.y, c.z)) - min(c.x, min(c.y, c.z));
    c = mix(vec3f(lum(c)), c, 1.0 + pp.a.z * (1.0 - sat));
    c = max(c, vec3f(0.0));
    // remapping styles
    let rs = pp.c.w;
    if (rs > 0.5 && rs < 1.5) { c = vec3f(lum(c)); }
    else if (rs > 1.5 && rs < 2.5) { let v = lum(c); c = vec3f(v * 1.07, v * 0.9, v * 0.68); }
    else if (rs > 2.5 && rs < 3.5) { let v = clamp(lum(c), 0.0, 1.0); c = clamp(vec3f(v * 3.0 - 1.5, sin(v * 3.14159), 1.5 - v * 3.0), vec3f(0.0), vec3f(1.0)); }
    else if (rs > 3.5 && rs < 4.5) { let v = lum(c); c = vec3f(v * 0.3, v * 1.4, v * 0.3); }
    else if (rs > 4.5) { let v = lum(c); c = vec3f(v * 1.4, v * 0.35, v * 0.5); }
    // quantization
    let d = dither_value(g.x, g.y) * pp.q2.x;
    if (pp.q.x > 1.5) { c = vec3f(quant(c.x, pp.q.x, d), quant(c.y, pp.q.x, d), quant(c.z, pp.q.x, d)); }
    if (pp.q.y > 1.5 || pp.q.z > 1.5 || pp.q.w > 1.5) {
        var hsv = rgb2hsv(clamp(c, vec3f(0.0), vec3f(1.0)));
        hsv = vec3f(quant(hsv.x, pp.q.y, d), quant(hsv.y, pp.q.w, d), quant(hsv.z, pp.q.z, d));
        c = hsv2rgb(hsv);
    }
    // channel swizzle
    let sw = pp.c.y;
    if (sw > 0.5 && sw < 1.5) { c = c.xzy; } else if (sw > 1.5 && sw < 2.5) { c = c.yxz; }
    else if (sw > 2.5 && sw < 3.5) { c = c.yzx; } else if (sw > 3.5 && sw < 4.5) { c = c.zxy; } else if (sw > 4.5) { c = c.zyx; }
    // vignette
    if (pp.vg.w > 0.0) {
        let p = vec2f(f32(x), f32(y));
        let half = vec2f(f32(w), f32(h)) * 0.5;
        let q = abs(p - half) - (half - vec2f(pp.vg.y + pp.vg.x));
        let rr = pp.vg.z * pp.vg.x;
        let dist = length(max(q + vec2f(rr), vec2f(0.0))) - rr;
        let v = smoothstep(0.0, max(pp.vg.x, 1.0), dist);
        c *= 1.0 - v * pp.vg.w;
    }
    // gamma + tonemap dithering
    c = pow(clamp(c, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / pp.a.x));
    c += vec3f(rnd(g.x, g.y) * pp.a.w / 255.0);
    // alpha
    if (pp.q2.z > 0.0) {
        let wdt = (1.0 - pp.q2.w) * 0.25 + 0.001;
        a = smoothstep(pp.q2.z - wdt, pp.q2.z + wdt, a);
    }
    if (pp.al.y != 0u) { a = 1.0 - a; }
    if (pp.al.x == 1u && a > 1e-4) { c = c / a; }
    if (opaque()) { a = 1.0; }
    textureStore(out_img, vec2i(g.xy), vec4f(clamp(c, vec3f(0.0), vec3f(1.0)), a));
}
