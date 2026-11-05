// Installed builds start without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gpu;
mod graph;
mod home;
mod modulate;
mod preset_thumbs;
mod presets;
mod render;
mod sim;

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use eframe::{egui_wgpu, wgpu};
use gpu::{Gpu, GpuSim, Target};
use graph::{ACCENT, Cmd, DIM, Graph, Kind, Node, TEXT, Val, template};
use render::{Camera, Shade, V3};
use sim::{Burst, Chaos, Emitter, FPS, Force, Particles, Shape, ShapeInst, SimParams};
use std::path::PathBuf;

const MENU_BG: Color32 = Color32::from_rgb(37, 40, 41);
const PANEL_BG: Color32 = Color32::from_rgb(32, 34, 35);
const FIELD_BG: Color32 = Color32::from_rgb(50, 52, 53);
const TRACK_BAR: Color32 = Color32::from_rgb(54, 59, 79);
/// One version for the whole app: bump it in Cargo.toml to publish a new release.
const VERSION: &str = env!("CARGO_PKG_VERSION");
// (name, pixel divisor, march step in cells)
// Always (nearly) native resolution; quality mainly sets how finely each ray samples.
const QUALITY: [(&str, f32, f32); 3] = [("Low", 1.25, 0.6), ("Medium", 1.0, 0.4), ("High", 1.0, 0.3)];
/// Weight of the newest frame while playing: smooths sampling noise across frames.
const TEMPORAL: f32 = 0.75;
const HQ_PASSES: u32 = 256;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let shot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let start_home = args.iter().any(|a| a == "--home");
    // a project file given on the command line (e.g. double-clicked) opens at start
    let open = args.iter().skip(1).find(|a| a.ends_with(".fcproj")).map(PathBuf::from);
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 784.0]).with_title("FireCraft").with_icon({
            let (rgba, width, height) = home::logo_rgba();
            egui::IconData { rgba, width, height }
        }),
        renderer: eframe::Renderer::Wgpu,
        // ask for everything the graphics card can do (big grids need big buffers)
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
                device_descriptor: std::sync::Arc::new(|adapter| wgpu::DeviceDescriptor {
                    label: Some("FireCraft"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                    memory_hints: wgpu::MemoryHints::Performance,
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native("FireCraft", opts, Box::new(|cc| Ok(Box::new({
        let mut app = App::new(cc, shot, start_home);
        if let Some(p) = open {
            app.open_path(p);
        }
        app
    }))))
}

// UI axes are Z-up (X toward camera, Y right); the simulation is Y-up.
fn ui2sim(a: V3) -> V3 {
    [a[0], a[2], -a[1]]
}
fn sim2ui(s: V3) -> V3 {
    [s[0], -s[2], s[1]]
}

/// Everything the simulator and renderer need, read from the node graph.
#[derive(Clone)]
struct Built {
    sp: SimParams,
    res: usize,
    domain: f32,
    shade: Shade,
    cam: Camera,
    duration: u32,
    out: [usize; 2],
    cols: usize,
    fstep: u32,
    /// Import nodes: (file, density scale, temperature scale).
    imports: Vec<(String, f32, f32)>,
}

fn chaos_of(n: &Node) -> Chaos {
    Chaos { strength: n.f("Strength"), scale: n.f("Scale"), speed: n.f("Speed") }
}

/// UI half-size (X, Y, Z-up) to sim axes (x, up, z).
fn ui2sim_size(a: V3) -> V3 {
    [a[0], a[2], a[1]]
}

/// Shape regions produced by shape node `i` (following Transform / Noise / Blend / Modifier chains).
fn shapes_of(g: &Graph, i: usize, depth: u32) -> Vec<ShapeInst> {
    let n = &g.nodes[i];
    if !n.enabled || depth > 12 {
        return vec![];
    }
    let child = |port: &str| -> Vec<ShapeInst> { g.sources(i, port).into_iter().flat_map(|j| shapes_of(g, j, depth + 1)).collect() };
    match n.kind {
        Kind::ShapePrimitive => {
            let s = n.f("Size");
            let sc = ui2sim_size(n.c("Scale"));
            let kind = match n.ch("Shape type") {
                1 => Shape::Box,
                2 => Shape::Cylinder,
                3 => Shape::Torus,
                _ => Shape::Sphere,
            };
            vec![ShapeInst { kind, half: [s * sc[0], s * sc[1], s * sc[2]], torus: n.f("Torus thickness"), ..ShapeInst::sphere(ui2sim(n.c("Position")), s) }]
        }
        Kind::ShapeBurst => {
            let mut s = ShapeInst::sphere(ui2sim(n.c("Position")), n.f("Final radius"));
            s.burst = Some(Burst { radius: n.f("Final radius"), duration: n.f("Duration"), thickness: n.f("Shell thickness"), start: n.f("Start time") });
            vec![s]
        }
        Kind::ShapeParticles => {
            let c = ui2sim(n.c("Position"));
            let (count, spread, size, seed) = (n.f("Count") as usize, n.f("Spread"), n.f("Particle size"), n.f("Seed"));
            (0..count.max(1))
                .map(|k| {
                    let h = |a: f32| (((k as f32 + 1.0) * 12.9898 + a * 78.233 + seed * 3.7).sin() * 43758.547).fract().abs() * 2.0 - 1.0;
                    let mut s = ShapeInst::sphere([c[0] + h(1.0) * spread, c[1] + h(2.0) * spread, c[2] + h(3.0) * spread], size);
                    s.wander = n.f("Wander");
                    s.seed = seed + k as f32 * 1.7;
                    s
                })
                .collect()
        }
        Kind::ShapeNoise => {
            let ch = chaos_of(n);
            child("Shape").into_iter().map(|mut s| {
                s.noise = Some(ch.clone());
                s
            }).collect()
        }
        Kind::ShapeTransform => {
            let off = ui2sim(n.c("Offset"));
            let sc = ui2sim_size(n.c("Scale"));
            child("Shape").into_iter().map(|mut s| {
                for k in 0..3 {
                    s.center[k] = s.center[k] * sc[k] + off[k];
                    s.half[k] *= sc[k];
                }
                if let Some(b) = &mut s.burst {
                    b.radius *= sc[0];
                }
                s
            }).collect()
        }
        Kind::ShapeBlend => match n.ch("Mode") {
            1 => child("Shape A"),
            2 => child("Shape B"),
            _ => [child("Shape A"), child("Shape B")].concat(),
        },
        Kind::ShapeModifier => {
            let (hollow, mul, edge) = (n.f("Hollow"), n.f("Size multiplier"), n.f("Edge softness"));
            child("Shape").into_iter().map(|mut s| {
                s.hollow = hollow;
                s.half = s.half.map(|v| v * mul);
                s.gradient = Some(edge);
                s
            }).collect()
        }
        _ => vec![],
    }
}

/// First shape region wired into input `port` of node `i` (force masks).
fn first_shape(g: &Graph, i: usize, port: &str) -> Option<ShapeInst> {
    g.sources(i, port).into_iter().flat_map(|j| shapes_of(g, j, 0)).next()
}

/// Approximate colour of a blackbody at `k` kelvin (normalised so the brightest channel is 1).
fn blackbody(k: f32, sat: f32) -> V3 {
    let t = (k / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 { 1.0 } else { (329.7 * (t - 60.0).powf(-0.1332) / 255.0).clamp(0.0, 1.0) };
    let gr = if t <= 66.0 { (99.47 * t.ln() - 161.1) / 255.0 } else { 288.1 * (t - 60.0).powf(-0.0755) / 255.0 };
    let b = if t >= 66.0 { 1.0 } else if t <= 19.0 { 0.0 } else { (138.5 * (t - 10.0).ln() - 305.0) / 255.0 };
    let c = [r, gr.clamp(0.0, 1.0), b.clamp(0.0, 1.0)];
    // linear-ish and adjustable saturation
    let grey = (c[0] + c[1] + c[2]) / 3.0;
    c.map(|v| (grey + (v - grey) * sat).clamp(0.0, 1.0).powf(2.2))
}

fn build(g: &Graph, lit: bool, ortho: bool, step: f32) -> Built {
    let live = g.live();
    let find = |k: Kind| (0..g.nodes.len()).find(|&i| g.nodes[i].kind == k && live[i]);
    let first_of = |i: Option<usize>, port: &str, ks: &[Kind]| -> Option<&Node> {
        i.and_then(|i| g.inputs(&live, i, port).into_iter().map(|j| &g.nodes[j]).find(|n| ks.contains(&n.kind)))
    };
    let sim_i = find(Kind::Simulation);
    let simn = sim_i.map(|i| &g.nodes[i]).cloned().unwrap_or_else(|| template(Kind::Simulation));

    let mut emitters = vec![];
    let mut forces = vec![];
    let mut colliders = vec![];
    let mut imports = vec![];
    if let Some(si) = sim_i {
        for e in g.inputs(&live, si, "Emitters") {
            let n = &g.nodes[e];
            if n.kind == Kind::Import {
                if !n.text.is_empty() {
                    imports.push((n.text.clone(), n.f("Density scale"), n.f("Temperature scale")));
                }
                continue;
            }
            if !matches!(n.kind, Kind::Emitter | Kind::EmitterParticles) || !n.b("Emitter activity") {
                continue;
            }
            let pos = ui2sim(n.c("Position"));
            let mut shapes: Vec<ShapeInst> = g.sources(e, "Shapes").into_iter().flat_map(|j| shapes_of(g, j, 0)).collect();
            if shapes.is_empty() {
                shapes.push(ShapeInst::sphere([0.0; 3], 0.22));
            }
            for s in &mut shapes {
                for k in 0..3 {
                    s.center[k] += pos[k];
                }
            }
            // forces wired into the emitter: noise breaks up the emission, lines push it
            let mut vel = if n.kind == Kind::Emitter { ui2sim(n.c("Initial velocity")) } else { [0.0; 3] };
            let mut chaos = None;
            for j in g.sources(e, "Forces") {
                let f = &g.nodes[j];
                match f.kind {
                    Kind::ForceNoise => chaos = Some(chaos_of(f)),
                    Kind::ForceLine => {
                        let d = ui2sim(f.c("Direction"));
                        for k in 0..3 {
                            vel[k] += d[k] * f.f("Strength") * 0.5;
                        }
                    }
                    _ => {}
                }
            }
            let particles = (n.kind == Kind::EmitterParticles).then(|| Particles {
                count: n.f("Count") as usize,
                lifetime: n.f("Lifetime"),
                speed: n.f("Speed"),
                spread_deg: n.f("Spread angle"),
                dir: ui2sim(n.c("Direction")),
                gravity: n.f("Gravity"),
                size: n.f("Particle size"),
                seed: n.f("Seed"),
                sparks: n.b("Render sparks"),
                brightness: n.f("Spark brightness"),
            });
            let volume = n.kind == Kind::Emitter;
            emitters.push(Emitter {
                shapes,
                particles,
                fuel: n.f("Fuel rate"),
                smoke: n.f("Smoke rate"),
                temp_k: n.f("Temperature"),
                gradient: if volume { n.f("Emission gradient") } else { 15.0 },
                expansion: n.f("Expansion"),
                vel,
                spread: if volume { n.f("Velocity spread") } else { 0.3 },
                fuel_add: !volume || n.ch("Fuel emission") == 0,
                smoke_add: !volume || n.ch("Smoke emission") == 0,
                temp_replace: !volume || n.ch("Temperature emission") == 1,
                burst: if volume && n.f("Time between bursts") > 0.0 { n.f("Duration of burst") } else { 0.0 },
                gap: if volume { n.f("Time between bursts") } else { 0.0 },
                chaos,
            });
        }
        for j in g.inputs(&live, si, "Forces") {
            let n = &g.nodes[j];
            let (kind, radius, dir, extra, scale, speed) = match n.kind {
                Kind::ForceLine => (0, 1.0, ui2sim(n.c("Direction")), 0.0, 0.0, 0.0),
                Kind::ForcePoint => (1, n.f("Radius"), [0.0, 1.0, 0.0], 0.0, 0.0, 0.0),
                Kind::ForceToroidal => (2, n.f("Ring radius"), ui2sim(n.c("Axis")), n.f("Tube radius"), 0.0, 0.0),
                Kind::ForceNoise => (3, 1.0, [0.0, 1.0, 0.0], 0.0, n.f("Scale"), n.f("Speed")),
                Kind::ForceVectorField => (4, n.f("Radius"), ui2sim(n.c("Axis")), n.ch("Field type") as f32, 0.0, 0.0),
                Kind::ForceVorticles => (5, n.f("Area"), [0.0, 1.0, 0.0], 0.0, n.f("Vortex size"), n.f("Rise speed")),
                _ => continue,
            };
            forces.push(Force {
                kind,
                strength: n.f("Strength"),
                scale,
                speed,
                pos: ui2sim(n.c("Position")),
                radius,
                dir,
                extra,
                mask: first_shape(g, j, "Mask Shapes"),
                softness: n.f("Mask softness"),
            });
        }
        for j in g.inputs(&live, si, "Colliders") {
            if g.nodes[j].kind == Kind::Collider && g.nodes[j].b("Solid") {
                colliders.extend(g.sources(j, "Shapes").into_iter().flat_map(|k| shapes_of(g, k, 0)));
            }
        }
    }
    let wind = ui2sim(simn.c("Wind"));
    let sp = SimParams {
        time_scale: simn.f("Time scale"),
        buoyancy: simn.f("Buoyancy"),
        smoke_weight: simn.f("Smoke weight"),
        vorticity: simn.f("Vorticity"),
        burn_rate: simn.f("Burn rate"),
        ignition_k: simn.f("Ignition temperature"),
        heat: simn.f("Heat from burning"),
        smoke_yield: simn.f("Smoke from burning"),
        cooling: simn.f("Cooling"),
        smoke_fade: simn.f("Smoke dissipation"),
        wind: [wind[0], wind[2]],
        iterations: simn.f("Pressure iterations") as usize,
        substeps: (simn.f("Substeps") as usize).max(1),
        forces,
        colliders,
        emitters,
    };

    // shading
    let shading_i = find(Kind::Shading);
    let shading = shading_i.map(|i| g.nodes[i].clone());
    let mut ambient = [0.0; 3];
    let mut sun = None;
    let mut point = None;
    if let Some(si) = shading_i {
        for j in g.inputs(&live, si, "Lights") {
            let n = &g.nodes[j];
            let col = render::mul(n.c("Color"), n.f("Intensity"));
            match n.kind {
                Kind::LightAmbient => ambient = render::add(ambient, col),
                Kind::LightDirectional if sun.is_none() => {
                    let (el, az) = (n.f("Elevation").to_radians(), n.f("Azimuth").to_radians());
                    sun = Some((col, render::norm(ui2sim([el.cos() * az.cos(), el.cos() * az.sin(), el.sin()]))));
                }
                Kind::LightPoint if point.is_none() => point = Some((ui2sim(n.c("Position")), col, n.f("Range"))),
                _ => {}
            }
        }
    } else {
        ambient = [1.0; 3];
    }
    // fire colour: gradient, constant or blackbody
    let colorn = first_of(shading_i, "Fire color", &[Kind::FireGradient, Kind::ColorConstant, Kind::ColorBlackbody]).cloned().unwrap_or_else(|| template(Kind::FireGradient));
    let (grad, t_min_k, t_max_k, brightness) = match colorn.kind {
        Kind::ColorConstant => {
            let c = colorn.c("Color");
            ([c.map(|v| v * 0.4), c, c.map(|v| (v * 1.3).min(1.0))], colorn.f("Min temperature"), colorn.f("Max temperature"), colorn.f("Brightness"))
        }
        Kind::ColorBlackbody => {
            let (lo, hi, sat) = (colorn.f("Min temperature"), colorn.f("Max temperature"), colorn.f("Saturation"));
            ([blackbody(lo, sat).map(|v| v * 0.5), blackbody((lo + hi) * 0.5, sat), blackbody(hi, sat)], lo, hi, colorn.f("Brightness"))
        }
        _ => ([colorn.c("Cold color"), colorn.c("Mid color"), colorn.c("Hot color")], colorn.f("Min temperature"), colorn.f("Max temperature"), colorn.f("Brightness")),
    };
    let shn = shading.unwrap_or_else(|| template(Kind::Shading));
    let scene_i = find(Kind::Scene);
    let proc_ = scene_i.and_then(|s| first_of(Some(s), "Volume", &[Kind::VolumeProcessing]));
    let has_volume = proc_.is_some_and(|p| {
        let pi = g.nodes.iter().position(|n| n.id == p.id).unwrap();
        !g.inputs(&live, pi, "Volume").is_empty()
    });
    let sky = first_of(scene_i, "Skybox", &[Kind::Skybox]).map(|n| (n.c("Top color"), n.c("Bottom color")));
    let ground = first_of(scene_i, "Ground", &[Kind::Ground]).map(|n| (n.c("Color"), n.f("Height")));
    let sparks = scene_i.is_some_and(|s| !g.inputs(&live, s, "Particles").is_empty());
    let shade = Shade {
        lit,
        ambient,
        sun,
        grad,
        t_min_k,
        t_max_k,
        brightness,
        smoke_color: shn.c("Smoke color"),
        absorption: shn.f("Absorption"),
        shadow: shn.f("Shadow strength"),
        detail: shn.f("Fine detail"),
        point,
        sparks,
        density: if has_volume { proc_.map_or(1.0, |p| p.f("Smoke density")) } else { 0.0 },
        temp_mul: if has_volume { proc_.map_or(1.0, |p| p.f("Temperature multiplier")) } else { 0.0 },
        sky,
        ground,
        step,
    };
    let render_i = find(Kind::Render);
    let camn = first_of(render_i, "Camera", &[Kind::Camera, Kind::CameraFree])
        .or_else(|| g.nodes.iter().find(|n| n.kind == Kind::Camera))
        .cloned()
        .unwrap_or_else(|| template(Kind::Camera));
    let cam = if camn.kind == Kind::CameraFree {
        let (eye, at) = (ui2sim(camn.c("Position")), ui2sim(camn.c("Look at")));
        let d = render::sub(eye, at);
        let dist = render::dot(d, d).sqrt().max(0.05);
        Camera {
            yaw: d[0].atan2(d[2]).to_degrees(),
            pitch: (d[1] / dist).clamp(-1.0, 1.0).asin().to_degrees(),
            dist,
            fov: camn.f("Field of view"),
            target: at,
            ortho,
        }
    } else {
        Camera {
            yaw: camn.f("Yaw"),
            pitch: camn.f("Pitch"),
            dist: camn.f("Distance"),
            fov: camn.f("Field of view"),
            target: [0.0, camn.f("Target height"), 0.0],
            ortho,
        }
    };
    let rn = g.nodes.iter().find(|n| n.kind == Kind::Render).cloned().unwrap_or_else(|| template(Kind::Render));
    Built {
        sp,
        res: simn.ch("Resolution"),
        domain: simn.f("Domain size").max(0.2),
        shade,
        cam,
        duration: rn.f("Duration").max(1.0) as u32,
        out: [rn.f("Width") as usize, rn.f("Height") as usize],
        cols: rn.f("Flipbook columns").max(1.0) as usize,
        fstep: rn.f("Frame step").max(1.0) as u32,
        imports,
    }
}


enum Job {
    Sequence,
    Flipbook,
    Volume,
}

fn save_png(path: &std::path::Path, px: &[u8], w: usize, h: usize) -> Result<(), String> {
    image::save_buffer(path, px, w as u32, h as u32, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

/// Mitsuba-style .vol grid (float32, 1 channel).
fn save_vol(path: &std::path::Path, sim: &GpuSim, data: impl Iterator<Item = f32>) -> Result<(), String> {
    let mut b = b"VOL\x03".to_vec();
    b.extend(1i32.to_le_bytes());
    for d in sim.n {
        b.extend((d as i32).to_le_bytes());
    }
    b.extend(1i32.to_le_bytes());
    let (o, s) = (sim.origin(), sim.size());
    for v in [o[0], o[1], o[2], o[0] + s[0], o[1] + s[1], o[2] + s[2]] {
        b.extend(v.to_le_bytes());
    }
    for v in data {
        b.extend(v.to_le_bytes());
    }
    std::fs::write(path, b).map_err(|e| e.to_string())
}

/// Read a .vol grid (as written by the volume export) resampled to grid size `n`.
fn load_vol(path: &std::path::Path, n: [usize; 3]) -> Result<Vec<f32>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.len() < 48 || &b[..3] != b"VOL" {
        return Err(format!("{} is not a .vol volume", path.display()));
    }
    let i32_at = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
    let src = [i32_at(8), i32_at(12), i32_at(16)];
    let data: Vec<f32> = b[48..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if data.len() < src[0] * src[1] * src[2] {
        return Err("volume file is truncated".into());
    }
    let mut out = vec![0.0; n[0] * n[1] * n[2]];
    for z in 0..n[2] {
        for y in 0..n[1] {
            for x in 0..n[0] {
                // nearest source cell
                let sx = (x * src[0] / n[0]).min(src[0] - 1);
                let sy = (y * src[1] / n[1]).min(src[1] - 1);
                let sz = (z * src[2] / n[2]).min(src[2] - 1);
                out[x + n[0] * (y + n[1] * z)] = data[sx + src[0] * (sy + src[1] * sz)];
            }
        }
    }
    Ok(out)
}

fn new_export_dir(base: &str) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = PathBuf::from(base).join(format!("export_{stamp}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn export_volume(gpu: &Gpu, sim: &GpuSim, base: &str) -> Result<String, String> {
    let dir = new_export_dir(base)?;
    let s = sim.read_scalars(gpu);
    save_vol(&dir.join("density.vol"), sim, s.iter().map(|c| c[1]))?;
    save_vol(&dir.join("temperature.vol"), sim, s.iter().map(|c| c[0] * 1000.0 + sim::AMBIENT_K))?;
    save_vol(&dir.join("fuel.vol"), sim, s.iter().map(|c| c[2]))?;
    Ok(format!("Saved volume (frame {}) to {}", sim.frame, dir.display()))
}

/// Image sequence / flipbook export, advanced a few frames per UI frame on the GPU.
struct ExportJob {
    flip: bool,
    b: Built,
    /// Graph and view settings, re-read each frame (modulators animate during export).
    src: (Graph, bool, bool, f32),
    sim: GpuSim,
    target: Target,
    dir: PathBuf,
    sheet: Vec<u8>,
    rows: usize,
    tile: usize,
}

impl ExportJob {
    fn new(gpu: &Gpu, b: Built, src: (Graph, bool, bool, f32), flip: bool, base: &str) -> Result<Self, String> {
        let dir = new_export_dir(base)?;
        let [w, h] = b.out;
        let frames = (0..b.duration).filter(|f| f % b.fstep == 0).count();
        let rows = frames.div_ceil(b.cols);
        Ok(ExportJob {
            flip,
            sim: GpuSim::new(gpu, b.res, b.domain),
            target: Target::new(gpu, w, h),
            sheet: if flip { vec![0u8; w * b.cols * h * rows * 4] } else { vec![] },
            rows,
            tile: 0,
            dir,
            b,
            src,
        })
    }

    fn progress(&self) -> f32 {
        self.sim.frame as f32 / self.b.duration as f32
    }

    /// Ok(Some(message)) when finished.
    fn advance(&mut self, gpu: &Gpu, steps: u32) -> Result<Option<String>, String> {
        let [w, h] = self.b.out;
        for _ in 0..steps {
            if self.sim.frame >= self.b.duration {
                break;
            }
            let f = self.sim.frame;
            let (g, lit, ortho, step) = &self.src;
            let now = build(&modulate::apply(g, f as f32 / FPS), *lit, *ortho, *step);
            self.sim.step(gpu, &now.sp);
            if f % self.b.fstep != 0 {
                continue;
            }
            self.sim.render(gpu, &self.target, &now.cam, &now.shade, 0, 1.0);
            let px = gpu.read_target(&self.target);
            if self.flip {
                let cols = self.b.cols;
                let (tx, ty) = (self.tile % cols, self.tile / cols);
                for row in 0..h {
                    let dst = ((ty * h + row) * w * cols + tx * w) * 4;
                    self.sheet[dst..dst + w * 4].copy_from_slice(&px[row * w * 4..(row + 1) * w * 4]);
                }
            } else {
                save_png(&self.dir.join(format!("frame_{:04}.png", f)), &px, w, h)?;
            }
            self.tile += 1;
        }
        if self.sim.frame < self.b.duration {
            return Ok(None);
        }
        if self.flip {
            save_png(&self.dir.join(format!("flipbook_{}x{}.png", self.b.cols, self.rows)), &self.sheet, w * self.b.cols, h * self.rows)?;
        }
        Ok(Some(format!("Saved {} frames to {}", self.tile, self.dir.display())))
    }
}

struct App {
    graph: Graph,
    gpu: Gpu,
    rs: egui_wgpu::RenderState,
    sim: GpuSim,
    view: Option<(Target, egui::TextureId)>,
    tool: usize,
    view_tab: usize,
    cam_view: usize,
    quality: usize,
    hq: bool,
    lit: bool,
    ortho: bool,
    render_shapes: bool,
    lock_camera: bool,
    show_axes: bool,
    show_bounds: bool,
    playing: bool,
    looping: bool,
    show_home: bool,
    home: home::Home,
    hq_samples: u32,
    hq_sig: String,
    last_frame: u32,
    undo: Vec<String>,
    redo: Vec<String>,
    committed: String,
    clipboard: String,
    show_about: bool,
    search: String,
    project: Option<PathBuf>,
    export_dir: String,
    export: Option<ExportJob>,
    status: String,
    shot: Option<PathBuf>,
    seek_to: Option<u32>,
    frames: u32,
}

impl App {
    fn new(cc: &eframe::CreationContext, shot: Option<PathBuf>, start_home: bool) -> Self {
        let mut v = egui::Visuals::dark();
        v.panel_fill = PANEL_BG;
        v.window_fill = Color32::from_rgb(40, 42, 44);
        v.extreme_bg_color = Color32::from_rgb(22, 23, 24);
        v.faint_bg_color = Color32::from_rgb(36, 38, 40);
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = ACCENT;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius = CornerRadius::same(2);
        }
        v.widgets.inactive.bg_fill = FIELD_BG;
        v.widgets.inactive.weak_bg_fill = FIELD_BG;
        v.widgets.inactive.fg_stroke.color = TEXT;
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(62, 65, 67);
        v.widgets.hovered.bg_fill = Color32::from_rgb(62, 65, 67);
        v.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(26, 27, 29);
        cc.egui_ctx.set_visuals(v);
        cc.egui_ctx.style_mut(|s| {
            s.spacing.item_spacing = vec2(6.0, 3.0);
            s.spacing.button_padding = vec2(6.0, 2.0);
            for (t, f) in s.text_styles.iter_mut() {
                f.size = match t {
                    egui::TextStyle::Heading => 15.0,
                    egui::TextStyle::Small => 9.5,
                    _ => 12.0,
                };
            }
        });
        let rs = cc.wgpu_render_state.clone().expect("FireCraft needs a graphics card with WebGPU support");
        let gpu = Gpu::new(rs.device.clone(), rs.queue.clone());
        App {
            graph: Graph::default(),
            sim: GpuSim::new(&gpu, 1, 1.0),
            gpu,
            rs,
            view: None,
            tool: 0,
            view_tab: 0,
            cam_view: 0,
            quality: 0,
            hq: false,
            lit: true,
            ortho: false,
            render_shapes: true,
            lock_camera: false,
            show_axes: true,
            show_bounds: false,
            playing: true,
            looping: false,
            show_home: start_home,
            home: home::Home::new(),
            hq_samples: 0,
            hq_sig: String::new(),
            last_frame: u32::MAX,
            undo: vec![],
            redo: vec![],
            committed: String::new(),
            clipboard: String::new(),
            show_about: false,
            search: String::new(),
            project: None,
            export_dir: home::default_export_dir().display().to_string(),
            export: None,
            status: String::new(),
            shot,
            seek_to: None,
            frames: 0,
        }
    }

    fn step(&self) -> f32 {
        if self.hq_active() { 0.2 } else { QUALITY[self.quality].2 }
    }

    fn built(&self) -> Built {
        let g = modulate::apply(&self.graph, self.sim.frame as f32 / FPS);
        build(&g, self.lit, self.ortho, self.step())
    }

    /// HQ mode: Ctrl+3 toggle, and always on in the Render tab.
    fn hq_active(&self) -> bool {
        self.hq || self.view_tab == 1
    }

    fn make_thumb(&mut self, i: usize) {
        let mut g = Graph::default();
        presets::apply(&mut g, i);
        let b = build(&g, true, false, 0.7);
        let mut s = GpuSim::new(&self.gpu, 0, b.domain);
        let frames = presets::PRESETS[i].frame.min(90);
        for _ in 0..frames {
            s.step(&self.gpu, &b.sp);
        }
        let t = Target::new(&self.gpu, 400, 248);
        s.render(&self.gpu, &t, &b.cam, &b.shade, 0, 1.0);
        let id = self.rs.renderer.write().register_native_texture(&self.gpu.device, &t.srgb_view, wgpu::FilterMode::Linear);
        self.home.thumbs[i] = Some((t, id));
    }

    fn home_action(&mut self, a: home::Action) {
        match a {
            home::Action::Return => self.show_home = false,
            home::Action::NewProject => {
                self.graph = Graph::default();
                self.project = None;
                self.restart();
                self.show_home = false;
            }
            home::Action::OpenDialog => self.open_project(),
            home::Action::Open(p) => self.open_path(p),
            home::Action::Preset(i) => {
                presets::apply(&mut self.graph, i);
                self.project = None;
                self.restart();
                self.reset_history();
                self.playing = true;
                self.show_home = false;
            }
        }
    }

    fn restart(&mut self) {
        let b = self.built();
        self.sim = GpuSim::new(&self.gpu, b.res, b.domain);
        self.load_imports(&b);
    }

    /// Import nodes: load .vol grids as the starting state of the simulation.
    fn load_imports(&mut self, b: &Built) {
        let mut cells = vec![[0.0f32; 4]; self.sim.n.iter().product()];
        let mut any = false;
        for (path, dens, temp) in &b.imports {
            let p = PathBuf::from(path);
            let read = |f: &std::path::Path| load_vol(f, self.sim.n);
            match read(&p) {
                Ok(d) => {
                    any = true;
                    // a temperature.vol next to it adds heat
                    let t = read(&p.with_file_name("temperature.vol")).ok();
                    for (k, c) in cells.iter_mut().enumerate() {
                        c[1] += d[k] * dens;
                        if let Some(t) = &t {
                            c[0] += ((t[k] - sim::AMBIENT_K) / 1000.0).max(0.0) * temp;
                        }
                    }
                }
                Err(e) => self.status = format!("Import failed: {e}"),
            }
        }
        if any {
            self.sim.upload_scalars(&self.gpu, &cells);
        }
    }

    /// Run an Export node's "Export now".
    fn export_node(&mut self, i: usize) {
        let n = self.graph.nodes[i].clone();
        let mut b = self.built();
        let job = match n.kind {
            Kind::ExportImage => {
                b.out = [n.f("Width") as usize, n.f("Height") as usize];
                b.duration = n.f("Frames").max(1.0) as u32;
                b.fstep = n.f("Frame step").max(1.0) as u32;
                Job::Sequence
            }
            Kind::ExportFlipbook => {
                b.out = [n.f("Tile width") as usize, n.f("Tile height") as usize];
                b.duration = n.f("Frames").max(1.0) as u32;
                b.fstep = n.f("Frame step").max(1.0) as u32;
                b.cols = n.f("Columns").max(1.0) as usize;
                Job::Flipbook
            }
            _ => Job::Volume,
        };
        self.start_export_with(job, b);
    }

    /// Jump to a frame. Going back restarts; the frames in between are simulated a few per
    /// UI frame (see `advance_seek`) so the window never freezes during long jumps.
    fn seek(&mut self, target: u32) {
        let target = target.min(2000);
        if target < self.sim.frame {
            self.restart();
        }
        self.seek_to = Some(target);
    }

    fn advance_seek(&mut self) {
        let Some(target) = self.seek_to else { return };
        let start = std::time::Instant::now();
        while self.sim.frame < target && start.elapsed().as_millis() < 30 {
            let b = self.built();
            self.sim.step(&self.gpu, &b.sp);
        }
        if self.sim.frame >= target {
            self.seek_to = None;
            self.status.clear();
        } else {
            self.status = format!("Jumping to frame {target}… ({})", self.sim.frame);
        }
    }

    fn start_export(&mut self, job: Job) {
        let b = self.built();
        self.start_export_with(job, b);
    }

    fn start_export_with(&mut self, job: Job, b: Built) {
        if self.export.is_some() {
            return;
        }
        let src = (self.graph.clone(), self.lit, self.ortho, self.step());
        let r = match job {
            Job::Volume => export_volume(&self.gpu, &self.sim, &self.export_dir).map(Some),
            _ => ExportJob::new(&self.gpu, b, src, matches!(job, Job::Flipbook), &self.export_dir).map(|j| {
                self.export = Some(j);
                None
            }),
        };
        self.status = match r {
            Ok(Some(msg)) => msg,
            Ok(None) => "Exporting…".into(),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    fn save_project(&mut self, ask: bool) {
        let path = match (&self.project, ask) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("FireCraft project", &["fcproj"]).set_file_name("Default_Fire.fcproj").save_file(),
        };
        if let Some(p) = path {
            self.status = match std::fs::write(&p, self.graph.save()) {
                Ok(_) => format!("Saved {}", p.display()),
                Err(e) => format!("Could not save: {e}"),
            };
            if let Some((t, _)) = &self.view {
                let px = self.gpu.read_target(t);
                let _ = save_png(&home::thumb_path(&p), &px, t.w, t.h);
            }
            self.home.add_recent(p.clone());
            self.project = Some(p);
        }
    }

    fn open_project(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("FireCraft project", &["fcproj"]).pick_file() {
            self.open_path(p);
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| Graph::load(&t)) {
            Ok(g) => {
                self.graph = g;
                self.home.add_recent(p.clone());
                self.project = Some(p);
                self.restart();
                self.reset_history();
                self.show_home = false;
            }
            Err(e) => self.status = format!("Could not open: {e}"),
        }
    }

    /// Record a finished change for undo (after drags/typing end).
    fn track_undo(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.any_down()) || ctx.wants_keyboard_input() {
            return;
        }
        let now = self.graph.save();
        if self.committed.is_empty() {
            self.committed = now;
        } else if now != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, now));
            self.redo.clear();
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
    }

    /// Forget history (new project, opened file, preset).
    fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.committed = self.graph.save();
    }

    fn edit(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Undo => {
                if let Some(prev) = self.undo.pop() {
                    self.redo.push(self.graph.save());
                    self.graph.restore(&prev);
                    self.committed = prev;
                    self.status = "Undo".into();
                }
            }
            Cmd::Redo => {
                if let Some(next) = self.redo.pop() {
                    self.undo.push(self.graph.save());
                    self.graph.restore(&next);
                    self.committed = next;
                    self.status = "Redo".into();
                }
            }
            Cmd::Copy | Cmd::Cut => {
                if let Some(t) = self.graph.copy_selection() {
                    ctx.copy_text(t.clone());
                    self.clipboard = t;
                    if cmd == Cmd::Cut {
                        self.graph.delete_selection();
                    }
                }
            }
            Cmd::Paste => {
                let clip = self.clipboard.clone();
                if !clip.is_empty() {
                    self.graph.paste(&clip);
                }
            }
            Cmd::Duplicate => self.graph.duplicate_selection(),
            Cmd::Delete => self.graph.delete_selection(),
            Cmd::SelectAll => self.graph.select_all(),
        }
    }

    /// App-wide editing shortcuts (skipped while typing in a text box).
    fn edit_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        let mut cmds = vec![];
        let mut pasted = None;
        ctx.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Copy => cmds.push(Cmd::Copy),
                    egui::Event::Cut => cmds.push(Cmd::Cut),
                    egui::Event::Paste(t) => pasted = Some(t.clone()),
                    _ => {}
                }
            }
        });
        if let Some(t) = pasted {
            if t.starts_with("firecraft-project") {
                self.clipboard = t;
            }
            cmds.push(Cmd::Paste);
        }
        ctx.input_mut(|i| {
            let sc = Modifiers::COMMAND | Modifiers::SHIFT;
            if i.consume_key(sc, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y) {
                cmds.push(Cmd::Redo);
            } else if i.consume_key(Modifiers::COMMAND, Key::Z) {
                cmds.push(Cmd::Undo);
            }
            if i.consume_key(Modifiers::COMMAND, Key::D) {
                cmds.push(Cmd::Duplicate);
            }
            if i.consume_key(Modifiers::COMMAND, Key::A) {
                cmds.push(Cmd::SelectAll);
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace) {
                cmds.push(Cmd::Delete);
            }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.graph.clear_selection();
        }
        let (save, open, new) = ctx.input_mut(|i| {
            (i.consume_key(Modifiers::COMMAND, Key::S), i.consume_key(Modifiers::COMMAND, Key::O), i.consume_key(Modifiers::COMMAND, Key::N))
        });
        for c in cmds {
            self.edit(ctx, c);
        }
        if save {
            self.save_project(false);
        }
        if open {
            self.open_project();
        }
        if new {
            self.graph = Graph::default();
            self.project = None;
            self.restart();
            self.reset_history();
        }
    }

    /// Save what the viewport shows as a PNG in the export folder.
    fn save_view_png(&mut self) {
        let Some((t, _)) = &self.view else { return };
        let px = self.gpu.read_target(t);
        self.status = match new_export_dir(&self.export_dir).and_then(|d| {
            let f = d.join(format!("frame_{:04}.png", self.sim.frame));
            save_png(&f, &px, t.w, t.h).map(|_| f)
        }) {
            Ok(f) => format!("Saved {}", f.display()),
            Err(e) => format!("Could not save: {e}"),
        };
    }

    fn set_view(&mut self, i: usize) {
        self.cam_view = i;
        let (yaw, pitch) = [(90.0, 8.0), (0.0, 0.0), (90.0, 0.0), (90.0, 89.0)][i];
        if let Some(c) = self.camera_node() {
            c.set("Yaw", Val::F(yaw));
            c.set("Pitch", Val::F(pitch));
            if i == 0 {
                c.set("Distance", Val::F(2.6));
            }
        }
    }

    fn camera_node(&mut self) -> Option<&mut Node> {
        self.graph.node_mut(Kind::Camera)
    }

    fn emitter_index(&self) -> Option<usize> {
        self.graph.selected.filter(|&i| self.graph.nodes.get(i).is_some_and(|n| n.kind == Kind::Emitter)).or_else(|| self.graph.first(Kind::Emitter))
    }

    // ---------- top menu ----------
    fn menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").exact_height(26.0).frame(egui::Frame::new().fill(MENU_BG).inner_margin(egui::Margin::symmetric(8, 2))).show(ctx, |ui| {
            let full = ui.max_rect();
            egui::menu::bar(ui, |ui| {
                home::logo(ui, 20.0);
                if ui.add(egui::Button::new("🏠").frame(false)).on_hover_text("Home, presets and projects").clicked() {
                    self.home.page = home::Page::Home;
                    self.show_home = true;
                }
                ui.menu_button("File", |ui| {
                    if ui.add(egui::Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                        self.graph = Graph::default();
                        self.project = None;
                        self.restart();
                        self.reset_history();
                        ui.close_menu();
                    }
                    if ui.button("Home / Projects…").clicked() {
                        self.home.page = home::Page::Projects;
                        self.show_home = true;
                        ui.close_menu();
                    }
                    if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).clicked() {
                        ui.close_menu();
                        self.open_project();
                    }
                    if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                        ui.close_menu();
                        self.save_project(false);
                    }
                    if ui.button("Save as…").clicked() {
                        ui.close_menu();
                        self.save_project(true);
                    }
                    ui.separator();
                    if ui.button("Export…").clicked() {
                        self.view_tab = 2;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let has_sel = !self.graph.picked.is_empty() || self.graph.selected.is_some();
                    let mut run = None;
                    for (label, key, cmd, on) in [
                        ("Undo", "Ctrl+Z", Cmd::Undo, !self.undo.is_empty()),
                        ("Redo", "Ctrl+Y", Cmd::Redo, !self.redo.is_empty()),
                        ("", "", Cmd::Undo, false),
                        ("Cut", "Ctrl+X", Cmd::Cut, has_sel),
                        ("Copy", "Ctrl+C", Cmd::Copy, has_sel),
                        ("Paste", "Ctrl+V", Cmd::Paste, !self.clipboard.is_empty()),
                        ("Duplicate", "Ctrl+D", Cmd::Duplicate, has_sel),
                        ("Delete", "Del", Cmd::Delete, has_sel),
                        ("", "", Cmd::Undo, false),
                        ("Select all", "Ctrl+A", Cmd::SelectAll, true),
                    ] {
                        if label.is_empty() {
                            ui.separator();
                        } else if ui.add_enabled(on, egui::Button::new(label).shortcut_text(key)).clicked() {
                            run = Some(cmd);
                            ui.close_menu();
                        }
                    }
                    if ui.add_enabled(has_sel, egui::Button::new("Deselect").shortcut_text("Esc")).clicked() {
                        self.graph.clear_selection();
                        ui.close_menu();
                    }
                    if let Some(c) = run {
                        self.edit(ui.ctx(), c);
                    }
                    ui.separator();
                    let sel = self.graph.selected;
                    if ui.add_enabled(sel.is_some(), egui::Button::new("Reset node settings")).clicked() {
                        let n = &mut self.graph.nodes[sel.unwrap()];
                        n.params = template(n.kind).params;
                        ui.close_menu();
                    }
                    if ui.button("Randomize marked settings").clicked() {
                        self.graph.randomize();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Simulation", |ui| {
                    if ui.button(if self.playing { "Pause  (Space)" } else { "Play  (Space)" }).clicked() {
                        self.playing = !self.playing;
                        ui.close_menu();
                    }
                    if ui.button("Restart  (R)").clicked() {
                        self.restart();
                        ui.close_menu();
                    }
                    ui.checkbox(&mut self.looping, "Loop at render duration");
                    ui.separator();
                    if ui.button("Browse presets…").clicked() {
                        self.home.page = home::Page::Presets;
                        self.show_home = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Settings", |ui| {
                    ui.label("Interface size");
                    for (l, z) in [("Small", 0.85), ("Normal", 1.0), ("Large", 1.2)] {
                        if ui.button(l).clicked() {
                            ui.ctx().set_zoom_factor(z);
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    ui.checkbox(&mut self.hq, "HQ render mode (Ctrl+3)").on_hover_text("Full resolution and very fine ray steps. While paused, the frame keeps refining (up to 256 passes) for a clean, grain-free still.");
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Shortcuts & about").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
            let name = self.project.as_ref().and_then(|p| p.file_name()).map_or("Default_Fire.fcproj".to_string(), |n| n.to_string_lossy().into());
            ui.painter().text(full.center(), egui::Align2::CENTER_CENTER, name, FontId::proportional(11.5), Color32::from_rgb(150, 152, 154));
        });
    }

    // ---------- viewport ----------
    fn viewport(&mut self, ui: &mut egui::Ui) {
        let head = pane_header(ui, |ui| {
            for (i, t) in ["Scene", "Render", "Export"].iter().enumerate() {
                let on = self.view_tab == i;
                if ui.add(egui::Button::new(RichText::new(*t).color(if on { TEXT } else { DIM })).frame(false)).clicked() {
                    self.view_tab = i;
                }
            }
            ui.add_space(8.0);
            let views = ["Default", "Front", "Side", "Top"];
            egui::ComboBox::from_id_salt("camview").selected_text(views[self.cam_view]).width(80.0).show_ui(ui, |ui| {
                for (i, v) in views.iter().enumerate() {
                    if ui.selectable_label(self.cam_view == i, *v).clicked() {
                        self.set_view(i);
                    }
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.show_axes, "Axis widget");
                ui.checkbox(&mut self.show_bounds, "Domain bounds");
                ui.checkbox(&mut self.render_shapes, "Emitter shapes");
            });
        });
        let rect = Rect::from_min_max(pos2(head.min.x, head.max.y), ui.max_rect().max);
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        if self.view_tab == 2 {
            self.export_panel(ui, rect);
            return;
        }
        let b = self.built();
        let ppp = ui.ctx().pixels_per_point();
        let hq = self.hq_active();
        let q = if hq { 1.0 } else { QUALITY[self.quality].1 };
        let (w, h) = (((rect.width() * ppp / q) as usize).max(8), ((rect.height() * ppp / q) as usize).max(8));
        if self.view.as_ref().is_none_or(|(t, _)| t.w != w || t.h != h) {
            let t = Target::new(&self.gpu, w, h);
            let mut ren = self.rs.renderer.write();
            let id = match self.view.take() {
                Some((_, id)) => {
                    ren.update_egui_texture_from_wgpu_texture(&self.gpu.device, &t.srgb_view, wgpu::FilterMode::Linear, id);
                    id
                }
                None => ren.register_native_texture(&self.gpu.device, &t.srgb_view, wgpu::FilterMode::Linear),
            };
            drop(ren);
            self.view = Some((t, id));
        }
        // Noise control: blend with the previous image while playing; when paused keep
        // refining the same frame (HQ: up to HQ_PASSES passes). Any change starts fresh.
        let sig = format!("{}{}{}{}x{}", self.graph.save(), self.lit, self.ortho, w, h);
        let same_view = sig == self.hq_sig;
        let same_frame = self.sim.frame == self.last_frame;
        let cap = if hq { HQ_PASSES } else { 24 };
        let (weight, draw) = if !same_view {
            self.hq_samples = 0;
            (1.0, true)
        } else if same_frame {
            self.hq_samples = self.hq_samples.saturating_add(1);
            (1.0 / (self.hq_samples + 1) as f32, self.hq_samples < cap)
        } else {
            self.hq_samples = 0;
            (TEMPORAL, true)
        };
        self.hq_sig = sig;
        self.last_frame = self.sim.frame;
        let (target, tex_id) = self.view.as_ref().unwrap();
        if draw {
            let seed = self.sim.frame.wrapping_mul(7).wrapping_add(self.hq_samples);
            self.sim.render(&self.gpu, target, &b.cam, &b.shade, seed, weight);
        }
        ui.painter().image(*tex_id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        let resp = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
        let aspect = rect.width() / rect.height();
        let proj = |p: V3| b.cam.project(p, aspect).map(|s| pos2(rect.min.x + s[0] * rect.width(), rect.min.y + s[1] * rect.height()));
        let painter = ui.painter_at(rect);
        let em = self.emitter_index();

        if self.view_tab == 0 {
            if self.show_bounds {
                let (o, s) = (self.sim.origin(), self.sim.size());
                let c = |i: usize| [o[0] + s[0] * (i & 1) as f32, o[1] + s[1] * ((i >> 1) & 1) as f32, o[2] + s[2] * ((i >> 2) & 1) as f32];
                for (a, bb) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    if let (Some(p), Some(q)) = (proj(c(a)), proj(c(bb))) {
                        painter.line_segment([p, q], Stroke::new(1.0, Color32::from_gray(70)));
                    }
                }
            }
            // emitter shape + move gizmo
            if let Some(ei) = em {
                let n = &self.graph.nodes[ei];
                let pos = ui2sim(n.c("Position"));
                if self.render_shapes && n.b("Show shape") {
                    let size = b.sp.emitters.first().and_then(|e| e.shapes.first()).map_or(0.22, |s| s.half[0]);
                    let sc = n.c("Shape color");
                    let col = Color32::from_rgb((sc[0] * 255.0) as u8, (sc[1] * 255.0) as u8, (sc[2] * 255.0) as u8);
                    for axis in 0..3 {
                        let pts: Vec<Pos2> = (0..=32)
                            .filter_map(|k| {
                                let a = k as f32 / 32.0 * std::f32::consts::TAU;
                                let (c, s) = (a.cos() * size, a.sin() * size);
                                let d = match axis { 0 => [c, s, 0.0], 1 => [c, 0.0, s], _ => [0.0, c, s] };
                                proj(render::add(pos, d))
                            })
                            .collect();
                        painter.add(egui::Shape::line(pts, Stroke::new(1.0, col.gamma_multiply(0.6))));
                    }
                }
                if let Some(c) = proj(pos) {
                    let arm = 0.45;
                    for (d, col) in [
                        (ui2sim([0.0, 0.0, arm]), Color32::from_rgb(40, 110, 255)),
                        (ui2sim([0.0, arm, 0.0]), Color32::from_rgb(30, 200, 40)),
                        (ui2sim([arm, 0.0, 0.0]), Color32::from_rgb(235, 30, 50)),
                    ] {
                        if let Some(e) = proj(render::add(pos, d)) {
                            painter.line_segment([c, e], Stroke::new(2.0, col));
                            let dir = (e - c).normalized();
                            let side = vec2(-dir.y, dir.x) * 5.0;
                            painter.add(egui::Shape::convex_polygon(vec![e + dir * 12.0, e + side, e - side], col, Stroke::NONE));
                        }
                    }
                    painter.circle_filled(c, 3.0, Color32::WHITE);
                }
            }
            // tool palette
            for (i, tip) in ["Move emitter", "Orbit camera", "Resize emitter"].iter().enumerate() {
                let r = Rect::from_min_size(rect.min + vec2(10.0, 12.0 + i as f32 * 30.0), vec2(26.0, 26.0));
                let tr = ui.interact(r, ui.id().with(("tool", i)), Sense::click()).on_hover_text(*tip);
                if tr.clicked() {
                    self.tool = i;
                }
                if self.tool == i {
                    painter.rect_filled(r, CornerRadius::same(2), ACCENT);
                }
                let (c, s) = (r.center(), Stroke::new(1.4, TEXT));
                match i {
                    0 => {
                        painter.line_segment([c - vec2(7.0, 0.0), c + vec2(7.0, 0.0)], s);
                        painter.line_segment([c - vec2(0.0, 7.0), c + vec2(0.0, 7.0)], s);
                        for d in [vec2(7.0, 0.0), vec2(-7.0, 0.0), vec2(0.0, 7.0), vec2(0.0, -7.0)] {
                            let side = vec2(-d.y, d.x) * 0.35;
                            painter.line_segment([c + d, c + d * 0.6 + side], s);
                            painter.line_segment([c + d, c + d * 0.6 - side], s);
                        }
                    }
                    1 => {
                        painter.circle_stroke(c, 6.0, s);
                        painter.circle_filled(c + vec2(6.0, 0.0), 2.0, TEXT);
                    }
                    _ => {
                        painter.line_segment([c + vec2(-6.0, -6.0), c + vec2(-6.0, 6.0)], s);
                        painter.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, 6.0)], s);
                        painter.line_segment([c + vec2(-6.0, 6.0), c + vec2(5.0, -5.0)], s);
                    }
                }
            }
        }

        // mouse: tool drag with left, orbit with right/middle, zoom with wheel
        let (_, f, r, u) = b.cam.basis();
        let _ = f;
        let delta = resp.drag_delta();
        let orbit = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (resp.dragged_by(egui::PointerButton::Primary) && (self.tool == 1 || self.view_tab == 1));
        if orbit && !self.lock_camera {
            if let Some(c) = self.camera_node() {
                let (yaw, pitch) = (c.f("Yaw") - delta.x * 0.4, (c.f("Pitch") + delta.y * 0.3).clamp(-89.0, 89.0));
                c.set("Yaw", Val::F(((yaw + 180.0).rem_euclid(360.0)) - 180.0));
                c.set("Pitch", Val::F(pitch));
            }
        } else if resp.dragged_by(egui::PointerButton::Primary) && self.view_tab == 0 {
            if let Some(ei) = em {
                let world_per_px = b.cam.dist * (b.cam.fov.to_radians() * 0.5).tan() * 2.0 / rect.height();
                let n = &mut self.graph.nodes[ei];
                if self.tool == 0 {
                    let p = ui2sim(n.c("Position"));
                    let p = render::add(p, render::add(render::mul(r, delta.x * world_per_px), render::mul(u, -delta.y * world_per_px)));
                    let p = sim2ui(p).map(|v| v.clamp(-1.0, 2.0));
                    n.set("Position", Val::V(p));
                } else if let Some(si) = self.graph.wires.iter().find(|w| w.to == ei && w.inp == 0).map(|w| w.from) {
                    let s = &mut self.graph.nodes[si];
                    s.set("Size", Val::F((s.f("Size") - delta.y * world_per_px * 0.5).clamp(0.02, 1.0)));
                }
            }
        }
        if resp.hovered() && !self.lock_camera {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(c) = self.camera_node() {
                    c.set("Distance", Val::F((c.f("Distance") * (-scroll * 0.002).exp()).clamp(1.0, 12.0)));
                }
            }
        }

        resp.context_menu(|ui| {
            if ui.add(egui::Button::new(if self.playing { "Pause" } else { "Play" }).shortcut_text("Space")).clicked() {
                self.playing = !self.playing;
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Restart simulation").shortcut_text("R")).clicked() {
                self.restart();
                ui.close_menu();
            }
            ui.separator();
            ui.menu_button("Camera view", |ui| {
                for (i, v) in ["Default", "Front", "Side", "Top"].iter().enumerate() {
                    if ui.button(*v).clicked() {
                        self.set_view(i);
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Reset camera").clicked() {
                self.set_view(0);
                ui.close_menu();
            }
            ui.checkbox(&mut self.lock_camera, "Lock camera");
            ui.separator();
            ui.checkbox(&mut self.hq, "HQ render (Ctrl+3)");
            ui.checkbox(&mut self.lit, "Lit");
            ui.checkbox(&mut self.render_shapes, "Show emitter shapes");
            ui.checkbox(&mut self.show_bounds, "Show domain bounds");
            ui.separator();
            if ui.button("Save frame as PNG").clicked() {
                self.save_view_png();
                ui.close_menu();
            }
        });

        // axis widget
        if self.show_axes {
            let c = pos2(rect.min.x + 30.0, rect.max.y - 30.0);
            for (axis, col, label) in [([0.0, 0.0, 1.0], Color32::from_rgb(40, 110, 255), "Z"), ([0.0, 1.0, 0.0], Color32::from_rgb(30, 200, 40), "Y"), ([1.0, 0.0, 0.0], Color32::from_rgb(235, 30, 50), "X")] {
                let d = ui2sim(axis);
                let e = c + vec2(render::dot(d, r), -render::dot(d, u)) * 20.0;
                let ne = c - vec2(render::dot(d, r), -render::dot(d, u)) * 20.0;
                painter.line_segment([c, e], Stroke::new(1.5, col));
                painter.circle_filled(ne, 4.0, col.gamma_multiply(0.45));
                painter.circle_filled(e, 6.0, col);
                painter.text(e, egui::Align2::CENTER_CENTER, label, FontId::proportional(8.0), Color32::BLACK);
            }
        }

        // overlay settings (top right)
        if self.view_tab == 0 {
            let r = Rect::from_min_size(pos2(rect.max.x - 185.0, rect.min.y + 22.0), vec2(170.0, 120.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(r));
            egui::Grid::new("overlay").num_columns(2).spacing(vec2(8.0, 5.0)).show(&mut cui, |ui| {
                let small = |s: &str| RichText::new(s).size(10.5).color(TEXT);
                ui.label(small("Render Quality"));
                stepper(ui, &mut self.quality, &["Low", "Medium", "High"]);
                ui.end_row();
                ui.label(small("View Mode"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (on, l) in [(true, "Lit"), (false, "Unlit")] {
                        let sel = self.lit == on;
                        let btn = egui::Button::new(RichText::new(l).size(10.5).color(if sel { TEXT } else { Color32::from_rgb(30, 30, 30) }))
                            .fill(if sel { Color32::from_rgb(20, 55, 82) } else { Color32::from_rgb(203, 203, 204) })
                            .corner_radius(0.0)
                            .min_size(vec2(40.0, 18.0));
                        if ui.add(btn).clicked() {
                            self.lit = on;
                        }
                    }
                });
                ui.end_row();
                ui.label(small("Projection"));
                let mut pi = self.ortho as usize;
                stepper(ui, &mut pi, &["Perspective", "Ortho"]);
                self.ortho = pi == 1;
                ui.end_row();
                ui.label(small("Render Shapes"));
                ui.checkbox(&mut self.render_shapes, "");
                ui.end_row();
                ui.label(small("Lock Camera"));
                ui.checkbox(&mut self.lock_camera, "");
                ui.end_row();
            });
        }
        let info = if hq {
            if self.playing { "HQ render · pause to refine".to_string() } else { format!("HQ render · {} / {HQ_PASSES} passes", (self.hq_samples + 1).min(HQ_PASSES)) }
        } else {
            format!("{} quality · {}×{} px", QUALITY[self.quality].0, w, h)
        };
        painter.text(rect.right_bottom() - vec2(10.0, 10.0), egui::Align2::RIGHT_BOTTOM, info, FontId::proportional(11.0), if hq { ACCENT } else { DIM });
    }

    fn export_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let r = rect.shrink(24.0);
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let ui = &mut cui;
        ui.label(RichText::new("Export").size(15.0));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Output folder");
            ui.add(egui::TextEdit::singleline(&mut self.export_dir).desired_width(260.0));
            if ui.button("Choose…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.export_dir = d.display().to_string();
                }
            }
        });
        ui.add_space(6.0);
        if let Some(n) = self.graph.nodes.iter_mut().find(|n| n.kind == Kind::Render) {
            let id = n.id;
            for p in n.params.iter_mut() {
                graph::param_row(ui, p, id);
            }
        }
        ui.add_space(8.0);
        let busy = self.export.is_some();
        let exports: Vec<(usize, String)> = self.graph.nodes.iter().enumerate().filter(|(_, n)| n.kind.group() == Some("Export") && n.enabled).map(|(i, n)| (i, n.title().to_string())).collect();
        if !exports.is_empty() {
            ui.label(RichText::new("Export nodes in the graph").color(TEXT));
            ui.horizontal_wrapped(|ui| {
                for (i, t) in &exports {
                    if ui.add_enabled(!busy, egui::Button::new(format!("⬇ {t}"))).clicked() {
                        self.export_node(*i);
                    }
                }
            });
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Export image sequence")).on_hover_text("One PNG per frame over the render duration").clicked() {
                self.start_export(Job::Sequence);
            }
            if ui.add_enabled(!busy, egui::Button::new("Export flipbook")).on_hover_text("All frames tiled into one sprite sheet").clicked() {
                self.start_export(Job::Flipbook);
            }
            if ui.add_enabled(!busy, egui::Button::new("Export volume (current frame)")).on_hover_text("3D density, temperature and fuel grids (.vol)").clicked() {
                self.start_export(Job::Volume);
            }
        });
        if let Some(job) = &self.export {
            ui.add(egui::ProgressBar::new(job.progress()).desired_width(400.0).show_percentage());
        }
        if !self.status.is_empty() {
            ui.label(RichText::new(&self.status).weak());
        }
    }

    // ---------- timeline ----------
    fn timeline(&mut self, ui: &mut egui::Ui) {
        pane_header(ui, |ui| {
            ui.label(RichText::new("Timeline").color(TEXT));
        });
        let duration = self.built().duration;
        let frame = self.sim.frame;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let icon = |s: &str, on: bool| egui::Button::new(RichText::new(s).color(if on { ACCENT } else { TEXT })).frame(false);
            if ui.add(icon("↻", self.looping)).on_hover_text("Loop at render duration").clicked() {
                self.looping = !self.looping;
            }
            if ui.add(icon("⏮", false)).on_hover_text("Restart (R)").clicked() {
                self.restart();
            }
            if ui.add(icon("⏪", false)).on_hover_text("Back one frame").clicked() {
                self.seek(frame.saturating_sub(1));
            }
            if ui.add(icon(if self.playing { "⏸" } else { "▶" }, self.playing)).on_hover_text("Play/Pause (Space)").clicked() {
                self.playing = !self.playing;
            }
            if ui.add(icon("⏩", false)).on_hover_text("Forward one frame").clicked() {
                self.seek(frame + 1);
            }
            if ui.add(icon("⏭", false)).on_hover_text("Go to end of render duration").clicked() {
                self.seek(duration);
            }
            ui.label(RichText::new(format!("{:.3} s", frame as f32 / FPS)).color(DIM));
            let mut f = frame;
            if ui.add(egui::DragValue::new(&mut f).suffix(" f").range(0..=2000)).changed() {
                self.seek(f);
            }
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 12.0), Sense::hover());
            ui.painter().rect_filled(r, 0.0, FIELD_BG);
            let k = (frame as f32 / duration as f32).min(1.0);
            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * k, r.height())), 0.0, TRACK_BAR);
        });
        ui.add_space(2.0);
        let area = Rect::from_min_max(pos2(ui.max_rect().min.x, ui.cursor().min.y), ui.max_rect().max);
        let split = area.min.x + (area.width() * 0.26).max(150.0);
        let painter = ui.painter_at(area);
        painter.line_segment([pos2(split, area.min.y), pos2(split, area.max.y)], Stroke::new(1.0, Color32::from_rgb(26, 27, 29)));
        // search + tracks list
        let mut left = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(area.min + vec2(6.0, 0.0), pos2(split - 6.0, area.max.y))));
        left.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Search").desired_width(split - area.min.x - 14.0));
        left.add_space(6.0);
        let show = self.search.is_empty() || "render duration".contains(&self.search.to_lowercase());
        if show {
            left.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                let c = r.center();
                ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)], TEXT, Stroke::NONE));
                ui.label(RichText::new("Render").color(TEXT));
            });
            left.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Duration").color(DIM));
                if let Some(n) = self.graph.node_mut(Kind::Render) {
                    let mut d = n.f("Duration");
                    if ui.add(egui::DragValue::new(&mut d).range(1.0..=400.0).suffix(" f").fixed_decimals(0)).changed() {
                        n.set("Duration", Val::F(d));
                    }
                }
            });
        }
        // ruler
        let span = if frame < 400 { 400.0 } else { (frame as f32 / 100.0).ceil() * 100.0 + 40.0 };
        let ruler = Rect::from_min_max(pos2(split + 8.0, area.min.y), pos2(area.max.x - 8.0, area.min.y + 24.0));
        let fx = |f: f32| ruler.min.x + f / span * ruler.width();
        let step = if span <= 400.0 { 20 } else { 100 };
        let mut t = 0;
        while t as f32 <= span {
            let x = fx(t as f32);
            painter.text(pos2(x, ruler.min.y + 2.0), egui::Align2::CENTER_TOP, t.to_string(), FontId::proportional(9.5), TEXT);
            painter.line_segment([pos2(x, ruler.max.y - 6.0), pos2(x, ruler.max.y)], Stroke::new(1.0, DIM));
            for k in 1..4 {
                let xs = fx(t as f32 + k as f32 * step as f32 / 4.0);
                if xs < ruler.max.x {
                    painter.line_segment([pos2(xs, ruler.max.y - 3.0), pos2(xs, ruler.max.y)], Stroke::new(1.0, Color32::from_rgb(60, 62, 64)));
                }
            }
            t += step;
        }
        let rr = ui.interact(ruler, ui.id().with("ruler"), Sense::click_and_drag());
        rr.context_menu(|ui| {
            if ui.button(if self.playing { "Pause" } else { "Play" }).clicked() {
                self.playing = !self.playing;
                ui.close_menu();
            }
            if ui.button("Go to start").clicked() {
                self.restart();
                ui.close_menu();
            }
            if ui.button("Go to end of render duration").clicked() {
                self.seek(duration);
                ui.close_menu();
            }
            ui.checkbox(&mut self.looping, "Loop at render duration");
        });
        if let Some(p) = rr.interact_pointer_pos() {
            if rr.clicked() || rr.dragged() {
                self.seek(((p.x - ruler.min.x) / ruler.width() * span).round().max(0.0) as u32);
            }
        }
        // duration track bar (drag to change)
        if show {
            let y = ruler.max.y + 32.0;
            let bar = Rect::from_min_max(pos2(fx(0.0), y), pos2(fx(duration as f32), y + 12.0));
            painter.rect_filled(bar, CornerRadius::same(3), TRACK_BAR);
            let track = Rect::from_min_max(pos2(ruler.min.x, y - 2.0), pos2(ruler.max.x, y + 14.0));
            let tr = ui.interact(track, ui.id().with("durtrack"), Sense::drag()).on_hover_text("Drag to set the render duration");
            if let (true, Some(p)) = (tr.dragged(), tr.interact_pointer_pos()) {
                let d = ((p.x - ruler.min.x) / ruler.width() * span).round().clamp(1.0, 400.0);
                if let Some(n) = self.graph.node_mut(Kind::Render) {
                    n.set("Duration", Val::F(d));
                }
            }
        }
        // playhead
        let x = fx(frame as f32);
        painter.line_segment([pos2(x, ruler.min.y), pos2(x, area.max.y)], Stroke::new(1.0, ACCENT));
        painter.rect_filled(Rect::from_center_size(pos2(x, ruler.max.y - 2.0), vec2(7.0, 7.0)), CornerRadius::same(1), ACCENT);
        // claim the whole pane so the panel keeps its height
        ui.allocate_rect(area, Sense::hover());
    }

    // ---------- details ----------
    fn details(&mut self, ui: &mut egui::Ui) {
        pane_header(ui, |ui| {
            for (i, t) in ["Node Details", "Favorites", "Timeline", "Randomized"].iter().enumerate() {
                let on = self.graph.details_tab == i;
                if ui.add(egui::Button::new(RichText::new(*t).color(if on { TEXT } else { DIM })).frame(false)).clicked() {
                    self.graph.details_tab = i;
                }
            }
        });
        self.graph.details(ui);
    }
}

/// Thin title strip at the top of a pane; returns its rect.
fn pane_header(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let full = ui.max_rect();
    let r = Rect::from_min_size(full.min, vec2(full.width(), 22.0));
    ui.painter().rect_filled(r, 0.0, PANEL_BG);
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, Color32::from_rgb(26, 27, 29)));
    for k in 0..3 {
        ui.painter().circle_filled(pos2(r.min.x + 6.0, r.min.y + 7.0 + k as f32 * 4.0), 0.9, DIM);
    }
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(14.0, 1.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    add(&mut cui);
    ui.advance_cursor_after_rect(r);
    r
}

fn stepper(ui: &mut egui::Ui, i: &mut usize, opts: &[&str]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let s = |t: &str| egui::Button::new(RichText::new(t).size(10.5)).fill(FIELD_BG).corner_radius(0.0).min_size(vec2(14.0, 18.0));
        if ui.add(s("‹")).clicked() {
            *i = (*i + opts.len() - 1) % opts.len();
        }
        let mid = egui::Button::new(RichText::new(opts[*i]).size(10.5)).fill(FIELD_BG).corner_radius(0.0).min_size(vec2(64.0, 18.0));
        if ui.add(mid).clicked() {
            *i = (*i + 1) % opts.len();
        }
        if ui.add(s("›")).clicked() {
            *i = (*i + 1) % opts.len();
        }
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --screenshot: draw ~120 frames, grab the window, save and quit
        self.frames += 1;
        if let Some(path) = self.shot.clone() {
            if self.frames == 120 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let img = ctx.input(|i| i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            }));
            if let Some(img) = img {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let [w, h] = img.size;
                let _ = image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        if !self.show_home {
            self.edit_shortcuts(ctx);
        }
        // shortcuts
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Space) {
                    self.playing = !self.playing;
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::Num3) {
                    self.hq = !self.hq;
                }
            });
            if ctx.input(|i| i.key_pressed(egui::Key::R) && !i.modifiers.ctrl) {
                self.restart();
            }
        }

        // advance a running export
        if let Some(job) = &mut self.export {
            match job.advance(&self.gpu, 4) {
                Ok(None) => {}
                Ok(Some(msg)) => {
                    self.status = msg;
                    self.export = None;
                }
                Err(e) => {
                    self.status = format!("Export failed: {e}");
                    self.export = None;
                }
            }
        }

        // simulate
        let b = self.built();
        if b.res != self.sim.res || (b.domain - self.sim.n[0] as f32 * self.sim.h).abs() > 1e-4 || std::mem::take(&mut self.graph.import_request) {
            self.sim = GpuSim::new(&self.gpu, b.res, b.domain);
            self.load_imports(&b);
        }
        if self.seek_to.is_some() {
            self.advance_seek();
        } else if self.playing {
            self.sim.step(&self.gpu, &b.sp);
            if self.looping && self.sim.frame >= b.duration {
                self.restart();
            }
        }

        if self.show_home {
            if let Some(i) = self.home.next_missing_thumb() {
                self.make_thumb(i);
            }
            if let Some(a) = self.home.ui(ctx, VERSION) {
                self.home_action(a);
            }
            ctx.request_repaint();
            return;
        }

        self.menu(ctx);
        egui::TopBottomPanel::bottom("status").exact_height(20.0).frame(egui::Frame::new().fill(MENU_BG).inner_margin(egui::Margin::symmetric(10, 2))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let hint = |ui: &mut egui::Ui, k: &str, t: &str| {
                    ui.label(RichText::new(k).size(10.0).color(TEXT));
                    ui.label(RichText::new(t).size(10.0).color(DIM));
                    ui.add_space(10.0);
                };
                hint(ui, "Ctrl+G", "Create Comment");
                hint(ui, "H", "Create Note");
                hint(ui, "Right-click", "Add Node");
                hint(ui, "Space", "Play/Pause");
                hint(ui, "Ctrl+3", "HQ Render");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(VERSION).size(10.0).color(DIM));
                    let n = self.sim.n;
                    ui.label(RichText::new(format!("{}×{}×{} grid · {:.0} fps", n[0], n[1], n[2], 1.0 / ctx.input(|i| i.stable_dt).max(1e-3))).size(10.0).color(DIM));
                    if !self.status.is_empty() && self.view_tab != 2 {
                        ui.label(RichText::new(&self.status).size(10.0).color(DIM));
                    }
                });
            });
        });

        let screen = ctx.screen_rect();
        let pane = egui::Frame::new().fill(PANEL_BG);
        let lower_h = screen.height() * 0.47;
        egui::SidePanel::left("left").resizable(true).default_width(screen.width() * 0.5).frame(pane).show(ctx, |ui| {
            egui::TopBottomPanel::bottom("timeline").resizable(true).default_height(lower_h).frame(pane).show_inside(ui, |ui| self.timeline(ui));
            egui::CentralPanel::default().frame(pane).show_inside(ui, |ui| self.viewport(ui));
        });
        egui::CentralPanel::default().frame(pane).show(ctx, |ui| {
            egui::TopBottomPanel::bottom("details").resizable(true).default_height(lower_h).frame(pane).show_inside(ui, |ui| self.details(ui));
            egui::CentralPanel::default().frame(egui::Frame::new().fill(graph::GRAPH_BG)).show_inside(ui, |ui| {
                pane_header(ui, |ui| {
                    ui.label(RichText::new("Node Graph").color(TEXT));
                });
                let r = Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
                let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(r));
                self.graph.canvas(&mut cui);
            });
        });
        if let Some(i) = self.graph.export_request.take() {
            self.export_node(i);
        }
        if let Some(c) = self.graph.cmd.take() {
            self.edit(ctx, c);
        }
        self.track_undo(ctx);

        // windows
        egui::Window::new("About FireCraft").open(&mut self.show_about).collapsible(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                home::logo(ui, 48.0);
                ui.label(RichText::new(format!("FireCraft {VERSION}
Real-time fire and smoke")).size(14.0));
            });
            ui.separator();
            for (k, t) in [
                ("Space", "Play / pause"),
                ("R", "Restart simulation"),
                ("Ctrl+3", "HQ render: full detail, refines while paused"),
                ("Ctrl+G / H", "Create comment / note (over the graph)"),
                ("Delete", "Delete selected nodes"),
                ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
                ("Ctrl+X / C / V", "Cut / copy / paste nodes"),
                ("Ctrl+D, Ctrl+A", "Duplicate, select all"),
                ("Ctrl+click, Shift+drag", "Add to selection, box select"),
                ("Right-click graph", "Add node"),
                ("Drag port → port", "Connect nodes"),
                ("Right/middle drag", "Orbit camera; wheel zooms"),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized([130.0, 16.0], egui::Label::new(RichText::new(k).strong()));
                    ui.label(t);
                });
            }
        });
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Simulates the default scene on the GPU, renders it to shots/test.png and runs the exports.
    #[test]
    fn default_scene_renders_fire_and_exports() {
        let gpu = gpu::test_gpu();
        let g = Graph::default();
        let b = build(&g, true, false, 0.7);
        let mut s = GpuSim::new(&gpu, b.res, b.domain);
        for _ in 0..120 {
            s.step(&gpu, &b.sp);
        }
        let t = Target::new(&gpu, 360, 300);
        s.render(&gpu, &t, &b.cam, &b.shade, 0, 1.0);
        let px = gpu.read_target(&t);
        let _ = std::fs::create_dir_all("shots");
        save_png(std::path::Path::new("shots/test.png"), &px, t.w, t.h).unwrap();
        let bright = px.chunks(4).filter(|p| p[0] > 200 && p[1] > 90).count();
        assert!(bright > 200, "expected visible flames, got {bright} bright pixels");

        let base = std::env::temp_dir().join("firecraft_test").display().to_string();
        let mut small = b.clone();
        small.duration = 4;
        small.out = [32, 32];
        small.cols = 2;
        let mut job = ExportJob::new(&gpu, small, (g.clone(), true, false, 0.7), true, &base).unwrap();
        while job.advance(&gpu, 4).unwrap().is_none() {}
        assert!(export_volume(&gpu, &s, &base).unwrap().contains("volume"));
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod stress {
    use super::*;
    /// Writes shots/stress.fcproj: the default fire at Ultra detail (for memory checks).
    #[test]
    #[ignore]
    fn write_stress_project() {
        let mut g = Graph::default();
        g.node_mut(Kind::Simulation).unwrap().set("Resolution", Val::Ch(3));
        std::fs::create_dir_all("shots").unwrap();
        std::fs::write("shots/stress.fcproj", g.save()).unwrap();
    }
}

#[cfg(test)]
mod look {
    use super::*;
    /// Close-up look-development renders (run with --ignored), like a zoomed-in viewport.
    #[test]
    #[ignore]
    fn look_renders() {
        let gpu = gpu::test_gpu();
        let mut g = Graph::default();
        presets::apply(&mut g, 0);
        g.node_mut(Kind::Simulation).unwrap().set("Resolution", Val::Ch(2));
        let cam = g.node_mut(Kind::Camera).unwrap();
        cam.set("Distance", Val::F(1.25));
        cam.set("Target height", Val::F(0.4));
        // experiment switches (comma list in FC_LOOK): nodetail, nojitter, nochaos, noemchaos
        let flags = std::env::var("FC_LOOK").unwrap_or_default();
        if flags.contains("nodetail") { g.node_mut(Kind::Shading).unwrap().set("Fine detail", Val::F(0.0)); }
        if flags.contains("nojitter") { g.node_mut(Kind::Emitter).unwrap().set("Velocity spread", Val::F(0.0)); }
        let by_title = |g: &mut Graph, t: &str| g.nodes.iter().position(|n| n.title == t).unwrap();
        if flags.contains("nochaos") { let i = by_title(&mut g, "Fire Chaos"); g.nodes[i].set("Strength", Val::F(0.0)); }
        if flags.contains("noemchaos") { let i = by_title(&mut g, "Emission Chaos"); g.nodes[i].set("Strength", Val::F(0.0)); }
        if flags.contains("novort") { g.node_mut(Kind::Simulation).unwrap().set("Vorticity", Val::F(0.0)); }
        if flags.contains("lowvort") { g.node_mut(Kind::Simulation).unwrap().set("Vorticity", Val::F(0.1)); }
        let step: f32 = std::env::var("FC_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.4);
        let b = build(&g, true, false, step);
        let mut s = GpuSim::new(&gpu, b.res, b.domain);
        let t = Target::new(&gpu, 720, 600);
        for f in 1..=150u32 {
            s.step(&gpu, &b.sp);
            if f == 90 {
                // raw temperature slice through the middle of the domain (no rendering involved)
                let sc = s.read_scalars(&gpu);
                let [nx, ny, nz] = s.n;
                let z = nz / 2;
                let mut px = vec![0u8; nx * ny * 4];
                for y in 0..ny {
                    for x in 0..nx {
                        let t = sc[x + nx * (y + ny * z)][0];
                        let v = (t / 3.0).clamp(0.0, 1.0);
                        let o = ((ny - 1 - y) * nx + x) * 4;
                        px[o] = (v * 255.0) as u8;
                        px[o + 1] = (v * v * 200.0) as u8;
                        px[o + 2] = 0;
                        px[o + 3] = 255;
                    }
                }
                save_png(std::path::Path::new("shots/slice_90.png"), &px, nx, ny).unwrap();
            }
            if f == 90 || f == 150 {
                s.render(&gpu, &t, &b.cam, &b.shade, f, 1.0);
                let px = gpu.read_target(&t);
                save_png(std::path::Path::new(&format!("shots/close_{f}.png")), &px, t.w, t.h).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod preset_pictures {
    use super::*;
    /// Renders every preset once in high quality into assets/presets/ and writes
    /// src/preset_thumbs.rs, which builds the pictures into the app.
    /// Run with: cargo test render_preset_pictures -- --ignored
    #[test]
    #[ignore]
    fn render_preset_pictures() {
        let gpu = gpu::test_gpu();
        let only: Option<String> = std::env::var("FC_ONLY").ok();
        std::fs::create_dir_all("assets/presets").unwrap();
        let (w, h) = (640, 400);
        for (i, pr) in presets::PRESETS.iter().enumerate() {
            if only.as_ref().is_some_and(|o| !o.split(',').any(|w| pr.name.to_lowercase().contains(&w.trim().to_lowercase()))) {
                continue;
            }
            let mut g = Graph::default();
            presets::apply(&mut g, i);
            g.node_mut(Kind::Simulation).unwrap().set("Resolution", Val::Ch(2));
            let b0 = build(&g, true, false, 0.25);
            let mut s = GpuSim::new(&gpu, b0.res, b0.domain);
            for f in 0..pr.frame {
                let b = build(&modulate::apply(&g, f as f32 / FPS), true, false, 0.25);
                s.step(&gpu, &b.sp);
            }
            let b = build(&modulate::apply(&g, pr.frame as f32 / FPS), true, false, 0.25);
            let t = Target::new(&gpu, w, h);
            for pass in 0..12u32 {
                s.render(&gpu, &t, &b.cam, &b.shade, pass, 1.0 / (pass + 1) as f32);
            }
            let px = gpu.read_target(&t);
            save_png(std::path::Path::new(&format!("assets/presets/{}.png", slug(pr.name))), &px, w, h).unwrap();
        }
        // the list that embeds the pictures (only presets that have one)
        let mut code = String::from("// Generated by `cargo test render_preset_pictures -- --ignored`. Pre-rendered preset pictures.\npub const PICTURES: &[(&str, &[u8])] = &[\n");
        for pr in presets::PRESETS {
            let f = format!("assets/presets/{}.png", slug(pr.name));
            if std::path::Path::new(&f).exists() {
                code += &format!("    ({:?}, include_bytes!(\"../{}\")),\n", pr.name, f);
            }
        }
        code += "];\n";
        std::fs::write("src/preset_thumbs.rs", code).unwrap();
    }

    fn slug(name: &str) -> String {
        name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
    }
}

#[cfg(test)]
mod readme_media {
    use super::*;
    /// Frames for the README's animated preview (shots/anim/) and project files for UI shots.
    /// Run with: cargo test render_readme_media -- --ignored
    #[test]
    #[ignore]
    fn render_readme_media() {
        let gpu = gpu::test_gpu();
        std::fs::create_dir_all("shots/anim").unwrap();
        let picks = [("Campfire", 30u32), ("Mushroom Cloud", 60), ("Magic Portal", 90), ("Spirit Swirl", 90), ("Gas Burner", 30), ("Ground Explosion", 0)];
        for (k, (name, start)) in picks.iter().enumerate() {
            let i = presets::PRESETS.iter().position(|p| p.name == *name).unwrap();
            let mut g = Graph::default();
            presets::apply(&mut g, i);
            std::fs::write(format!("shots/readme_{k}.fcproj"), g.save()).unwrap();
            g.node_mut(Kind::Simulation).unwrap().set("Resolution", Val::Ch(2));
            let b0 = build(&g, true, false, 0.3);
            let mut s = GpuSim::new(&gpu, b0.res, b0.domain);
            let t = Target::new(&gpu, 360, 240);
            let total = start + 96;
            for f in 0..total {
                let b = build(&modulate::apply(&g, f as f32 / FPS), true, false, 0.3);
                s.step(&gpu, &b.sp);
                if f >= *start && (f - start) % 2 == 0 {
                    for pass in 0..4u32 {
                        s.render(&gpu, &t, &b.cam, &b.shade, f * 4 + pass, 1.0 / (pass + 1) as f32);
                    }
                    let px = gpu.read_target(&t);
                    save_png(std::path::Path::new(&format!("shots/anim/{k}_{:03}.png", (f - start) / 2)), &px, t.w, t.h).unwrap();
                }
            }
        }
    }
}
