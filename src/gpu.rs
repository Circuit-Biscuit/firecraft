// Runs the fluid simulation, the raymarcher and the picture finishing on the graphics card.
use crate::render::{Camera, Post, Shade};
use crate::sim::{AMBIENT_K, Domain, FPS, M3, ShapeInst, SimParams, mat_mul, particle, transpose};
use eframe::wgpu;

// vel0, sca0, sca1 (also new velocity), tmp (also curl), p0, p1, divergence, light
const BUFS: usize = 8;
const SCA0: usize = 1;
const SCA1: usize = 2;
const P0: usize = 4;
const LIGHT: usize = 7;
fn is_vec4(k: usize) -> bool {
    k < 4 || k == LIGHT
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    sim_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
    render_pipe: wgpu::ComputePipeline,
    post_pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
    noise_view: wgpu::TextureView,
    noise_sampler: wgpu::Sampler,
    /// Last simulation/render submission; the next step waits for it (see `throttle`).
    last: std::sync::Mutex<Option<wgpu::SubmissionIndex>>,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn store_tex(binding: u32, format: wgpu::TextureFormat, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: dim },
        count: None,
    }
}
fn tex3(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
        count: None,
    }
}
fn samp(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

const PASSES: [&str; 19] = [
    "gs_red", "gs_black", "sor_red", "sor_black", "pack", "emit_forces", "curl_pass", "confine", "divergence", "jacobi_a", "jacobi_b", "project",
    "advect_vel", "advect_fwd", "advect_back", "correct", "light_pass", "hourglass", "histogram",
];
const POST_PASSES: [&str; 4] = ["bloom_down", "bloom_h", "bloom_v", "finish"];

impl Gpu {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let module = |label, src: &str| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let sim_mod = module("sim", include_str!("sim.wgsl"));
        let ren_mod = module("render", include_str!("render.wgsl"));
        let post_mod = module("post", include_str!("post.wgsl"));
        let mut entries = vec![uniform(0)];
        entries.extend((1..=BUFS as u32).map(|b| storage(b, false)));
        entries.push(store_tex(9, wgpu::TextureFormat::Rgba16Float, wgpu::TextureViewDimension::D3));
        entries.push(storage(10, false)); // brick occupancy
        entries.push(store_tex(11, wgpu::TextureFormat::Rgba16Float, wgpu::TextureViewDimension::D3));
        entries.push(storage(12, false)); // histograms
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sim"), entries: &entries });
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render"),
            entries: &[uniform(0), tex3(1), samp(2), tex3(3), storage(4, false), storage(5, true), tex3(6), samp(7), storage(8, true), storage(9, false)],
        });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                store_tex(5, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureViewDimension::D2),
                storage(6, false),
            ],
        });
        let mk = |layout: &wgpu::BindGroupLayout, module: &wgpu::ShaderModule, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[layout], push_constant_ranges: &[] });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipes = PASSES.iter().map(|e| (*e, mk(&sim_layout, &sim_mod, e))).collect();
        let render_pipe = mk(&render_layout, &ren_mod, "main");
        let post_pipes = POST_PASSES.iter().map(|e| (*e, mk(&post_layout, &post_mod, e))).collect();
        // 32^3 tileable random texture for fine render detail
        const NS: u32 = 32;
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("detail noise"),
            size: wgpu::Extent3d { width: NS, height: NS, depth_or_array_layers: NS },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut seed = 0x2545_f491u32;
        let data: Vec<u8> = (0..NS * NS * NS * 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed >> 24) as u8
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &noise, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(NS * 4), rows_per_image: Some(NS) },
            wgpu::Extent3d { width: NS, height: NS, depth_or_array_layers: NS },
        );
        let noise_view = noise.create_view(&Default::default());
        let noise_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Gpu { device, queue, sim_layout, render_layout, post_layout, pipes, render_pipe, post_pipes, noise_view, noise_sampler, last: Default::default() }
    }

    fn submit(&self, enc: wgpu::CommandEncoder) {
        let i = self.queue.submit([enc.finish()]);
        *self.last.lock().unwrap() = Some(i);
    }

    /// Wait until the graphics card has finished the previous frame's work.
    /// Without this, a busy card falls behind while new work keeps arriving: the unfinished
    /// work and its uploaded data pile up in memory without limit and the app freezes.
    pub fn throttle(&self) {
        if let Some(i) = self.last.lock().unwrap().take() {
            self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(i));
        }
    }

    fn pipe(&self, name: &str) -> &wgpu::ComputePipeline {
        &self.pipes.iter().find(|(n, _)| *n == name).unwrap().1
    }
    fn post_pipe(&self, name: &str) -> &wgpu::ComputePipeline {
        &self.post_pipes.iter().find(|(n, _)| *n == name).unwrap().1
    }

    /// Blocking read of a buffer.
    pub fn read(&self, src: &wgpu::Buffer, size: u64) -> Vec<u8> {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(src, 0, &staging, 0, size);
        self.queue.submit([enc.finish()]);
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let out = slice.get_mapped_range().to_vec();
        staging.unmap();
        out
    }

    /// Blocking read of an RGBA8 target into tightly packed rows.
    pub fn read_target(&self, t: &Target) -> Vec<u8> {
        let row = (t.w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * t.h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: None } },
            wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity(t.w * t.h * 4);
        for y in 0..t.h {
            out.extend_from_slice(&data[y * row..y * row + t.w * 4]);
        }
        out
    }

    /// Linear HDR pixels (after exposure and bloom, before tonemapping), RGBA float.
    pub fn read_hdr(&self, t: &Target) -> Vec<f32> {
        self.read(&t.hdr, t.hdr.size()).chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Finish the picture in `t` (bloom, tonemap, style, alpha). `pass` 0 = beauty, else a data pass.
    pub fn post(&self, t: &Target, p: &Post, pass: u32, seed: u32) {
        let (bw, bh) = (t.w.div_ceil(4) as u32, t.h.div_ceil(4) as u32);
        let mut u = U::default();
        u.u(&[t.w as u32, t.h as u32, p.style, p.tonemap])
            .f(&[p.gamma, p.exposure, p.vibrance, p.dither])
            .f(&[p.contrast, p.brightness, p.saturation, p.bloom])
            .f(&[p.bloom_radius * bh as f32 * 0.25, p.swizzle as f32, p.style_size, p.remap_style as f32])
            .f(&p.vignette)
            .f(&p.quant)
            .f(&[p.quant_dither, p.dither_mode as f32, p.alpha_limit, p.alpha_sharpen])
            .u(&[p.alpha_mode, p.invert_alpha as u32, bw, bh])
            .u(&[seed, pass, 0, 0]);
        self.queue.write_buffer(&t.post_uni, 0, &u.0);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.post_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: t.post_uni.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: t.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: t.bg.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: t.bloom_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: t.bloom_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.view) },
                wgpu::BindGroupEntry { binding: 6, resource: t.hdr.as_entire_binding() },
            ],
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass_ = enc.begin_compute_pass(&Default::default());
            pass_.set_bind_group(0, &bind, &[]);
            if pass == 0 && p.bloom > 0.0 {
                for name in ["bloom_down", "bloom_h", "bloom_v"] {
                    pass_.set_pipeline(self.post_pipe(name));
                    pass_.dispatch_workgroups(bw.div_ceil(8), bh.div_ceil(8), 1);
                }
            }
            pass_.set_pipeline(self.post_pipe("finish"));
            pass_.dispatch_workgroups((t.w as u32).div_ceil(8), (t.h as u32).div_ceil(8), 1);
        }
        if pass == 0 && p.bloom <= 0.0 {
            enc.clear_buffer(&t.bloom_a, 0, None);
        }
        self.submit(enc);
    }
}

/// Render target: buffers for the raymarcher and the finished picture.
pub struct Target {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// sRGB view of the same pixels, for display (values are stored gamma-encoded).
    pub srgb_view: wgpu::TextureView,
    accum: wgpu::Buffer,
    bg: wgpu::Buffer,
    bloom_a: wgpu::Buffer,
    bloom_b: wgpu::Buffer,
    hdr: wgpu::Buffer,
    post_uni: wgpu::Buffer,
    pub w: usize,
    pub h: usize,
}

impl Target {
    pub fn new(gpu: &Gpu, w: usize, h: usize) -> Self {
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("view"),
            size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        let view = tex.create_view(&Default::default());
        let srgb_view = tex.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8UnormSrgb), usage: Some(wgpu::TextureUsages::TEXTURE_BINDING), ..Default::default() });
        let buf = |label, size: usize, extra: wgpu::BufferUsages| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16) as u64, usage: wgpu::BufferUsages::STORAGE | extra, mapped_at_creation: false })
        };
        let bsz = w.div_ceil(4) * h.div_ceil(4) * 16;
        Target {
            tex,
            view,
            srgb_view,
            accum: buf("accum", w * h * 16, wgpu::BufferUsages::empty()),
            bg: buf("bg", w * h * 16, wgpu::BufferUsages::empty()),
            bloom_a: buf("bloom a", bsz, wgpu::BufferUsages::COPY_DST),
            bloom_b: buf("bloom b", bsz, wgpu::BufferUsages::empty()),
            hdr: buf("hdr", w * h * 16, wgpu::BufferUsages::COPY_SRC),
            post_uni: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("post"), size: 16 * 9, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            w,
            h,
        }
    }
}

/// Little-endian uniform writer (every field is 4 bytes, grouped as vec4s).
#[derive(Default)]
struct U(Vec<u8>);
impl U {
    fn f(&mut self, v: &[f32]) -> &mut Self {
        v.iter().for_each(|x| self.0.extend(x.to_le_bytes()));
        self
    }
    fn u(&mut self, v: &[u32]) -> &mut Self {
        v.iter().for_each(|x| self.0.extend(x.to_le_bytes()));
        self
    }
    /// world -> local rotation rows (transpose of the local -> world matrix)
    fn rot(&mut self, m: &M3) -> &mut Self {
        let t = transpose(m);
        for row in t {
            self.f(&[row[0], row[1], row[2], 0.0]);
        }
        self
    }
}

pub struct GpuSim {
    pub dom: Domain,
    pub n: [usize; 3],
    pub h: f32,
    pub frame: u32,
    bufs: Vec<wgpu::Buffer>,
    vol_view: wgpu::TextureView,
    vol2_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    occ: wgpu::Buffer,
    hist: wgpu::Buffer,
    lut: wgpu::Buffer,
    sim_uni: wgpu::Buffer,
    ren_uni: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// Glowing particles from the last step (position, radius, brightness), for the renderer.
    pub sparks: Vec<([f32; 3], f32, f32)>,
}

const MAX_EMIT: usize = 32;
const MAX_FORCES: usize = 8;
const MAX_COLL: usize = 8;
const MAX_SPARKS: usize = 32;
const HEAD: u64 = 24; // vec4s before the arrays
const RENDER_TIME_AT: u64 = 18; // first vec4 written at render time (light, sharpening, post)
const SIM_UNI: u64 = 16 * HEAD + 176 * MAX_EMIT as u64 + 208 * MAX_FORCES as u64 + 32 * MAX_COLL as u64;
const REN_UNI: u64 = 16 * (30 + 8 + 48 + 64);

impl GpuSim {
    pub fn new(gpu: &Gpu, dom: Domain) -> Self {
        let n = dom.n;
        let cells = (n[0] * n[1] * n[2]) as u64;
        let bufs: Vec<wgpu::Buffer> = (0..BUFS)
            .map(|k| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: cells * if is_vec4(k) { 16 } else { 4 },
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let mk_buf = |label, size, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let sim_uni = mk_buf("sim", SIM_UNI, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let ren_uni = mk_buf("render", REN_UNI, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let lut = mk_buf("lut", 256 * 16, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let hist = mk_buf("hist", 128 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let mk_vol = || {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("volume"),
                size: wgpu::Extent3d { width: n[0] as u32, height: n[1] as u32, depth_or_array_layers: n[2] as u32 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let vol_view = mk_vol().create_view(&Default::default());
        let vol2_view = mk_vol().create_view(&Default::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let bricks = (n[0].div_ceil(8) * n[1].div_ceil(8) * n[2].div_ceil(8)) as u64;
        let occ = mk_buf("occupancy", bricks * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: sim_uni.as_entire_binding() }];
        entries.extend(bufs.iter().enumerate().map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32 + 1, resource: b.as_entire_binding() }));
        entries.push(wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&vol_view) });
        entries.push(wgpu::BindGroupEntry { binding: 10, resource: occ.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&vol2_view) });
        entries.push(wgpu::BindGroupEntry { binding: 12, resource: hist.as_entire_binding() });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &gpu.sim_layout, entries: &entries });
        GpuSim { dom, n, h: dom.h, frame: 0, bufs, vol_view, vol2_view, sampler, occ, hist, lut, sim_uni, ren_uni, bind, sparks: vec![] }
    }

    pub fn origin(&self) -> [f32; 3] {
        self.dom.origin
    }
    pub fn size(&self) -> [f32; 3] {
        [self.n[0] as f32 * self.h, self.n[1] as f32 * self.h, self.n[2] as f32 * self.h]
    }

    fn groups(&self) -> (u32, u32, u32) {
        (self.n[0].div_ceil(4) as u32, self.n[1].div_ceil(4) as u32, self.n[2].div_ceil(4) as u32)
    }

    fn dispatch(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, names: &[&str]) {
        let (x, y, z) = self.groups();
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0, &self.bind, &[]);
        for name in names {
            pass.set_pipeline(gpu.pipe(name));
            pass.dispatch_workgroups(x, y, z);
        }
    }

    /// One frame; split into `p.substeps` smaller steps so gas never jumps too many cells at once.
    pub fn step(&mut self, gpu: &Gpu, p: &SimParams) {
        gpu.throttle();
        if p.freeze {
            self.frame += 1;
            return;
        }
        let n = p.substeps.max(1);
        let mut sp = p.clone();
        sp.time_scale = p.time_scale / n as f32;
        for k in 0..n {
            self.substep(gpu, &sp, k + 1 == n);
        }
    }

    fn substep(&mut self, gpu: &Gpu, p: &SimParams, last: bool) {
        let t = self.frame as f32 / FPS;
        let o = self.origin();
        // velocities are in cells/frame: same physical speed on any grid or domain size
        let scale = 0.03125 / self.h;
        let h = self.h;
        let cells = |m: [f32; 3]| [(m[0] - o[0]) / h, (m[1] - o[1]) / h, (m[2] - o[2]) / h];
        // expand emitters into shape entries at this moment (bursts grow, particles fly)
        let mut entries: Vec<U> = vec![];
        let mut sparks = vec![];
        for e in &p.emitters {
            if e.burst > 0.0 {
                let phase = if e.gap > 0.0 { t % (e.burst + e.gap) } else { t };
                if phase > e.burst {
                    continue;
                }
            }
            let rates = [e.fuel / 100.0 * 0.05, e.smoke / 100.0 * 0.05, (e.temp_k - AMBIENT_K) / 1000.0, e.gradient];
            let mut push = |s: &ShapeInst, center: [f32; 3], half: [f32; 3], hollow: f32, vel: [f32; 3], fade: f32| {
                let c = cells(center);
                let ch = s.noise.clone().or_else(|| e.chaos.clone()).map_or([0.0; 3], |c| [c.strength, c.scale * h, c.speed]);
                let pn = e.pressure_noise;
                let mut u = U::default();
                u.f(&[c[0], c[1], c[2], e.shadow_caster as u8 as f32])
                    .f(&[(half[0] / h).max(0.6), (half[1] / h).max(0.6), (half[2] / h).max(0.6), s.torus])
                    .f(&[rates[0], rates[1], rates[2], s.gradient.unwrap_or(rates[3])])
                    .f(&[vel[0] * scale, vel[1] * scale, vel[2] * scale, e.spread])
                    .f(&[s.kind.id(), e.fuel_add as u8 as f32, e.smoke_add as u8 as f32, e.temp_replace as u8 as f32])
                    .f(&[ch[0], ch[1], ch[2], e.expansion])
                    .f(&[hollow, fade, pn[0], h / pn[1].max(0.005)])
                    .f(&[pn[2], pn[3], 0.0, 0.0])
                    .rot(&s.rot);
                entries.push(u);
            };
            match &e.particles {
                Some(pp) => {
                    let origin = e.shapes.first().map_or([0.0; 3], |s| s.center);
                    for k in 0..pp.count {
                        let (mut pos, vel, life) = particle(pp, origin, k, t);
                        // noise forces set to act on particles push them around
                        for f in p.forces.iter().filter(|f| f.kind == 3 && f.mode != 0) {
                            let np = crate::noise::NoiseParams { freq: f.scale, octaves: f.noise[0] as u32, lacunarity: f.noise[1], gain: f.noise[2], curl: f.noise[3] > 0.5, seed: f.seed, speed: f.speed };
                            let d = crate::noise::fbm(&np, pos, t);
                            let age = life * pp.lifetime;
                            for a in 0..3 {
                                pos[a] += d[a] * f.strength * 0.05 * age * age;
                            }
                        }
                        let fade = (1.0 - life).max(0.0);
                        let s = ShapeInst::sphere(pos, pp.size);
                        push(&s, pos, [pp.size; 3], 0.0, vel, fade);
                        if pp.sparks {
                            sparks.push((pos, pp.size * 0.6, pp.brightness * fade));
                        }
                    }
                }
                None => {
                    for s in &e.shapes {
                        let mut center = s.center;
                        if s.wander > 0.0 {
                            for (k, c) in center.iter_mut().enumerate() {
                                let f = 0.7 + 0.3 * k as f32;
                                *c += s.wander * 0.3 * ((t * f + s.seed * (k as f32 + 1.3)).sin() + 0.5 * (t * f * 2.3 + s.seed).cos());
                            }
                        }
                        match &s.burst {
                            Some(b) => {
                                let local = t - b.start;
                                if local < 0.0 || local > b.duration {
                                    continue;
                                }
                                let x = (local / b.duration.max(0.01)).min(1.0);
                                let r = b.radius * (1.0 - (1.0 - x) * (1.0 - x)).max(0.05);
                                push(s, center, [r; 3], b.thickness.clamp(0.05, 1.0), e.vel, 1.0);
                            }
                            None => push(s, center, s.half, s.hollow, e.vel, 1.0),
                        }
                    }
                }
            }
        }
        entries.truncate(MAX_EMIT);
        if last {
            sparks.truncate(MAX_SPARKS);
            self.sparks = sparks;
        }
        let forces: Vec<_> = p.forces.iter().take(MAX_FORCES).collect();
        let colls: Vec<_> = p.colliders.iter().take(MAX_COLL).collect();
        let kk = |x: f32| (x - AMBIENT_K) / 1000.0;
        let wl = (p.wind_dir[0] * p.wind_dir[0] + p.wind_dir[1] * p.wind_dir[1] + p.wind_dir[2] * p.wind_dir[2]).sqrt();
        // wind direction wanders with its randomisation, speed scales the final length
        let wn = |a: f32| (crate::noise::vnoise([t * 0.25 + a, a * 3.1, 0.0]) - 0.5) * 2.0;
        let wdir = [p.wind_dir[0] + p.wind_rand[0] * wn(1.0), p.wind_dir[1] + p.wind_rand[1] * wn(7.0), p.wind_dir[2] + p.wind_rand[2] * wn(13.0)];
        let wgoal = if wl > 1e-6 { wdir.map(|x| x * p.wind_speed * scale * 0.53) } else { [0.0; 3] };
        let adv = p.adv_offset.map(|x| x * scale * 0.53);
        let mut u = U::default();
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, entries.len() as u32])
            .f(&[p.time_scale, p.buoyancy, p.smoke_weight, p.fuel_weight])
            .f(&[p.burn_rate, kk(p.ignition_k), p.heat, p.smoke_yield])
            .f(&[p.flames_yield, p.expansion, p.ignition_soft_k / 1000.0, kk(p.max_burn_k)])
            .f(&[p.diss[0], p.diss[1], p.diss[2], p.diss[3]])
            .f(&[p.diss[4], p.sub[0], p.sub[1], p.sub[2]])
            .f(&[p.diffusion[0], p.diffusion[1], p.diffusion[2], p.diffusion[3]])
            .f(&[forces.len() as f32, colls.len() as f32, p.mode as f32, t])
            .f(&[self.h, scale, p.diffusion[4], p.tightness])
            .f(&[p.vorticity, p.vorticity_large, p.vort_ramp, 0.0])
            .f(&p.vort_masks)
            .f(&[kk(p.vort_ranges[0]), kk(p.vort_ranges[1]), p.vort_ranges[2], p.vort_ranges[3]])
            .f(&[wgoal[0], wgoal[1], wgoal[2], p.wind_chaos * scale * 0.53])
            .f(&[p.shred[0], kk(p.shred[1]), p.shred[2] / 1000.0, 0.0])
            .f(&[p.mask[0], p.mask[1], p.mask[2], p.walls as f32])
            .f(&[adv[0], adv[1], adv[2], p.order as f32])
            .f(&[p.vel_cubic as u8 as f32, p.vox_cubic as u8 as f32, p.leak_guard as u8 as f32, 0.0])
            .f(&[p.hourglass, p.collider_removal, p.omega, 0.0]);
        // render-time block (light, sharpening, post) keeps its last values: pad here
        u.f(&[0.0; 24]);
        debug_assert_eq!(u.0.len() as u64, 16 * HEAD);
        for k in 0..MAX_EMIT {
            match entries.get(k) {
                Some(e) => {
                    u.0.extend(&e.0);
                }
                None => {
                    u.f(&[0.0; 44]);
                }
            }
        }
        for k in 0..MAX_FORCES {
            match forces.get(k) {
                Some(fo) => {
                    let c = cells(fo.pos);
                    // vorticles: scale is a vortex size (m); toroidal: extra is a tube radius (m)
                    let sc = if fo.kind == 5 { fo.scale / h } else { fo.scale };
                    let ex = if fo.kind == 2 { fo.extra / h } else { fo.extra };
                    let l = (fo.dir[0] * fo.dir[0] + fo.dir[1] * fo.dir[1] + fo.dir[2] * fo.dir[2]).sqrt().max(1e-4);
                    u.f(&[fo.kind as f32, fo.strength, sc, fo.speed]).f(&[c[0], c[1], c[2], fo.radius / h]).f(&[fo.dir[0] / l, fo.dir[1] / l, fo.dir[2] / l, ex]);
                    let volume = (fo.mode != 1) as u8 as f32;
                    match &fo.mask {
                        Some(m) => {
                            let mc = cells(m.center);
                            u.f(&[mc[0], mc[1], mc[2], 1.0]).f(&[m.half[0] / h, m.half[1] / h, m.half[2] / h, m.kind.id()]).f(&[m.torus, fo.softness, volume, 0.0]);
                        }
                        None => {
                            u.f(&[0.0; 8]).f(&[0.0, 0.0, volume, 0.0]);
                        }
                    }
                    u.f(&fo.masks)
                        .f(&[kk(fo.ranges[0]), kk(fo.ranges[1]), fo.ranges[2], fo.ranges[3]])
                        .f(&fo.noise)
                        .f(&[fo.bias[0], fo.bias[1], fo.bias[2], fo.seed])
                        .rot(&fo.rot);
                }
                None => {
                    u.f(&[0.0; 52]);
                }
            }
        }
        for k in 0..MAX_COLL {
            match colls.get(k) {
                Some(cs) => {
                    let c = cells(cs.center);
                    u.f(&[c[0], c[1], c[2], cs.kind.id()]).f(&[cs.half[0] / h, cs.half[1] / h, cs.half[2] / h, cs.torus]);
                }
                None => {
                    u.f(&[0.0; 8]);
                }
            }
        }
        // the render-time block must not be overwritten: write around it
        let head_end = (16 * RENDER_TIME_AT) as usize;
        let block_end = (16 * HEAD) as usize;
        gpu.queue.write_buffer(&self.sim_uni, 0, &u.0[..head_end]);
        gpu.queue.write_buffer(&self.sim_uni, block_end as u64, &u.0[block_end..]);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.dispatch(gpu, &mut enc, &["emit_forces", "curl_pass", "confine", "divergence"]);
        if !p.reuse_pressure {
            enc.clear_buffer(&self.bufs[P0], 0, None);
        }
        // over-relaxed sweeps settle the large-scale pressure fast, then plain sweeps smooth
        // out the cell-sized leftovers that otherwise show up as grain
        let n = p.iterations.max(2);
        let n_sor = ((n as f32) * p.sor_share) as usize;
        let jac: Vec<&str> = (0..n).flat_map(|k| if k < n_sor { ["sor_red", "sor_black"] } else { ["gs_red", "gs_black"] }).collect();
        self.dispatch(gpu, &mut enc, &jac);
        if p.hourglass > 0.0 {
            self.dispatch(gpu, &mut enc, &["hourglass"]);
            let ps = self.bufs[P0].size();
            enc.copy_buffer_to_buffer(&self.bufs[P0 + 1], 0, &self.bufs[P0], 0, ps);
        }
        self.dispatch(gpu, &mut enc, &["project", "advect_vel"]);
        let vs = self.bufs[0].size();
        enc.copy_buffer_to_buffer(&self.bufs[SCA1], 0, &self.bufs[0], 0, vs);
        self.dispatch(gpu, &mut enc, &["advect_fwd", "advect_back", "correct"]);
        enc.copy_buffer_to_buffer(&self.bufs[SCA1], 0, &self.bufs[SCA0], 0, vs);
        gpu.submit(enc);
        if last {
            self.frame += 1;
        }
    }

    /// Replace the scalars (temperature kK, smoke, fuel, flames) with given values (Import).
    pub fn upload_scalars(&self, gpu: &Gpu, data: &[[f32; 4]]) {
        let bytes: Vec<u8> = data.iter().flat_map(|c| c.iter().flat_map(|v| v.to_le_bytes())).collect();
        gpu.queue.write_buffer(&self.bufs[SCA0], 0, &bytes);
        gpu.queue.write_buffer(&self.bufs[SCA1], 0, &bytes);
    }

    /// Scalars per cell: (temperature kK, smoke, fuel, flames).
    pub fn read_scalars(&self, gpu: &Gpu) -> Vec<[f32; 4]> {
        let raw = gpu.read(&self.bufs[SCA0], self.bufs[SCA0].size());
        raw.chunks_exact(16)
            .map(|c| {
                let f = |k: usize| f32::from_le_bytes([c[k * 4], c[k * 4 + 1], c[k * 4 + 2], c[k * 4 + 3]]);
                [f(0), f(1), f(2), f(3)]
            })
            .collect()
    }

    /// Temperature (0..10000 K) and smoke (0..2) histograms, 64 bins each.
    pub fn histograms(&self, gpu: &Gpu) -> (Vec<f32>, Vec<f32>) {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.clear_buffer(&self.hist, 0, None);
        {
            let (x, y, z) = (self.n[0].div_ceil(8) as u32, self.n[1].div_ceil(8) as u32, self.n[2].div_ceil(8) as u32);
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_pipeline(gpu.pipe("histogram"));
            pass.dispatch_workgroups(x, y, z);
        }
        gpu.queue.submit([enc.finish()]);
        let raw = gpu.read(&self.hist, 512);
        let v: Vec<f32> = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32).collect();
        (v[..64].to_vec(), v[64..].to_vec())
    }

    /// Raymarch the volume into `target`, then finish the picture.
    /// `seed` varies the sample offsets; `history` is the weight of this frame against the
    /// previous image (1 = fresh image, 0.5 = smooth over frames, 1/n = progressive refinement).
    pub fn render(&self, gpu: &Gpu, target: &Target, cam: &Camera, s: &Shade, post: &Post, seed: u32, history: f32) {
        let sun = s.sun.filter(|_| s.lit);
        // render-time solver settings: light, glow, sharpening, post processing
        {
            let mut u = U::default();
            let (dir, k) = match sun {
                Some((_, d)) => (d, s.smoke_scale * 0.9 * s.shadow_density * 1.5),
                None => ([0.0, 1.0, 0.0], 0.0),
            };
            let stepc = (self.n[0] as f32 / 40.0).max(1.5) * (1.75 - s.shadow_sharpness);
            let steps = s.shadow_steps * s.shadow_reach;
            u.f(&[dir[0], dir[1], dir[2], k])
                .f(&[stepc.max(0.75), steps, (s.ao > 0.0) as u8 as f32, if s.flames_light > 0.0 { s.scatter_radius / self.h } else { 0.0 }])
                .f(&[(s.remap[0] - AMBIENT_K) / 1000.0, (s.remap[1] - AMBIENT_K) / 1000.0, s.remap_ramp, s.scatter_occlusion]);
            match s.sharpen {
                Some(a) => u.f(&[a[0], a[1], a[2], 1.0]),
                None => u.f(&[0.0; 4]),
            };
            u.f(&[s.post[0], s.post[1], s.post[2], s.post[3] * self.h]).f(&[s.post[4], s.post[5], 0.0, 0.0]);
            gpu.queue.write_buffer(&self.sim_uni, 16 * RENDER_TIME_AT, &u.0);
        }
        let lut: Vec<u8> = s.lut.iter().flat_map(|c| [c[0], c[1], c[2], 1.0]).flat_map(|v| v.to_le_bytes()).collect();
        gpu.queue.write_buffer(&self.lut, 0, &lut);
        let (eye, f, r, up) = cam.basis();
        let tanf = (cam.fov.to_radians() * 0.5).tan();
        let mut flags = s.lit as u32 | (cam.ortho as u32) << 1 | (s.sky.is_some() as u32) << 2 | (s.ground.is_some() as u32) << 3 | (s.sun.is_some() as u32) << 4;
        let bits = [
            (s.render_flames, 32),
            (s.render_smoke, 64),
            (s.render_fuel, 128),
            (s.clamp_remap, 256),
            (s.flames_translucent, 512),
            (s.emask[0] > 0.5, 1024),
            (s.smoke_by_temp, 2048),
            (s.render_volume, 4096),
            (s.render_particles, 8192),
            (s.volume_occludes_particles, 16384),
            (s.particles_occlude_volume, 32768),
        ];
        for (on, b) in bits {
            if on {
                flags |= b;
            }
        }
        let (o, size) = (self.origin(), self.size());
        let sky = s.sky.unwrap_or(([0.0; 3], [0.0; 3]));
        let ground = s.ground.unwrap_or(([0.0; 3], 0.0));
        let (sun_c, sun_d) = s.sun.unwrap_or(([0.0; 3], [0.0, 1.0, 0.0]));
        let sl = s.slicing;
        let mut u = U::default();
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, seed])
            .u(&[target.w as u32, target.h as u32, flags, s.pass])
            .f(&[eye[0], eye[1], eye[2], tanf])
            .f(&[f[0], f[1], f[2], target.w as f32 / target.h as f32])
            .f(&[r[0], r[1], r[2], cam.dist * tanf])
            .f(&[up[0], up[1], up[2], self.frame as f32 / FPS])
            .f(&[o[0], o[1], o[2], self.h])
            .f(&[size[0], size[1], size[2], s.step])
            .f(&[s.ambient[0], s.ambient[1], s.ambient[2], s.ao])
            .f(&[sun_c[0], sun_c[1], sun_c[2], s.anisotropy * 0.9])
            .f(&[sun_d[0], sun_d[1], sun_d[2], s.direct])
            .f(&[s.extinction[0], s.extinction[1], s.extinction[2], s.shadow_intensity])
            .f(&[s.remap[0], s.remap[1], s.remap_ramp, s.coloring as f32])
            .f(&[s.shaping_flames, s.shaping_density, s.flames_min, s.flames_scale])
            .f(&[s.fire_color[0], s.fire_color[1], s.fire_color[2], s.flames_opacity])
            .f(&[s.smoke_color[0], s.smoke_color[1], s.smoke_color[2], s.smoke_scale])
            .f(&[s.smoke_remap[0], s.smoke_remap[1], s.smoke_remap_ramp, s.smoke_coloring as f32])
            .f(&[s.smoke_min, s.smoke_clamp, history, ground.1])
            .f(&[s.fuel_color[0], s.fuel_color[1], s.fuel_color[2], s.fuel_scale])
            .f(&[s.flames_light, s.particles_light, 0.0, s.detail])
            .f(&[s.emask[1], s.emask[2], s.emask[3], s.emask[4]])
            .f(&[s.emask[5], s.emask[6], s.emask[7], s.fast_detail as u8 as f32])
            .f(&[sl[0], sl[1], sl[2], sl[3]])
            .f(&[sl[4], sl[5], sl[6], sl[7]])
            .f(&[sl[8], sl[9], sl[10], sl[11]])
            .f(&[sl[12], 0.0, 0.0, 0.0])
            .f(&[sky.0[0], sky.0[1], sky.0[2], 0.0])
            .f(&[sky.1[0], sky.1[1], sky.1[2], 0.0])
            .f(&[ground.0[0], ground.0[1], ground.0[2], 0.0]);
        let lights: Vec<_> = s.points.iter().take(4).collect();
        let shapes: Vec<_> = s.shapes.iter().take(8).collect();
        let group = s.spark_group.max(1) as usize;
        let sparks: Vec<_> = if s.sparks { self.sparks.iter().step_by(group).take(MAX_SPARKS).collect() } else { vec![] };
        let shape_flags = s.shape_mode[0] | s.shape_mode[1] << 2 | (s.shape_mode[2] << 4) | (s.sparks_hq as u32) << 5;
        u.u(&[lights.len() as u32, shapes.len() as u32, sparks.len() as u32, shape_flags]);
        for k in 0..4 {
            match lights.get(k) {
                Some((p, c, range)) => u.f(&[p[0], p[1], p[2], *range]).f(&[c[0], c[1], c[2], 0.0]),
                None => u.f(&[0.0; 8]),
            };
        }
        for k in 0..8 {
            match shapes.get(k) {
                Some((sh, c)) => {
                    u.f(&[sh.center[0], sh.center[1], sh.center[2], sh.kind.id()]).f(&[sh.half[0], sh.half[1], sh.half[2], sh.torus]).f(&[c[0], c[1], c[2], 0.0]).rot(&sh.rot);
                }
                None => {
                    u.f(&[0.0; 24]);
                }
            }
        }
        let hot = s.lut.last().copied().unwrap_or([1.0, 0.7, 0.3]);
        let boost = (group as f32).sqrt();
        for k in 0..MAX_SPARKS {
            match sparks.get(k) {
                Some((p, r, _)) => u.f(&[p[0], p[1], p[2], *r * boost]),
                None => u.f(&[0.0; 4]),
            };
        }
        for k in 0..MAX_SPARKS {
            match sparks.get(k) {
                Some((_, _, b)) => u.f(&[hot[0] * b, hot[1] * b, hot[2] * b, 0.0]),
                None => u.f(&[0.0; 4]),
            };
        }
        debug_assert_eq!(u.0.len() as u64, REN_UNI);
        gpu.queue.write_buffer(&self.ren_uni, 0, &u.0);
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.render_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ren_uni.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.vol_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.vol2_view) },
                wgpu::BindGroupEntry { binding: 4, resource: target.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.occ.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&gpu.noise_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&gpu.noise_sampler) },
                wgpu::BindGroupEntry { binding: 8, resource: self.lut.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: target.bg.as_entire_binding() },
            ],
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.dispatch(gpu, &mut enc, &["light_pass"]);
        enc.clear_buffer(&self.occ, 0, None);
        self.dispatch(gpu, &mut enc, &["pack"]);
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&gpu.render_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(target.w.div_ceil(8) as u32, target.h.div_ceil(8) as u32, 1);
        }
        gpu.submit(enc);
        gpu.post(target, post, if s.pass == 0 { 0 } else { s.pass }, seed);
    }
}

/// Light direction (towards the light) rotated by a matrix: used by gizmos.
#[allow(dead_code)]
pub fn rotate(m: &M3, v: [f32; 3]) -> [f32; 3] {
    mat_mul(m, v)
}

#[cfg(test)]
pub fn test_gpu() -> Gpu {
    let instance = wgpu::Instance::default();
    let opts = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = block_on(instance.request_adapter(&opts)).expect("no GPU adapter");
    let desc = wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() };
    let (device, queue) = block_on(adapter.request_device(&desc, None)).expect("no device");
    Gpu::new(device, queue)
}

#[cfg(test)]
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}
