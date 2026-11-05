// Runs the fluid simulation and the raymarcher on the graphics card.
use crate::render::{Camera, Finish, Shade};
use crate::sim::{AMBIENT_K, FPS, IDENTITY, Shape, ShapeInst, SimParams, dims, particle, transpose};
use eframe::wgpu;

// vel0, sca0, sca1 (also new velocity), tmp (also curl), p0, p1, divergence, shadow
const BUFS: usize = 8;
const VEC4_BUFS: usize = 4;
const SCA0: usize = 1;
const SCA1: usize = 2;
const SHADOW: usize = 7;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    sim_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,
    pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
    render_pipe: wgpu::ComputePipeline,
    /// Picture finishing (Scene node), run only when it differs from 0.4's ACES + gamma.
    post_layout: wgpu::BindGroupLayout,
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

const PASSES: [&str; 18] = [
    "histogram", "gs_red", "gs_black", "sor_red", "sor_black", "pack", "emit_forces", "curl_pass", "confine", "divergence", "jacobi_a", "jacobi_b", "project", "advect_vel", "advect_fwd", "advect_back", "correct", "shadow_pass",
];

impl Gpu {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let sim_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sim"), source: wgpu::ShaderSource::Wgsl(include_str!("sim.wgsl").into()) });
        let ren_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("render"), source: wgpu::ShaderSource::Wgsl(include_str!("render.wgsl").into()) });
        let mut entries = vec![uniform(0)];
        entries.extend((1..=BUFS as u32).map(|b| storage(b, false)));
        // packed volume for the renderer (hardware-filtered 3D texture)
        entries.push(storage(BUFS as u32 + 2, false)); // brick occupancy
        entries.push(storage(BUFS as u32 + 3, false)); // histograms (added after 0.4)
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: BUFS as u32 + 4, // flames volume (added after 0.4)
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: BUFS as u32 + 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        });
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sim"), entries: &entries });
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                storage(4, false),
                storage(5, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let post_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()) });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
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
        let post_pipes = ["bloom_down", "bloom_h", "bloom_v", "finish"].iter().map(|e| (*e, mk(&post_layout, &post_mod, e))).collect();
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
        Gpu { device, queue, sim_layout, render_layout, pipes, render_pipe, post_layout, post_pipes, noise_view, noise_sampler, last: Default::default() }
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

    /// Linear colour of the last render (before tonemapping), RGBA floats, for EXR export.
    pub fn read_hdr(&self, t: &Target) -> Vec<f32> {
        let raw = self.read(&t.accum, (t.w * t.h * 16) as u64);
        raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Scene node finishing over the raymarched picture: bloom, tonemapping, grading, vignette,
    /// styles. `alpha`: 0 opaque, 1 premultiplied, 2 straight.
    fn finish(&self, t: &Target, p: &Finish, alpha: u32, invert: bool, seed: u32) {
        let (bw, bh) = (t.w.div_ceil(4) as u32, t.h.div_ceil(4) as u32);
        let mut u = U::default();
        u.u(&[t.w as u32, t.h as u32, p.style, p.tonemap])
            .f(&[p.gamma, p.exposure, p.vibrance, p.dither])
            .f(&[p.contrast, p.brightness, p.saturation, p.bloom])
            .f(&[p.bloom_radius * bh as f32 * 0.25, p.swizzle as f32, p.style_size, p.remap_style as f32])
            .f(&p.vignette)
            .f(&p.quant)
            .f(&[p.quant_dither, p.dither_mode as f32, p.translucency_limit, p.translucency_sharpen])
            .u(&[[2, 0, 1][alpha.min(2) as usize], invert as u32, bw, bh])
            .u(&[seed, 0, 0, 0]);
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
            if p.bloom > 0.0 {
                for name in ["bloom_down", "bloom_h", "bloom_v"] {
                    pass_.set_pipeline(self.post_pipe(name));
                    pass_.dispatch_workgroups(bw.div_ceil(8), bh.div_ceil(8), 1);
                }
            }
            pass_.set_pipeline(self.post_pipe("finish"));
            pass_.dispatch_workgroups((t.w as u32).div_ceil(8), (t.h as u32).div_ceil(8), 1);
        }
        if p.bloom <= 0.0 {
            enc.clear_buffer(&t.bloom_a, 0, None);
        }
        self.submit(enc);
    }

    /// Blocking read of a buffer (used for exports).
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
}

/// Render target: a storage texture the raymarcher writes into.
pub struct Target {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// sRGB view of the same pixels, for display (values are stored gamma-encoded).
    pub srgb_view: wgpu::TextureView,
    /// Running average of linear colour, for HQ refinement.
    accum: wgpu::Buffer,
    // picture finishing (added after 0.4)
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
        let accum = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("accum"), size: (w * h * 16) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let buf = |label, size: usize, extra| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16) as u64, usage: wgpu::BufferUsages::STORAGE | extra, mapped_at_creation: false });
        let bsz = w.div_ceil(4) * h.div_ceil(4) * 16;
        Target {
            tex,
            view,
            srgb_view,
            accum,
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
}

pub struct GpuSim {
    pub res: usize,
    pub n: [usize; 3],
    pub h: f32,
    pub frame: u32,
    bufs: Vec<wgpu::Buffer>,
    vol_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    occ: wgpu::Buffer,
    hist: wgpu::Buffer,
    vol2_view: wgpu::TextureView,
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
const EMIT_F: usize = 52; // floats per emitter entry (13 vec4)
const FORCE_F: usize = 60; // floats per force (15 vec4)
const SIM_UNI: u64 = 16 * 8 + (EMIT_F * 4 * MAX_EMIT) as u64 + (FORCE_F * 4 * MAX_FORCES) as u64 + 32 * MAX_COLL as u64;
const RENDER_UNI: u64 = 16 * (23 + 2 * MAX_SPARKS as u64 + 1 + 16 + 256 + 48);
const LIGHT_OFFSET: u64 = 16 * 6;

impl GpuSim {
/// `width` is the domain width in metres; smaller = more detail on the same grid.
    pub fn new(gpu: &Gpu, res: usize, width: f32) -> Self {
        let n = dims(res);
        let cells = (n[0] * n[1] * n[2]) as u64;
        let bufs: Vec<wgpu::Buffer> = (0..BUFS)
            .map(|k| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: cells * if k < VEC4_BUFS { 16 } else { 4 },
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let mk_uni = |size| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sim_uni = mk_uni(SIM_UNI);
        let ren_uni = mk_uni(RENDER_UNI);
        let vol = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volume"),
            size: wgpu::Extent3d { width: n[0] as u32, height: n[1] as u32, depth_or_array_layers: n[2] as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let vol_view = vol.create_view(&Default::default());
        let vol2 = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flames"),
            size: wgpu::Extent3d { width: n[0] as u32, height: n[1] as u32, depth_or_array_layers: n[2] as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let vol2_view = vol2.create_view(&Default::default());
        let hist = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("histograms"), size: 512, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bricks = (n[0].div_ceil(8) * n[1].div_ceil(8) * n[2].div_ceil(8)) as u64;
        let occ = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("occupancy"), size: bricks * 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: sim_uni.as_entire_binding() }];
        entries.extend(bufs.iter().enumerate().map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32 + 1, resource: b.as_entire_binding() }));
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 1, resource: wgpu::BindingResource::TextureView(&vol_view) });
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 2, resource: occ.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 3, resource: hist.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 4, resource: wgpu::BindingResource::TextureView(&vol2_view) });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &gpu.sim_layout, entries: &entries });
        GpuSim { res, n, h: width / n[0] as f32, frame: 0, bufs, vol_view, sampler, occ, hist, vol2_view, sim_uni, ren_uni, bind, sparks: vec![] }
    }

    pub fn origin(&self) -> [f32; 3] {
        let [nx, ny, nz] = self.n;
        [-(nx as f32) * self.h * 0.5, -(ny as f32) * self.h * 0.15, -(nz as f32) * self.h * 0.5]
    }
    pub fn size(&self) -> [f32; 3] {
        [self.n[0] as f32 * self.h, self.n[1] as f32 * self.h, self.n[2] as f32 * self.h]
    }

    /// Temperature (64 bins, 0..10000 K) and smoke (64 bins, 0..2) histograms of every 8th voxel.
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
        let shape_id = |s: Shape| match s {
            Shape::Sphere => 0.0,
            Shape::Box => 1.0,
            Shape::Cylinder => 2.0,
            Shape::Torus => 3.0,
        };
        // expand emitters into shape entries at this moment (bursts grow, particles fly)
        let mut entries: Vec<[f32; EMIT_F]> = vec![];
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
                let chaos = s.noise.clone().or_else(|| e.chaos.clone());
                let ch = chaos.as_ref().map_or([0.0; 3], |c| [c.strength, c.scale, c.speed]);
                let mut en = [0.0f32; EMIT_F];
                en[..4].copy_from_slice(&[c[0], c[1], c[2], 0.0]);
                en[4..8].copy_from_slice(&[(half[0] / h).max(0.6), (half[1] / h).max(0.6), (half[2] / h).max(0.6), s.torus]);
                en[8..12].copy_from_slice(&[rates[0], rates[1], rates[2], s.gradient.unwrap_or(rates[3])]);
                en[12..16].copy_from_slice(&[vel[0] * scale, vel[1] * scale, vel[2] * scale, e.spread]);
                en[16..20].copy_from_slice(&[shape_id(s.kind), e.fuel_add as u8 as f32, e.smoke_add as u8 as f32, e.temp_replace as u8 as f32]);
                en[20..24].copy_from_slice(&[ch[0], ch[1], ch[2], e.expansion]);
                en[24..28].copy_from_slice(&[hollow, fade, 0.0, 0.0]);
                // added after 0.4: rotation (world -> local rows), random pressure, emission noise octaves/seed
                if s.rot != IDENTITY {
                    let rt = transpose(&s.rot);
                    en[28..32].copy_from_slice(&[rt[0][0], rt[0][1], rt[0][2], 1.0]);
                    en[32..36].copy_from_slice(&[rt[1][0], rt[1][1], rt[1][2], 0.0]);
                    en[36..40].copy_from_slice(&[rt[2][0], rt[2][1], rt[2][2], 0.0]);
                }
                let pn = e.pressure_noise;
                en[40..44].copy_from_slice(&[pn[0], h / pn[1].max(0.005), pn[2] * 0.37, pn[3]]);
                let (oct, lac, gain, so) = chaos.as_ref().map_or((1.0, 2.0, 0.5, [0.0; 3]), |c| (c.octaves, c.lacunarity, c.gain, c.seed_off));
                en[44..48].copy_from_slice(&[oct, lac, gain, e.shadow_caster as u8 as f32]);
                en[48..52].copy_from_slice(&[so[0], so[1], so[2], 0.0]);
                entries.push(en);
            };
            match &e.particles {
                Some(pp) => {
                    let origin = e.shapes.first().map_or([0.0; 3], |s| s.center);
                    for k in 0..pp.count {
                        let (mut pos, vel, life) = particle(pp, origin, k, t);
                        // Force: Noise set to act on particles pushes them around (added after 0.4)
                        for f in p.forces.iter().filter(|f| f.kind == 3 && f.mode != 0) {
                            let np = crate::noise::NoiseParams { freq: f.scale * 4.0, octaves: f.noise[0] as u32, lacunarity: f.noise[1], gain: f.noise[2], curl: f.noise[3] > 0.5, seed: 0.0, speed: f.speed * 2.0 };
                            let d = crate::noise::fbm(&np, pos, t);
                            let age = life * pp.lifetime;
                            for a in 0..3 {
                                pos[a] += d[a] * f.strength * 0.05 * age * age;
                            }
                        }
                        let fade = (1.0 - life).max(0.0);
                        let s = ShapeInst::sphere(pos, pp.size);
                        // particle speed couples into the gas as m/s (like an emitter's initial velocity)
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
        let casters = p.emitters.iter().filter(|e| e.shadow_caster).count();
        let colls: Vec<_> = p.colliders.iter().take(MAX_COLL).collect();
        let mut u = U::default();
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, entries.len() as u32])
            .f(&[p.time_scale, p.buoyancy, p.smoke_weight, p.vorticity])
            .f(&[p.burn_rate, (p.ignition_k - AMBIENT_K) / 1000.0, p.heat, p.smoke_yield])
            .f(&[p.cooling, p.smoke_fade, p.wind[0], p.wind[1]])
            .f(&[forces.len() as f32, colls.len() as f32, 0.0, t])
            .f(&[self.h, scale, 0.0, 0.0])
            .f(&[0.0, 1.0, 0.0, 0.0])
            .f(&[1.0, 64.0, casters as f32, 0.0]);
        for k in 0..MAX_EMIT {
            u.f(entries.get(k).map_or(&[0.0f32; EMIT_F], |e| e));
        }
        for k in 0..MAX_FORCES {
            match forces.get(k) {
                Some(fo) => {
                    let c = cells(fo.pos);
                    // vorticles: scale is a vortex size (m); toroidal: extra is a tube radius (m)
                    let sc = if fo.kind == 5 { fo.scale / h } else { fo.scale };
                    let ex = if fo.kind == 2 { fo.extra / h } else { fo.extra };
                    // Rotation turns a force's direction / axis (noise rotates its field below)
                    let dir = if fo.rot != IDENTITY && fo.kind != 3 { crate::sim::mat_apply(&fo.rot, fo.dir) } else { fo.dir };
                    let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-4);
                    let fo_dir = dir;
                    u.f(&[fo.kind as f32, fo.strength, sc, fo.speed])
                        .f(&[c[0], c[1], c[2], fo.radius / h])
                        .f(&[fo_dir[0] / l, fo_dir[1] / l, fo_dir[2] / l, ex]);
                    match &fo.mask {
                        Some(m) => {
                            let mc = cells(m.center);
                            u.f(&[mc[0], mc[1], mc[2], 1.0])
                                .f(&[m.half[0] / h, m.half[1] / h, m.half[2] / h, shape_id(m.kind)])
                                .f(&[m.torus, fo.softness, 0.0, 0.0]);
                        }
                        None => {
                            u.f(&[0.0; 12]);
                        }
                    }
                    // added after 0.4: noise octaves, bias, seed, rotation, field masks, mode
                    let moved = fo.kind == 3 && (fo.rot != IDENTITY || fo.pos != [0.0; 3]);
                    let rt = transpose(&fo.rot);
                    let kk = |x: f32| (x - AMBIENT_K) / 1000.0;
                    u.f(&fo.noise)
                        .f(&[fo.bias[0], fo.bias[1], fo.bias[2], fo.mode as f32])
                        .f(&[fo.seed_off[0], fo.seed_off[1], fo.seed_off[2], fo.masks_04 as u8 as f32])
                        .f(&[rt[0][0], rt[0][1], rt[0][2], moved as u8 as f32])
                        .f(&[rt[1][0], rt[1][1], rt[1][2], 0.0])
                        .f(&[rt[2][0], rt[2][1], rt[2][2], 0.0])
                        .f(&fo.masks)
                        .f(&[kk(fo.ranges[0]), kk(fo.ranges[1]), fo.ranges[2], fo.ranges[3]])
                        .f(&[fo.pos[0], fo.pos[1], fo.pos[2], 0.0]);
                }
                None => {
                    u.f(&[0.0; FORCE_F]);
                }
            }
        }
        for k in 0..MAX_COLL {
            match colls.get(k) {
                Some(cs) => {
                    let c = cells(cs.center);
                    u.f(&[c[0], c[1], c[2], shape_id(cs.kind)]).f(&[cs.half[0] / h, cs.half[1] / h, cs.half[2] / h, cs.torus]);
                }
                None => {
                    u.f(&[0.0; 8]);
                }
            }
        }
        gpu.queue.write_buffer(&self.sim_uni, 0, &u.0);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.dispatch(gpu, &mut enc, &["emit_forces", "curl_pass", "confine", "divergence"]);
        // over-relaxed sweeps settle the large-scale pressure fast, then plain sweeps smooth
        // out the cell-sized leftovers that otherwise show up as grain
        let share = 0.5;
        let n = p.iterations.max(2);
        let n_sor = ((n as f32) * share) as usize;
        let jac: Vec<&str> = (0..n).flat_map(|k| if k < n_sor { ["sor_red", "sor_black"] } else { ["gs_red", "gs_black"] }).collect();
        self.dispatch(gpu, &mut enc, &jac);
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

    /// Replace the scalars (temperature kK, smoke, fuel, flame) with given values (Import).
    pub fn upload_scalars(&self, gpu: &Gpu, data: &[[f32; 4]]) {
        let bytes: Vec<u8> = data.iter().flat_map(|c| c.iter().flat_map(|v| v.to_le_bytes())).collect();
        gpu.queue.write_buffer(&self.bufs[SCA0], 0, &bytes);
        gpu.queue.write_buffer(&self.bufs[SCA1], 0, &bytes);
    }

    /// Scalars per cell: (temperature kK, smoke, fuel, flame).
    pub fn read_scalars(&self, gpu: &Gpu) -> Vec<[f32; 4]> {
        let raw = gpu.read(&self.bufs[SCA0], self.bufs[SCA0].size());
        raw.chunks_exact(16)
            .map(|c| {
                let f = |k: usize| f32::from_le_bytes([c[k * 4], c[k * 4 + 1], c[k * 4 + 2], c[k * 4 + 3]]);
                [f(0), f(1), f(2), f(3)]
            })
            .collect()
    }

    /// Raymarch the volume into `target`.
    /// `seed` varies the sample offsets; `history` is the weight of this frame against the
    /// previous image (1 = fresh image, 0.5 = smooth over frames, 1/n = progressive refinement).
    pub fn render(&self, gpu: &Gpu, target: &Target, cam: &Camera, s: &Shade, fin: &Finish, seed: u32, history: f32) {
        let lk = &s.look;
        let sun = s.sun.filter(|_| s.lit);
        if let Some((_, dir)) = sun {
            let k = s.absorption * 0.3 * s.density * s.shadow * 1.5;
            let mut u = U::default();
            u.f(&[dir[0], dir[1], dir[2], k]);
            // Shading › Lighting: shadow step scale (sharpness 75% = 0.4) and step count
            let sharp = if lk.shadow_sharpness == 0.75 { 1.0 } else { 1.75 - lk.shadow_sharpness };
            u.f(&[sharp, (lk.shadow_steps * lk.shadow_reach).round().max(1.0)]);
            gpu.queue.write_buffer(&self.sim_uni, LIGHT_OFFSET, &u.0);
        }
        let (eye, f, r, up) = cam.basis();
        let tanf = (cam.fov.to_radians() * 0.5).tan();
        let flags = s.lit as u32 | (cam.ortho as u32) << 1 | (s.sky.is_some() as u32) << 2 | (s.ground.is_some() as u32) << 3 | (s.sun.is_some() as u32) << 4;
        let (o, size) = (self.origin(), self.size());
        let sky = s.sky.unwrap_or(([0.0; 3], [0.0; 3]));
        let ground = s.ground.unwrap_or(([0.0; 3], 0.0));
        let (sun_c, sun_d) = s.sun.unwrap_or(([0.0; 3], [0.0, 1.0, 0.0]));
        let mut u = U::default();
        // export pass, alpha (added after 0.4; 0 = the picture)
        let passbits = lk.pass.min(15) | ((lk.alpha > 0) as u32) << 4 | (lk.invert_alpha as u32) << 5 | ((lk.alpha == 2) as u32) << 6;
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, seed])
            .u(&[target.w as u32, target.h as u32, flags, passbits])
            .f(&[eye[0], eye[1], eye[2], tanf])
            .f(&[f[0], f[1], f[2], target.w as f32 / target.h as f32])
            .f(&[r[0], r[1], r[2], cam.dist * tanf])
            .f(&[up[0], up[1], up[2], 0.0])
            .f(&[o[0], o[1], o[2], self.h])
            .f(&[size[0], size[1], size[2], s.step])
            .f(&[s.ambient[0], s.ambient[1], s.ambient[2], 0.0])
            .f(&[sun_c[0], sun_c[1], sun_c[2], 0.0])
            .f(&[sun_d[0], sun_d[1], sun_d[2], self.frame as f32 / FPS]);
        for g in &s.grad {
            u.f(&[g[0], g[1], g[2], 0.0]);
        }
        u.f(&[s.t_min_k, s.t_max_k, s.brightness, s.temp_mul])
            .f(&[s.smoke_color[0], s.smoke_color[1], s.smoke_color[2], s.absorption])
            .f(&[s.density, history, ground.1, s.detail])
            .f(&[sky.0[0], sky.0[1], sky.0[2], 0.0])
            .f(&[sky.1[0], sky.1[1], sky.1[2], 0.0])
            .f(&[ground.0[0], ground.0[1], ground.0[2], 0.0]);
        match s.point {
            Some((pos, col, range)) => {
                u.f(&[pos[0], pos[1], pos[2], 1.0]).f(&[col[0], col[1], col[2], range]);
            }
            None => {
                u.f(&[0.0; 8]);
            }
        }
        let sparks: Vec<_> = if s.sparks { self.sparks.iter().take(MAX_SPARKS).collect() } else { vec![] };
        u.u(&[sparks.len() as u32, 0, 0, 0]);
        let hot = s.grad[2];
        for k in 0..MAX_SPARKS {
            match sparks.get(k) {
                Some((p, r, _)) => u.f(&[p[0], p[1], p[2], *r]),
                None => u.f(&[0.0; 4]),
            };
        }
        for k in 0..MAX_SPARKS {
            match sparks.get(k) {
                Some((_, _, b)) => u.f(&[hot[0] * b, hot[1] * b, hot[2] * b, 0.0]),
                None => u.f(&[0.0; 4]),
            };
        }
        // ---- settings added after 0.4 (feature bits off = the 0.4 picture) ----
        let mut bits = 0u32;
        let mut bit = |on: bool, b: u32| {
            if on {
                bits |= b;
            }
        };
        bit(lk.lut.is_some(), 1);
        bit(!lk.clamp_remap, 2);
        bit(lk.remap_ramp != 1.0, 4);
        bit(!lk.render_flames, 8);
        bit(lk.translucent, 16);
        bit(!lk.render_smoke, 32);
        bit(lk.smoke_remap.is_some(), 64);
        bit(lk.smoke_remap.is_some_and(|r| r.0), 128);
        bit(lk.fuel.is_some(), 256);
        bit(lk.shadow_gamma != 1.0 || lk.extinction != [1.0; 3] || lk.anisotropy != 0.0 || lk.direct != 1.0, 512);
        bit(lk.ao > 0.0, 1024);
        bit(lk.flames_light > 0.0, 2048);
        bit(lk.particles_light > 0.0 && !sparks.is_empty(), 4096);
        bit(lk.emask.is_some(), 8192);
        bit(lk.sharpen.is_some(), 16384);
        bit(lk.post != 0, 32768);
        bit(lk.modulation.is_some(), 65536);
        bit(lk.slice.iter().any(|&v| v > 0.0), 131072);
        bit(!lk.render_volume, 262144);
        bit(!lk.render_particles, 524288);
        bit(lk.vol_occludes_particles, 1048576);
        bit(lk.particles_occlude_vol, 2097152);
        bit(lk.smoke_min > 0.0 || lk.smoke_clamp < 10.0, 4194304);
        bit(lk.flames_min > 0.0 || lk.flames_opacity > 0.0, 8388608);
        bit(lk.shaping_flames > 0.0 || lk.shaping_density > 0.0, 16777216);
        bit(lk.flames_coloring != 0, 33554432);
        let shapes: Vec<_> = lk.shapes.iter().take(8).collect();
        u.u(&[bits, lk.flames_coloring, shapes.len() as u32, lk.shapes_render.min(3) | lk.shapes_mask.min(3) << 2 | (lk.shapes_invert as u32) << 4]);
        let (sr_t, sr_lo, sr_hi, sr_ramp) = lk.smoke_remap.unwrap_or((false, 0.0, 1.0, 1.0));
        let fuel = lk.fuel.unwrap_or(([0.3, 0.55, 1.0], 1.0));
        let em = lk.emask.unwrap_or([0.3, 0.3, 0.4, 1.0, 0.5, 0.5, 0.5]);
        let sh = lk.sharpen.unwrap_or([1.0; 3]);
        let (mch, msc, mint, msp) = lk.modulation.unwrap_or((0, 0.1, 0.5, 1.0));
        let (lo, hi) = (sr_lo, sr_hi);
        let _ = sr_t;
        u.f(&[lk.remap_ramp, lk.flames_min, lk.flames_opacity, lk.shaping_flames])
            .f(&[lk.fire_color[0], lk.fire_color[1], lk.fire_color[2], lk.shaping_density])
            .f(&[lo, hi, sr_ramp, lk.smoke_min])
            .f(&[lk.smoke_clamp, lk.ao, lk.shadow_gamma, lk.direct])
            .f(&[lk.extinction[0], lk.extinction[1], lk.extinction[2], lk.anisotropy * 0.9])
            .f(&[fuel.0[0], fuel.0[1], fuel.0[2], fuel.1])
            .f(&[lk.scatter_radius, lk.flames_light, lk.scatter_occlusion, lk.post_strength])
            .f(&[em[0], em[1], em[2], em[3]])
            .f(&[em[4], em[5], em[6], lk.post as f32])
            .f(&[sh[0], sh[1], sh[2], mch as f32])
            .f(&[msc, mint, msp, lk.slice_ramp]);
        for a in 0..3 {
            u.f(&lk.slice[a * 4..a * 4 + 4]);
        }
        u.f(&[lk.particles_light, 0.0, 0.0, 0.0]).f(&[0.0; 4]);
        for k in 0..256 {
            let c = lk.lut.as_ref().and_then(|l| l.get(k)).copied().unwrap_or([0.0; 3]);
            u.f(&[c[0], c[1], c[2], 0.0]);
        }
        for k in 0..8 {
            match shapes.get(k) {
                Some((si, col)) => {
                    let kind = match si.kind {
                        Shape::Sphere => 0.0,
                        Shape::Box => 1.0,
                        Shape::Cylinder => 2.0,
                        Shape::Torus => 3.0,
                    };
                    let rt = transpose(&si.rot);
                    u.f(&[si.center[0], si.center[1], si.center[2], kind])
                        .f(&[si.half[0], si.half[1], si.half[2], si.torus])
                        .f(&[col[0], col[1], col[2], 0.0])
                        .f(&[rt[0][0], rt[0][1], rt[0][2], 0.0])
                        .f(&[rt[1][0], rt[1][1], rt[1][2], 0.0])
                        .f(&[rt[2][0], rt[2][1], rt[2][2], 0.0]);
                }
                None => {
                    u.f(&[0.0; 24]);
                }
            }
        }
        debug_assert_eq!(u.0.len() as u64, RENDER_UNI);
        gpu.queue.write_buffer(&self.ren_uni, 0, &u.0);
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.render_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ren_uni.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.vol_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&target.view) },
                wgpu::BindGroupEntry { binding: 4, resource: target.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.occ.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&gpu.noise_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&gpu.noise_sampler) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.vol2_view) },
            ],
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        if sun.is_some() {
            self.dispatch(gpu, &mut enc, &["shadow_pass"]);
        }
        enc.clear_buffer(&self.occ, 0, None);
        self.dispatch(gpu, &mut enc, &["pack"]);
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&gpu.render_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(target.w.div_ceil(8) as u32, target.h.div_ceil(8) as u32, 1);
        }
        gpu.submit(enc);
        // Scene node finishing, only when it differs from 0.4's ACES + gamma 2.2
        if !fin.is_04() && (lk.pass == 0 || lk.pass == 2 || lk.pass == 3) {
            gpu.finish(target, fin, lk.alpha, lk.invert_alpha, seed);
        }
    }
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
