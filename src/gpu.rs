// Runs the fluid simulation and the raymarcher on the graphics card.
// This is the 0.4.0 engine, unchanged in what it computes. Additions only: an explicit grid
// size and position (Domain), a histogram pass, and reading the linear image back for EXR.
use crate::render::Camera;
use crate::sim::Domain;
use crate::sim04::{AMBIENT_K, FPS, Shade, Shape, ShapeInst, SimParams, particle};
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
        entries.push(storage(BUFS as u32 + 3, false)); // histograms (Shading node)
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
        Gpu { device, queue, sim_layout, render_layout, pipes, render_pipe, noise_view, noise_sampler, last: Default::default() }
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
    /// Linear colour of the last render (before tonemapping), RGBA floats, for EXR export.
    pub fn read_hdr(&self, t: &Target) -> Vec<f32> {
        let raw = self.read(&t.accum, (t.w * t.h * 16) as u64);
        raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

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
        Target { tex, view, srgb_view, accum, w, h }
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
    /// Grid size, voxel size and position (from the Simulation node's Domain tab).
    pub dom: Domain,
    pub n: [usize; 3],
    pub h: f32,
    pub frame: u32,
    bufs: Vec<wgpu::Buffer>,
    vol_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    occ: wgpu::Buffer,
    hist: wgpu::Buffer,
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
const SIM_UNI: u64 = 16 * 7 + 112 * MAX_EMIT as u64 + 96 * MAX_FORCES as u64 + 32 * MAX_COLL as u64;
const LIGHT_OFFSET: u64 = 16 * 6;

impl GpuSim {
    /// A grid of `dom.n` voxels (sim axes x, up, z) of `dom.h` metres, with its min corner at `dom.origin`.
    pub fn new(gpu: &Gpu, dom: Domain) -> Self {
        let n = dom.n;
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
        let ren_uni = mk_uni(16 * (23 + 2 * MAX_SPARKS as u64));
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
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bricks = (n[0].div_ceil(8) * n[1].div_ceil(8) * n[2].div_ceil(8)) as u64;
        let occ = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("occupancy"), size: bricks * 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let hist = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("histograms"), size: 512, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: sim_uni.as_entire_binding() }];
        entries.extend(bufs.iter().enumerate().map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32 + 1, resource: b.as_entire_binding() }));
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 1, resource: wgpu::BindingResource::TextureView(&vol_view) });
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 2, resource: occ.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: BUFS as u32 + 3, resource: hist.as_entire_binding() });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &gpu.sim_layout, entries: &entries });
        GpuSim { dom, n, h: dom.h, frame: 0, bufs, vol_view, sampler, occ, hist, sim_uni, ren_uni, bind, sparks: vec![] }
    }

    pub fn origin(&self) -> [f32; 3] {
        self.dom.origin
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
        let mut entries: Vec<[f32; 28]> = vec![];
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
                let ch = s.noise.clone().or_else(|| e.chaos.clone()).map_or([0.0; 3], |c| [c.strength, c.scale, c.speed]);
                let mut en = [0.0f32; 28];
                en[..4].copy_from_slice(&[c[0], c[1], c[2], 0.0]);
                en[4..8].copy_from_slice(&[(half[0] / h).max(0.6), (half[1] / h).max(0.6), (half[2] / h).max(0.6), s.torus]);
                en[8..12].copy_from_slice(&[rates[0], rates[1], rates[2], s.gradient.unwrap_or(rates[3])]);
                en[12..16].copy_from_slice(&[vel[0] * scale, vel[1] * scale, vel[2] * scale, e.spread]);
                en[16..20].copy_from_slice(&[shape_id(s.kind), e.fuel_add as u8 as f32, e.smoke_add as u8 as f32, e.temp_replace as u8 as f32]);
                en[20..24].copy_from_slice(&[ch[0], ch[1], ch[2], e.expansion]);
                en[24..28].copy_from_slice(&[hollow, fade, 0.0, 0.0]);
                entries.push(en);
            };
            match &e.particles {
                Some(pp) => {
                    let origin = e.shapes.first().map_or([0.0; 3], |s| s.center);
                    for k in 0..pp.count {
                        let (pos, vel, life) = particle(pp, origin, k, t);
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
        let colls: Vec<_> = p.colliders.iter().take(MAX_COLL).collect();
        let mut u = U::default();
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, entries.len() as u32])
            .f(&[p.time_scale, p.buoyancy, p.smoke_weight, p.vorticity])
            .f(&[p.burn_rate, (p.ignition_k - AMBIENT_K) / 1000.0, p.heat, p.smoke_yield])
            .f(&[p.cooling, p.smoke_fade, p.wind[0], p.wind[1]])
            .f(&[forces.len() as f32, colls.len() as f32, 0.0, t])
            .f(&[self.h, scale, 0.0, 0.0])
            .f(&[0.0, 1.0, 0.0, 0.0]);
        for k in 0..MAX_EMIT {
            u.f(entries.get(k).map_or(&[0.0f32; 28], |e| e));
        }
        for k in 0..MAX_FORCES {
            match forces.get(k) {
                Some(fo) => {
                    let c = cells(fo.pos);
                    // vorticles: scale is a vortex size (m); toroidal: extra is a tube radius (m)
                    let sc = if fo.kind == 5 { fo.scale / h } else { fo.scale };
                    let ex = if fo.kind == 2 { fo.extra / h } else { fo.extra };
                    let l = (fo.dir[0] * fo.dir[0] + fo.dir[1] * fo.dir[1] + fo.dir[2] * fo.dir[2]).sqrt().max(1e-4);
                    u.f(&[fo.kind as f32, fo.strength, sc, fo.speed])
                        .f(&[c[0], c[1], c[2], fo.radius / h])
                        .f(&[fo.dir[0] / l, fo.dir[1] / l, fo.dir[2] / l, ex]);
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
                }
                None => {
                    u.f(&[0.0; 24]);
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
    pub fn render(&self, gpu: &Gpu, target: &Target, cam: &Camera, s: &Shade, seed: u32, history: f32) {
        let sun = s.sun.filter(|_| s.lit);
        if let Some((_, dir)) = sun {
            let k = s.absorption * 0.3 * s.density * s.shadow * 1.5;
            let mut u = U::default();
            u.f(&[dir[0], dir[1], dir[2], k]);
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
        u.u(&[self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, seed])
            .u(&[target.w as u32, target.h as u32, flags, s.pass.min(15) | ((s.alpha > 0) as u32) << 4 | (s.invert_alpha as u32) << 5 | ((s.alpha == 2) as u32) << 6])
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
