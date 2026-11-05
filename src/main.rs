// Installed builds start without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gpu;
mod graph;
mod home;
mod modulate;
mod noise;
mod preset_thumbs;
mod presets;
mod render;
mod sim;
mod widgets;

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use eframe::{egui_wgpu, wgpu};
use gpu::{Gpu, GpuSim, Target};
use graph::{ACCENT, Cmd, DIM, Graph, Kind, Node, RPass, TEXT, Val, template};
use render::{Camera, Post, Shade, V3};
use sim::{Burst, Chaos, Domain, Emitter, FPS, Force, Particles, Shape, ShapeInst, SimParams, mat_mul, rotation_ui};
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
    dom: Domain,
    shade: Shade,
    post: Post,
    cam: Camera,
    duration: u32,
    /// The Render node (export settings).
    render: Node,
    /// Restart at the render duration (Simulation › Time Control › Looping Mode).
    looping: bool,
    /// Show the simulation bounds (Simulation › Visual › Enable gizmo).
    sim_gizmo: bool,
    /// Graph node of each entry in `sp.forces` (for the viewport force arrows).
    force_nodes: Vec<usize>,
    /// Import nodes: (file, density scale, temperature scale).
    imports: Vec<(String, f32, f32)>,
    /// Grid was reduced to fit the graphics card.
    reduced: bool,
}

/// Direction and relative size of a force at a point (sim coordinates), for the viewport arrows.
/// ponytail: approximates the shader's falloffs and ignores field masks; exact for Line and Noise.
fn force_vec(f: &Force, p: V3, t: f32) -> V3 {
    let len = |a: V3| render::dot(a, a).sqrt();
    let d = [p[0] - f.pos[0], p[1] - f.pos[1], p[2] - f.pos[2]];
    let r = len(d).max(1e-4);
    let axis = render::norm(mat_mul(&f.rot, f.dir));
    let v = match f.kind {
        0 => axis,
        1 => d.map(|x| -x / r * (1.0 - r / f.radius.max(0.01)).max(0.0)),
        3 => {
            let np = noise::NoiseParams { freq: f.scale, octaves: f.noise[0] as u32, lacunarity: f.noise[1], gain: f.noise[2], curl: f.noise[3] > 0.5, seed: f.seed, speed: f.speed };
            let v = noise::fbm(&np, p, t);
            [v[0] + f.bias[0], v[1] + f.bias[1], v[2] + f.bias[2]]
        }
        4 if f.extra as u32 == 3 => [0.0, 1.0, 0.0],
        4 if f.extra as u32 <= 1 => d.map(|x| x / r * if f.extra == 0.0 { 1.0 } else { -1.0 }),
        _ => {
            // swirl around the axis (toroidal, vector-field swirl, vorticles)
            let c = render::cross(axis, d);
            c.map(|x| x / len(c).max(1e-4))
        }
    };
    v.map(|x| x * f.strength)
}

/// Emission chaos (a Force: Noise wired into an emitter) in shader units.
fn chaos_of(n: &Node) -> Chaos {
    Chaos { strength: n.f("Amplitude") / 166.7, scale: 50.0 / n.f("Scale").max(0.5), speed: n.f("Animation speed") * 0.5 }
}

/// Shape: Noise node in shader units.
fn shape_noise_of(n: &Node) -> Chaos {
    Chaos { strength: n.f("Strength"), scale: n.f("Scale") * 6.0, speed: n.f("Speed") }
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
            let ch = shape_noise_of(n);
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
    let grey = (c[0] + c[1] + c[2]) / 3.0;
    c.map(|v| (grey + (v - grey) * sat).clamp(0.0, 1.0).powf(2.2))
}

/// Most voxels the solver will allocate (about 4 GB of graphics memory).
const MAX_CELLS: f32 = 48.0e6;

/// Grid, cell size and origin from the Simulation node (Domain tab).
fn domain_of(simn: &Node) -> (Domain, bool) {
    let vc = simn.c("Voxel count");
    let mut ui_n = vc.map(|x| x.round().clamp(8.0, 512.0));
    let mut h = (simn.f("Voxel size") / 100.0).max(1e-4);
    let total = ui_n[0] * ui_n[1] * ui_n[2];
    let reduced = total > MAX_CELLS;
    if reduced {
        // keep the domain's size, use fewer, larger voxels
        let f = (MAX_CELLS / total).cbrt();
        ui_n = ui_n.map(|x| (x * f).floor().max(8.0));
        h /= f;
    }
    let n = [ui_n[0] as usize, ui_n[2] as usize, ui_n[1] as usize]; // sim axes: x, up, z
    let ext = [n[0] as f32 * h, n[1] as f32 * h, n[2] as f32 * h];
    let z = domain_zero(simn, ext);
    let bp = ui2sim(simn.c("Bounds Position"));
    (Domain { n, h, origin: [z[0] + bp[0], z[1] + bp[1], z[2] + bp[2]] }, reduced)
}

/// Where the box's min corner sits before the Bounds Position offset (sim axes), from the
/// Zero position choices (Center / Min / Max). UI Y is mirrored into sim z.
fn domain_zero(simn: &Node, ext: V3) -> V3 {
    let zero = |choice: usize, len: f32, mirrored: bool| match (choice, mirrored) {
        (1, false) | (2, true) => 0.0,
        (2, false) | (1, true) => -len,
        _ => -len * 0.5,
    };
    [
        zero(simn.ch("Zero position on X"), ext[0], false),
        zero(simn.ch("Zero position on Z"), ext[1], false),
        zero(simn.ch("Zero position on Y"), ext[2], true),
    ]
}

/// Set the simulation box from its min corner and size (sim axes, metres): the voxel count
/// follows the size at the current voxel size, Bounds Position follows the corner.
fn set_domain_box(simn: &mut Node, origin: V3, ext: V3) {
    let h = (simn.f("Voxel size") / 100.0).max(1e-4);
    let n = ext.map(|e| (e / h).round().clamp(8.0, 512.0));
    let ext = n.map(|c| c * h);
    let z = domain_zero(simn, ext);
    simn.set("Voxel count", Val::V([n[0], n[2], n[1]]));
    simn.set("Bounds Position", Val::V(sim2ui([origin[0] - z[0], origin[1] - z[1], origin[2] - z[2]])));
}

/// Emitter positions (sim axes) in graph order, for emitter motion between frames.
fn emitter_positions(g: &Graph) -> Vec<V3> {
    g.nodes.iter().filter(|n| matches!(n.kind, Kind::Emitter | Kind::EmitterParticles)).map(|n| ui2sim(n.c("Position"))).collect()
}

fn build(g: &Graph, lit: bool, ortho: bool, step: f32) -> Built {
    build_moving(g, None, lit, ortho, step)
}

/// `prev`: emitter positions one frame earlier (moving emitters pass their motion to the gas).
fn build_moving(g: &Graph, prev: Option<&[V3]>, lit: bool, ortho: bool, step: f32) -> Built {
    let live = g.live();
    let find = |k: Kind| (0..g.nodes.len()).find(|&i| g.nodes[i].kind == k && live[i]);
    let first_of = |i: Option<usize>, port: &str, ks: &[Kind]| -> Option<&Node> {
        i.and_then(|i| g.inputs(&live, i, port).into_iter().map(|j| &g.nodes[j]).find(|n| ks.contains(&n.kind)))
    };
    let sim_i = find(Kind::Simulation);
    let simn = sim_i.map(|i| &g.nodes[i]).cloned().unwrap_or_else(|| template(Kind::Simulation));
    let (dom, reduced) = domain_of(&simn);
    let pct = |n: &Node, k: &str| n.f(k) / 100.0;

    let mut emitters = vec![];
    let mut forces = vec![];
    let mut force_nodes = vec![];
    let mut colliders = vec![];
    let mut imports = vec![];
    let mut points = vec![];
    let mut shown = vec![];
    let all_emitters: Vec<usize> = (0..g.nodes.len()).filter(|&i| matches!(g.nodes[i].kind, Kind::Emitter | Kind::EmitterParticles)).collect();
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
            let volume = n.kind == Kind::Emitter;
            let pos = ui2sim(n.c("Position"));
            let rot = if volume { rotation_ui(n.c("Rotation")) } else { sim::IDENTITY };
            let mut shapes: Vec<ShapeInst> = g.sources(e, "Shapes").into_iter().flat_map(|j| shapes_of(g, j, 0)).collect();
            if shapes.is_empty() {
                shapes.push(ShapeInst::sphere([0.0; 3], 0.22));
            }
            for s in &mut shapes {
                let c = mat_mul(&rot, s.center);
                s.center = [c[0] + pos[0], c[1] + pos[1], c[2] + pos[2]];
                s.rot = rot;
            }
            // forces wired into the emitter: noise breaks up the emission, lines push it
            let mut vel = if volume { ui2sim(n.c("Initial velocity")) } else { [0.0; 3] };
            let mut chaos = None;
            for j in g.sources(e, "Forces") {
                let f = &g.nodes[j];
                if !f.b("Force activity") {
                    continue;
                }
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
            if volume {
                // a moving emitter drags the gas along (Velocity transfer)
                let order = all_emitters.iter().position(|&k| k == e);
                if let (Some(prev), Some(o)) = (prev, order) {
                    if let Some(p0) = prev.get(o) {
                        for k in 0..3 {
                            vel[k] += (pos[k] - p0[k]) * FPS;
                        }
                    }
                }
                vel = vel.map(|v| v * pct(n, "Velocity transfer"));
                if n.b("Emit light") && n.f("Emissive light intensity") > 0.0 {
                    let reach = shapes.iter().map(|s| s.half[0].max(s.half[1]).max(s.half[2])).fold(0.1f32, f32::max) * 4.0;
                    points.push((pos, render::mul(n.c("Emissive light color"), n.f("Emissive light intensity")), reach));
                }
                if n.b("Show emitter") {
                    shown.extend(shapes.iter().map(|s| (s.clone(), n.c("Emitter color"))));
                }
            }
            let particles = (!volume).then(|| Particles {
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
            emitters.push(Emitter {
                shapes,
                particles,
                fuel: n.f("Fuel rate"),
                smoke: n.f("Smoke rate"),
                temp_k: n.f("Temperature"),
                gradient: if volume { n.f("Emission gradient") } else { 15.0 },
                expansion: if volume { pct(n, "Additional pressure rate") } else { 0.0 },
                vel,
                spread: if volume { n.f("Velocity spread") } else { 0.3 },
                fuel_add: !volume || n.ch("Fuel emission") == 0,
                smoke_add: !volume || n.ch("Smoke emission") == 0,
                temp_replace: !volume || n.ch("Temperature emission") == 1,
                burst: if volume && n.f("Time between bursts") > 0.0 { n.f("Duration of burst") } else { 0.0 },
                gap: if volume { n.f("Time between bursts") } else { 0.0 },
                chaos,
                pressure_noise: [pct(n, "Pressure random intensity"), n.f("Pressure random scale"), n.f("Pressure random seed"), n.f("Pressure random speed")],
                shadow_caster: volume && n.b("Shadow caster"),
            });
        }
        for j in g.inputs(&live, si, "Forces") {
            let n = &g.nodes[j];
            if !n.b("Force activity") {
                continue;
            }
            let (kind, radius, dir, extra, scale, speed, strength) = match n.kind {
                Kind::ForceLine => (0, 1.0, ui2sim(n.c("Direction")), 0.0, 0.0, 0.0, n.f("Strength")),
                Kind::ForcePoint => (1, n.f("Radius"), [0.0, 1.0, 0.0], 0.0, 0.0, 0.0, n.f("Strength")),
                Kind::ForceToroidal => (2, n.f("Ring radius"), ui2sim(n.c("Axis")), n.f("Tube radius"), 0.0, 0.0, n.f("Strength")),
                Kind::ForceNoise => (3, 1.0, [0.0, 1.0, 0.0], 0.0, 100.0 / n.f("Scale").max(0.5), n.f("Animation speed"), n.f("Amplitude") / 9.667),
                Kind::ForceVectorField => (4, n.f("Radius"), ui2sim(n.c("Axis")), n.ch("Field type") as f32, 0.0, 0.0, n.f("Strength")),
                Kind::ForceVorticles => (5, n.f("Area"), [0.0, 1.0, 0.0], 0.0, n.f("Vortex size"), n.f("Rise speed"), n.f("Strength")),
                _ => continue,
            };
            let pos = ui2sim(n.c("Position"));
            let mut mask = first_shape(g, j, "Mask Shapes");
            if n.b("Tether Mask Shapes") {
                if let Some(m) = &mut mask {
                    for k in 0..3 {
                        m.center[k] += pos[k];
                    }
                }
            }
            let sr = n.r("Smoke Range");
            let tr = n.r("Temperature Range");
            force_nodes.push(j);
            forces.push(Force {
                kind,
                strength,
                scale,
                speed,
                pos,
                radius,
                dir,
                extra,
                mask,
                softness: n.f("Mask softness"),
                masks: [pct(n, "Constant mask"), pct(n, "Velocity mask"), pct(n, "Temperature mask"), pct(n, "Smoke mask")],
                ranges: [tr[0], tr[1], sr[0] / 100.0, sr[1] / 100.0],
                noise: if kind == 3 { [n.f("Octaves"), n.f("Lacunarity"), n.f("Gain"), n.b("Curl noise") as u8 as f32] } else { [1.0, 2.0, 0.5, 1.0] },
                bias: ui2sim(n.c("Bias")),
                seed: n.f("Seed"),
                mode: n.ch("Mode") as u32,
                rot: rotation_ui(n.c("Rotation")),
            });
        }
        for j in g.inputs(&live, si, "Colliders") {
            if g.nodes[j].kind == Kind::Collider && g.nodes[j].b("Solid") {
                colliders.extend(g.sources(j, "Shapes").into_iter().flat_map(|k| shapes_of(g, k, 0)));
            }
        }
    }
    let s = &simn;
    let adv = s.b("Advanced parameters");
    let advanced_proj = s.ch("Control Mode") == 1;
    let grav = pct(s, "Gravity multiplier");
    let walls = [("Solid Ground", 1), ("Solid Ceiling", 2), ("Solid Wall -X", 4), ("Solid Wall +X", 8), ("Solid Wall -Y", 32), ("Solid Wall +Y", 16)]
        .iter()
        .filter(|(k, _)| s.b(k))
        .map(|(_, b)| *b)
        .sum();
    let wr = s.c("Wind dir randomization");
    let vtr = s.r("Temperature Range");
    let vsr = s.r("Smoke Range");
    let sp = SimParams {
        time_scale: 60.0 / s.f("Timestep").max(1.0),
        substeps: (s.f("Substeps") as usize).max(1),
        freeze: s.b("Freeze Simulation"),
        mode: s.ch("Simulation Mode") as u32,
        vel_cubic: s.ch("Velocity Advection method") == 1,
        vox_cubic: s.ch("Voxel Advection method") == 1,
        order: s.ch("Advection accuracy") as u32,
        adv_offset: ui2sim(s.c("Advection offset")),
        hourglass: pct(s, "Hourglass Filter Strength"),
        reuse_pressure: s.b("Reuse Projection Solution"),
        leak_guard: s.b("Prevent Velocity Leakage"),
        collider_removal: pct(s, "Collider Removal Amplifier"),
        iterations: match s.ch("Projection Quality") {
            0 => 10,
            1 => 20,
            2 => 40,
            3 => 80,
            _ => s.f("Projection iterations") as usize,
        },
        sor_share: if advanced_proj { pct(s, "Over-relaxed share") } else { 0.5 },
        omega: if advanced_proj { s.f("Relaxation factor") } else { 1.8 },
        burn_rate: pct(s, "Burn rate"),
        ignition_k: s.f("Ignition temperature"),
        ignition_soft_k: if adv { s.f("Ignition softness") } else { 400.0 },
        max_burn_k: if adv { s.f("Max burn temperature") } else { 6000.0 },
        heat: pct(s, "Generated heat"),
        smoke_yield: pct(s, "Generated smoke"),
        flames_yield: pct(s, "Generated flames"),
        expansion: pct(s, "Combustion expansion"),
        diss: [
            s.f("Temperature dissipation (*)") * 0.0036,
            s.f("Velocity dissipation (*)") * 0.002,
            s.f("Smoke dissipation (*)") * 0.000526,
            s.f("Flames dissipation (*)") * 0.004,
            s.f("Fuel dissipation (*)") * 0.002,
        ],
        sub: [s.f("Smoke dissipation (-)") / 600.0, s.f("Flames dissipation (-)") / 600.0, s.f("Fuel dissipation (-)") / 600.0],
        diffusion: [
            (s.f("Temperature Diffusion Speed") * 0.1).min(0.9),
            (s.f("Smoke Diffusion Speed") * 0.1).min(0.9),
            (s.f("Fuel Diffusion Speed") * 0.1).min(0.9),
            (s.f("Flames Diffusion Speed") * 0.1).min(0.9),
            (s.f("Velocity Diffusion Speed") * 0.1).min(0.9),
        ],
        vorticity: pct(s, "Vorticity intensity"),
        vorticity_large: pct(s, "Large vorticity intensity"),
        vort_masks: [pct(s, "Velocity mask"), pct(s, "Temperature mask"), pct(s, "Smoke mask"), pct(s, "Constant mask")],
        vort_ranges: [vtr[0], vtr[1], vsr[0] / 100.0, vsr[1] / 100.0],
        vort_ramp: s.f("Vorticity ramp"),
        tightness: pct(s, "Force Tightness") * 0.1,
        buoyancy: s.f("Buoyancy") / 300.0 * grav,
        smoke_weight: pct(s, "Smoke weight") * grav,
        fuel_weight: pct(s, "Fuel weight") * grav,
        walls,
        wind_dir: ui2sim(s.c("Wind direction")),
        wind_rand: [wr[0].abs(), wr[2].abs(), wr[1].abs()],
        wind_speed: s.f("Wind speed"),
        wind_chaos: s.f("Wind chaos"),
        shred: [pct(s, "Shredding intensity"), s.f("Shredding temp threshold"), s.f("Shredding threshold width")],
        mask: [s.ch("Mask shape") as f32, s.f("Mask intensity"), s.f("Mask distance")],
        forces,
        colliders,
        emitters,
    };

    // ---------- shading ----------
    let shading_i = find(Kind::Shading);
    let mut ambient = [0.0; 3];
    let mut sun = None;
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
                Kind::LightPoint => points.insert(0, (ui2sim(n.c("Position")), col, n.f("Range"))),
                _ => {}
            }
        }
    } else {
        ambient = [1.0; 3];
    }
    let shn = shading_i.map(|i| g.nodes[i].clone()).unwrap_or_else(|| template(Kind::Shading));
    let remap = shn.r("Flames remap range");
    // fire colour lookup table: gradient, constant or blackbody node
    let colorn = first_of(shading_i, "Fire color", &[Kind::FireGradient, Kind::ColorConstant, Kind::ColorBlackbody]).cloned().unwrap_or_else(|| template(Kind::FireGradient));
    let lut = match colorn.kind {
        Kind::ColorConstant => vec![colorn.c("Color"); 256],
        Kind::ColorBlackbody => (0..256).map(|i| blackbody(remap[0] + (remap[1] - remap[0]) * i as f32 / 255.0, colorn.f("Saturation"))).collect(),
        _ => widgets::bake_gradient(&colorn.grad("Color gradient"), colorn.ch("Interpolation color space")),
    };
    let scene_i = find(Kind::Scene);
    let proc_ = scene_i.and_then(|s| first_of(Some(s), "Volume", &[Kind::VolumeProcessing])).cloned();
    let has_volume = proc_.as_ref().is_some_and(|p| {
        let pi = g.nodes.iter().position(|n| n.id == p.id).unwrap();
        !g.inputs(&live, pi, "Volume").is_empty()
    });
    let vp = proc_.unwrap_or_else(|| template(Kind::VolumeProcessing));
    let sky = first_of(scene_i, "Skybox", &[Kind::Skybox]).map(|n| (n.c("Top color"), n.c("Bottom color")));
    let ground = first_of(scene_i, "Ground", &[Kind::Ground]).map(|n| (n.c("Color"), n.f("Height")));
    let sparks = scene_i.is_some_and(|s| !g.inputs(&live, s, "Particles").is_empty());
    let rn = g.nodes.iter().find(|n| n.kind == Kind::Render).cloned().unwrap_or_else(|| template(Kind::Render));
    let sharp = |k: &str| (shn.f(k) - 100.0) / 100.0 * 0.25;
    let sl = |k: &str| vp.f(k) / 100.0;
    let modulation = if vp.b("Active") { vp.ch("Modulated channel") as f32 } else { -1.0 };
    let shade = Shade {
        lit,
        ambient,
        sun,
        lut,
        coloring: shn.ch("Flames coloring") as u32,
        fire_color: shn.c("Fire color"),
        remap,
        remap_ramp: shn.f("Flames remap ramp"),
        clamp_remap: shn.b("Clamp the remapping"),
        shaping_flames: pct(&shn, "Shaping by flames"),
        shaping_density: pct(&shn, "Shaping by density"),
        flames_min: pct(&shn, "Flames density min limit"),
        flames_scale: pct(&shn, "Flames density scale"),
        flames_opacity: pct(&shn, "Flames opacity"),
        flames_translucent: shn.b("Flames translucency"),
        render_flames: shn.b("Render flames"),
        render_smoke: shn.b("Render smoke"),
        render_fuel: shn.b("Render fuel"),
        smoke_color: shn.c("Smoke color"),
        smoke_scale: pct(&shn, "Smoke density scale"),
        smoke_min: pct(&shn, "Smoke density min limit"),
        smoke_clamp: pct(&shn, "Smoke density clamp"),
        smoke_coloring: shn.ch("Smoke coloring") as u32,
        smoke_remap: shn.r("Smoke remap range"),
        smoke_remap_ramp: shn.f("Smoke remap ramp"),
        smoke_by_temp: shn.b("Remap by temperature"),
        fuel_color: shn.c("Fuel color"),
        fuel_scale: pct(&shn, "Fuel density scale"),
        shadow_density: pct(&shn, "Smoke shadow density"),
        // 50% reproduces plain transmittance; more darkens shadows, less lightens them
        shadow_intensity: shn.f("Smoke shadow intensity") / 50.0,
        shadow_sharpness: pct(&shn, "Shadowing sharpness"),
        shadow_steps: if shn.b("Advanced parameters") { shn.f("Shadow steps") } else { 64.0 },
        shadow_reach: if shn.b("Advanced parameters") { pct(&shn, "Shadow max distance") } else { 1.0 },
        anisotropy: pct(&shn, "Lighting anisotropy"),
        ao: pct(&shn, "Ambient occlusion"),
        extinction: shn.c("Extinction color"),
        direct: pct(&shn, "Direct light contribution") * 4.0,
        flames_light: pct(&shn, "Flames contribution"),
        particles_light: pct(&shn, "Particles contribution"),
        scatter_radius: shn.f("Scattering radius"),
        scatter_occlusion: shn.f("Scattering occlusion"),
        emask: [
            shn.b("Masking Active") as u8 as f32,
            shn.f("Density reference"),
            shn.f("Width"),
            shn.f("Masking ramp"),
            pct(&shn, "Masking intensity"),
            pct(&shn, "Low frequencies"),
            pct(&shn, "Medium frequencies"),
            pct(&shn, "High frequencies"),
        ],
        sharpen: shn.b("Raymarch sharpening").then(|| [sharp("Smoke sharpening"), sharp("Flames sharpening"), sharp("Temperature sharpening")]),
        post: [vp.ch("Volume post process") as f32, sl("Post process strength"), modulation, 100.0 / vp.f("Noise scale").max(0.5), sl("Modulation intensity"), vp.f("Modulation speed")],
        // sim axes: x-, x+, z- (= UI Y+), z+ (= UI Y-), y- (ground), y+ (roof)
        slicing: [
            sl("X- wall offset"), sl("X- wall width"), sl("X+ wall offset"), sl("X+ wall width"),
            sl("Y+ wall offset"), sl("Y+ wall width"), sl("Y- wall offset"), sl("Y- wall width"),
            sl("Z- ground offset"), sl("Z- ground width"), sl("Z+ roof offset"), sl("Z+ roof width"),
            vp.f("Mask ramp"),
        ],
        points,
        shapes: shown,
        shape_mode: [rn.ch("Shapes Rendering") as u32, rn.ch("Shapes Masking") as u32, rn.b("Invert Shapes Alpha") as u32],
        sparks,
        sparks_hq: s.b("Particles HQ rendering"),
        spark_group: [1, 2, 4, 8][s.ch("Grouping size").min(3)],
        render_volume: rn.b("Render Volume") && has_volume,
        render_particles: rn.b("Render Particles"),
        volume_occludes_particles: rn.b("Volume Occludes Particles"),
        particles_occlude_volume: rn.b("Particles Occlude Volume"),
        detail: [0.0, 0.8, 1.6][s.ch("Upscaling").min(2)],
        fast_detail: s.b("Fast upscaler"),
        sky,
        ground,
        step,
        pass: 0,
    };
    let scn = scene_i.map(|i| g.nodes[i].clone()).unwrap_or_else(|| template(Kind::Scene));
    let post = Post {
        gamma: scn.f("Gamma"),
        exposure: scn.f("Exposure"),
        vibrance: scn.f("Color vibrance"),
        tonemap: scn.ch("HDR Tonemapping") as u32,
        dither: pct(&scn, "Tonemap dithering"),
        contrast: scn.f("Contrast"),
        brightness: scn.f("Brightness"),
        saturation: scn.f("Saturation"),
        swizzle: scn.ch("Channels swizzling") as u32,
        bloom: scn.f("Bloom Intensity"),
        bloom_radius: scn.f("Bloom Radius"),
        vignette: [scn.f("Width"), scn.f("Offset"), pct(&scn, "Roundness"), pct(&scn, "Intensity")],
        style: scn.ch("Rendering style") as u32,
        style_size: scn.f("Style size"),
        quant: [scn.f("RGB quantization"), scn.f("Hue quantization"), scn.f("Value quantization"), scn.f("Saturation quantization")],
        quant_dither: pct(&scn, "Quantization dither"),
        dither_mode: scn.ch("Dithering mode") as u32,
        alpha_limit: pct(&scn, "Translucency limit"),
        alpha_sharpen: pct(&scn, "Translucency sharpening"),
        remap_style: scn.ch("Remapping style") as u32,
        alpha_mode: 2,
        invert_alpha: false,
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
        Camera { yaw: d[0].atan2(d[2]).to_degrees(), pitch: (d[1] / dist).clamp(-1.0, 1.0).asin().to_degrees(), dist, fov: camn.f("Field of view"), target: at, ortho }
    } else {
        Camera { yaw: camn.f("Yaw"), pitch: camn.f("Pitch"), dist: camn.f("Distance"), fov: camn.f("Field of view"), target: [0.0, camn.f("Target height"), 0.0], ortho }
    };
    Built {
        sp,
        dom,
        shade,
        post,
        cam,
        duration: rn.f("Duration").max(1.0) as u32,
        looping: s.ch("Looping Mode") == 1,
        sim_gizmo: s.b("Enable gizmo"),
        force_nodes,
        render: rn,
        imports,
        reduced,
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

/// What and how to export (from the Render node, or a simple Export node).
#[derive(Clone)]
struct ExportCfg {
    flipbook: bool,
    /// Size of one image (sequence) or one tile (flipbook)
    size: [usize; 2],
    cols: usize,
    rows: usize,
    first: u32,
    count: u32,
    stride: u32,
    offset: i64,
    absolute: bool,
    dir: PathBuf,
    pattern: String,
    project: String,
    /// 0 png, 1 exr, 2 tga
    format: usize,
    passes: Vec<RPass>,
    alpha_mode: u32,
    invert_alpha: bool,
}

impl ExportCfg {
    fn from_render(rn: &Node, project: &str) -> Self {
        let flipbook = rn.ch("Export Mode") == 0;
        let (cols, rows) = (rn.f("Flipbook columns").max(1.0) as usize, rn.f("Flipbook rows").max(1.0) as usize);
        let size = if flipbook {
            [(rn.f("Flipbook width") as usize / cols).max(1), (rn.f("Flipbook height") as usize / rows).max(1)]
        } else {
            [rn.f("Image width") as usize, rn.f("Image height") as usize]
        };
        let mut passes = rn.passes("Mapping");
        if passes.is_empty() {
            passes.push(RPass { src: 0, hdr: false, dst: 0 });
        }
        ExportCfg {
            flipbook,
            size,
            cols,
            rows,
            first: rn.f("First Frame").max(0.0) as u32,
            count: rn.f("Number of Frames").max(1.0) as u32,
            stride: rn.f("Frame Stride").max(1.0) as u32,
            offset: rn.f("Numbering offset") as i64,
            absolute: rn.b("Absolute Frames"),
            dir: widgets::export_dir(rn),
            pattern: rn.s("Filename"),
            project: project.to_string(),
            format: rn.ch("File format"),
            passes,
            alpha_mode: rn.ch("Alpha Blending Mode") as u32,
            invert_alpha: rn.b("Invert Render Alpha"),
        }
    }

    fn file(&self, pass: &RPass, frame: Option<i64>) -> PathBuf {
        let mut name = widgets::expand_filename(&self.pattern, &self.project, &widgets::safename(pass.src));
        if let Some(f) = frame {
            name = if name.contains("$(frame)") { name.replace("$(frame)", &format!("{f:04}")) } else { format!("{name}_{f:04}") };
        }
        self.dir.join(name + [".png", ".exr", ".tga"][self.format.min(2)])
    }
}

/// Save one image: `rgba8` (display) or `hdr` (linear floats), stored as RGBA, RGB or one channel.
fn save_image(path: &std::path::Path, format: usize, dst: usize, w: usize, h: usize, rgba8: &[u8], hdr: Option<&[f32]>) -> Result<(), String> {
    let err = |e: image::ImageError| format!("{}: {e}", path.display());
    if format == 1 {
        {
            // EXR: floats (from the HDR buffer, or the 8-bit picture if HDR is off)
            let fl: Vec<f32> = match hdr {
                Some(f) => f.to_vec(),
                None => rgba8.iter().map(|&b| b as f32 / 255.0).collect(),
            };
            match dst {
                2 => image::ImageBuffer::<image::Rgb<f32>, _>::from_raw(w as u32, h as u32, fl.chunks(4).flat_map(|c| [c[0], c[0], c[0]]).collect::<Vec<f32>>()).map(image::DynamicImage::ImageRgb32F),
                1 => image::ImageBuffer::<image::Rgb<f32>, _>::from_raw(w as u32, h as u32, fl.chunks(4).flat_map(|c| [c[0], c[1], c[2]]).collect::<Vec<f32>>()).map(image::DynamicImage::ImageRgb32F),
                _ => image::ImageBuffer::<image::Rgba<f32>, _>::from_raw(w as u32, h as u32, fl).map(image::DynamicImage::ImageRgba32F),
            }
            .ok_or("image size mismatch")?
            .save(path)
            .map_err(err)
        }
    } else {
        {
            let fmt = if format == 2 { image::ImageFormat::Tga } else { image::ImageFormat::Png };
            match dst {
                2 => image::save_buffer_with_format(path, &rgba8.chunks(4).map(|c| c[0]).collect::<Vec<u8>>(), w as u32, h as u32, image::ColorType::L8, fmt),
                1 => image::save_buffer_with_format(path, &rgba8.chunks(4).flat_map(|c| [c[0], c[1], c[2]]).collect::<Vec<u8>>(), w as u32, h as u32, image::ColorType::Rgb8, fmt),
                _ => image::save_buffer_with_format(path, rgba8, w as u32, h as u32, image::ColorType::Rgba8, fmt),
            }
            .map_err(err)
        }
    }
}

/// Image sequence / flipbook export, advanced a few frames per UI frame on the GPU.
struct ExportJob {
    cfg: ExportCfg,
    /// Graph and view settings, re-read each frame (modulators animate during export).
    src: (Graph, bool, bool, f32),
    sim: GpuSim,
    target: Target,
    /// One sheet per pass (flipbook): 8-bit and float pixels
    sheets: Vec<(Vec<u8>, Vec<f32>)>,
    saved: u32,
    prev: Option<Vec<V3>>,
}

impl ExportJob {
    fn new(gpu: &Gpu, b: &Built, src: (Graph, bool, bool, f32), cfg: ExportCfg) -> Result<Self, String> {
        std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("{}: {e}", cfg.dir.display()))?;
        let [w, h] = cfg.size;
        let sheet_px = w * cfg.cols * h * cfg.rows;
        let sheets = if cfg.flipbook {
            cfg.passes.iter().map(|p| (vec![0u8; sheet_px * 4], if p.hdr || cfg.format == 1 { vec![0.0f32; sheet_px * 4] } else { vec![] })).collect()
        } else {
            vec![]
        };
        Ok(ExportJob { sim: GpuSim::new(gpu, b.dom), target: Target::new(gpu, w.max(8), h.max(8)), sheets, saved: 0, prev: None, src, cfg })
    }

    fn last_frame(&self) -> u32 {
        let count = if self.cfg.flipbook { self.cfg.count.min((self.cfg.cols * self.cfg.rows) as u32) } else { self.cfg.count };
        self.cfg.first + (count.max(1) - 1) * self.cfg.stride
    }

    fn progress(&self) -> f32 {
        self.sim.frame as f32 / (self.last_frame() + 1) as f32
    }

    /// Ok(Some(message)) when finished.
    fn advance(&mut self, gpu: &Gpu, steps: u32) -> Result<Option<String>, String> {
        let [w, h] = self.cfg.size;
        let last = self.last_frame();
        for _ in 0..steps {
            if self.sim.frame > last {
                break;
            }
            let f = self.sim.frame;
            let (g, lit, ortho, step) = &self.src;
            let mg = modulate::apply(g, f as f32 / FPS);
            let now = build_moving(&mg, self.prev.as_deref(), *lit, *ortho, *step);
            self.prev = Some(emitter_positions(&mg));
            self.sim.step(gpu, &now.sp);
            if f < self.cfg.first || (f - self.cfg.first) % self.cfg.stride != 0 {
                continue;
            }
            let index = (f - self.cfg.first) / self.cfg.stride;
            let mut post = now.post.clone();
            post.alpha_mode = self.cfg.alpha_mode;
            post.invert_alpha = self.cfg.invert_alpha;
            for (pi, pass) in self.cfg.passes.clone().iter().enumerate() {
                let mut shade = now.shade.clone();
                shade.pass = pass.src as u32;
                // a few refinement passes for clean frames
                for k in 0..4u32 {
                    self.sim.render(gpu, &self.target, &now.cam, &shade, &post, f * 7 + k, 1.0 / (k + 1) as f32);
                }
                let px = gpu.read_target(&self.target);
                let want_float = pass.hdr || self.cfg.format == 1;
                let fl = want_float.then(|| gpu.read_hdr(&self.target));
                if self.cfg.flipbook {
                    let cols = self.cfg.cols;
                    let (tx, ty) = (index as usize % cols, index as usize / cols);
                    let (sheet, fsheet) = &mut self.sheets[pi];
                    for row in 0..h {
                        let dst = ((ty * h + row) * w * cols + tx * w) * 4;
                        sheet[dst..dst + w * 4].copy_from_slice(&px[row * w * 4..(row + 1) * w * 4]);
                        if let Some(fl) = &fl {
                            if !fsheet.is_empty() {
                                fsheet[dst..dst + w * 4].copy_from_slice(&fl[row * w * 4..(row + 1) * w * 4]);
                            }
                        }
                    }
                } else {
                    let number = if self.cfg.absolute { f as i64 } else { index as i64 } + self.cfg.offset;
                    let hdr = if pass.hdr { fl.as_deref() } else { None };
                    save_image(&self.cfg.file(pass, Some(number)), self.cfg.format, pass.dst, w, h, &px, hdr)?;
                }
            }
            self.saved += 1;
        }
        if self.sim.frame <= last {
            return Ok(None);
        }
        if self.cfg.flipbook {
            let (sw, sh) = (w * self.cfg.cols, h * self.cfg.rows);
            for (pi, pass) in self.cfg.passes.iter().enumerate() {
                let (sheet, fsheet) = &self.sheets[pi];
                let hdr = (pass.hdr && !fsheet.is_empty()).then_some(fsheet.as_slice());
                save_image(&self.cfg.file(pass, None), self.cfg.format, pass.dst, sw, sh, sheet, hdr)?;
            }
        }
        Ok(Some(format!("Saved {} frames × {} passes to {}", self.saved, self.cfg.passes.len(), self.cfg.dir.display())))
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
    /// Dragging a face of the simulation box: (face 0..6, min corner, size), sim axes.
    bounds_drag: Option<(usize, V3, V3)>,
    playing: bool,
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
            sim: GpuSim::new(&gpu, domain_of(&template(Kind::Simulation)).0),
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
            bounds_drag: None,
            playing: true,
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
        // thumbnails use a coarser grid of the same size
        let simi = g.first(Kind::Simulation).unwrap();
        let (vc, vs) = (g.nodes[simi].c("Voxel count"), g.nodes[simi].f("Voxel size"));
        g.nodes[simi].set("Voxel count", Val::V(vc.map(|x| (x * 0.6).round())));
        g.nodes[simi].set("Voxel size", Val::F(vs / 0.6));
        let b = build(&g, true, false, 0.7);
        let mut s = GpuSim::new(&self.gpu, b.dom);
        let frames = presets::PRESETS[i].frame.min(90);
        for _ in 0..frames {
            s.step(&self.gpu, &b.sp);
        }
        let t = Target::new(&self.gpu, 400, 248);
        s.render(&self.gpu, &t, &b.cam, &b.shade, &b.post, 0, 1.0);
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
        self.sim = GpuSim::new(&self.gpu, b.dom);
        self.load_imports(&b);
    }

    fn project_name(&self) -> String {
        self.project.as_ref().and_then(|p| p.file_stem()).map_or("Untitled".to_string(), |s| s.to_string_lossy().to_string())
    }

    /// Select the Simulation node on its Domain tab: the bounds box and its handles appear.
    fn edit_bounds(&mut self) {
        if let Some(i) = self.graph.first(Kind::Simulation) {
            self.graph.selected = Some(i);
            self.graph.details_tab = 0;
            if let Some(p) = self.graph.nodes[i].params.iter().find(|p| p.section == "Domain") {
                self.graph.active_section = p.section;
            }
            self.view_tab = 0;
        }
    }

    /// Simulation › Time Control › Looping Mode
    fn looping(&self) -> bool {
        self.graph.first(Kind::Simulation).is_some_and(|i| self.graph.nodes[i].ch("Looping Mode") == 1)
    }
    fn set_looping(&mut self, on: bool) {
        if let Some(n) = self.graph.node_mut(Kind::Simulation) {
            n.set("Looping Mode", Val::Ch(on as usize));
        }
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

    /// Run an Export node's or the Render node's "Export Now".
    fn export_node(&mut self, i: usize) {
        let n = self.graph.nodes[i].clone();
        let b = self.built();
        let project = self.project_name();
        let mut cfg = ExportCfg::from_render(&b.render, &project);
        match n.kind {
            Kind::Render => {}
            Kind::ExportImage => {
                cfg.flipbook = false;
                cfg.size = [n.f("Width") as usize, n.f("Height") as usize];
                cfg.first = 0;
                cfg.count = n.f("Frames").max(1.0) as u32;
                cfg.stride = n.f("Frame step").max(1.0) as u32;
            }
            Kind::ExportFlipbook => {
                cfg.flipbook = true;
                cfg.size = [n.f("Tile width") as usize, n.f("Tile height") as usize];
                cfg.cols = n.f("Columns").max(1.0) as usize;
                cfg.count = n.f("Frames").max(1.0) as u32;
                cfg.rows = (cfg.count as usize).div_ceil(cfg.cols);
                cfg.first = 0;
                cfg.stride = n.f("Frame step").max(1.0) as u32;
            }
            _ => {
                self.status = match export_volume(&self.gpu, &self.sim, &self.export_dir) {
                    Ok(m) => m,
                    Err(e) => format!("Export failed: {e}"),
                };
                return;
            }
        }
        self.start_export_cfg(cfg, b);
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
        let mut cfg = ExportCfg::from_render(&b.render, &self.project_name());
        match job {
            Job::Volume => {
                self.status = match export_volume(&self.gpu, &self.sim, &self.export_dir) {
                    Ok(m) => m,
                    Err(e) => format!("Export failed: {e}"),
                };
                return;
            }
            Job::Flipbook => cfg.flipbook = true,
            Job::Sequence => {
                cfg.flipbook = false;
                cfg.size = [b.render.f("Image width") as usize, b.render.f("Image height") as usize];
            }
        }
        self.start_export_cfg(cfg, b);
    }

    fn start_export_cfg(&mut self, cfg: ExportCfg, b: Built) {
        if self.export.is_some() {
            return;
        }
        let src = (self.graph.clone(), self.lit, self.ortho, self.step());
        self.status = match ExportJob::new(&self.gpu, &b, src, cfg) {
            Ok(j) => {
                self.export = Some(j);
                "Exporting…".into()
            }
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
                    let mut lp = self.looping();
                    if ui.checkbox(&mut lp, "Loop at render duration").changed() {
                        self.set_looping(lp);
                    }
                    ui.separator();
                    if ui.button("Edit simulation bounds…").on_hover_text("Shows the box in the viewport: drag its dots to resize it").clicked() {
                        self.edit_bounds();
                        ui.close_menu();
                    }
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
            self.sim.render(&self.gpu, target, &b.cam, &b.shade, &b.post, seed, weight);
        }
        ui.painter().image(*tex_id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        let resp = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
        let aspect = rect.width() / rect.height();
        let proj = |p: V3| b.cam.project(p, aspect).map(|s| pos2(rect.min.x + s[0] * rect.width(), rect.min.y + s[1] * rect.height()));
        let painter = ui.painter_at(rect);
        let em = self.emitter_index();
        // simulation bounds: shown with the Simulation node selected, editable by dragging a face handle
        let sim_sel = self.graph.selected.is_some_and(|i| self.graph.nodes.get(i).is_some_and(|n| n.kind == Kind::Simulation));
        let (box_o, box_s) = match self.bounds_drag {
            Some((_, o, s)) => (o, s),
            None => (b.dom.origin, b.dom.n.map(|c| c as f32 * b.dom.h)),
        };
        let face = |k: usize| {
            let (a, side) = (k / 2, (k % 2) as f32);
            let mut c = [box_o[0] + box_s[0] * 0.5, box_o[1] + box_s[1] * 0.5, box_o[2] + box_s[2] * 0.5];
            c[a] = box_o[a] + box_s[a] * side;
            c
        };
        let handles: Vec<(usize, Pos2)> = if sim_sel && self.view_tab == 0 { (0..6).filter_map(|k| proj(face(k)).map(|p| (k, p))).collect() } else { vec![] };
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hot = pointer.and_then(|p| handles.iter().filter(|(_, h)| h.distance(p) < 10.0).min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p))).map(|h| h.0));
        if hot.is_some() || self.bounds_drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        if self.view_tab == 0 {
            // arrows of the selected force (Visual › Enable gizmo) and its mask outline
            if let Some(fi) = self.graph.selected.and_then(|s| b.force_nodes.iter().position(|&j| j == s)) {
                let n = &self.graph.nodes[b.force_nodes[fi]];
                let f = &b.sp.forces[fi];
                let (o, s) = (self.sim.origin(), self.sim.size());
                if n.b("Enable gizmo") {
                    let t = self.sim.frame as f32 / FPS;
                    let k = (3.0 + n.f("Density").clamp(0.1, 1.0) * 9.0).round() as usize;
                    let slice = n.ch("Visual mode") == 1;
                    let mut pts = vec![];
                    for i in 0..k {
                        for jy in 0..k {
                            for jz in 0..if slice { 1 } else { k } {
                                let u = |a: usize| (a as f32 + 0.5) / k as f32;
                                let z = if slice { ((f.pos[2] - o[2]) / s[2]).clamp(0.0, 1.0) } else { u(jz) };
                                pts.push([o[0] + s[0] * u(i), o[1] + s[1] * u(jy), o[2] + s[2] * z]);
                            }
                        }
                    }
                    let vecs: Vec<V3> = pts.iter().map(|&p| force_vec(f, p, t)).collect();
                    let max = vecs.iter().map(|v| render::dot(*v, *v).sqrt()).fold(1e-6, f32::max);
                    let arm = s[0].min(s[1]) / k as f32 * 0.8 / max;
                    let arrow = n.ch("Arrow type") == 0;
                    let st = Stroke::new(1.2, Color32::from_rgb(90, 200, 255).gamma_multiply(0.85));
                    for (p, v) in pts.iter().zip(&vecs) {
                        if let (Some(a), Some(c)) = (proj(*p), proj(render::add(*p, v.map(|x| x * arm)))) {
                            painter.line_segment([a, c], st);
                            let d = c - a;
                            if arrow && d.length() > 4.0 {
                                let (dn, w) = (d.normalized(), d.normalized().rot90());
                                painter.line_segment([c, c - dn * 4.0 + w * 2.5], st);
                                painter.line_segment([c, c - dn * 4.0 - w * 2.5], st);
                            }
                        }
                    }
                }
                if let Some(mk) = f.mask.as_ref().filter(|_| n.b("Show Force Mask")) {
                    for axis in 0..3 {
                        let pts: Vec<Pos2> = (0..=32)
                            .filter_map(|q| {
                                let a = q as f32 / 32.0 * std::f32::consts::TAU;
                                let (c, sn, h) = (a.cos(), a.sin(), mk.half);
                                let d = match axis { 0 => [c * h[0], sn * h[1], 0.0], 1 => [c * h[0], 0.0, sn * h[2]], _ => [0.0, c * h[1], sn * h[2]] };
                                proj(render::add(mk.center, d))
                            })
                            .collect();
                        painter.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_rgb(255, 200, 80))));
                    }
                }
            }
            if self.show_bounds || b.sim_gizmo || sim_sel {
                let (o, s) = (box_o, box_s);
                let c = |i: usize| [o[0] + s[0] * (i & 1) as f32, o[1] + s[1] * ((i >> 1) & 1) as f32, o[2] + s[2] * ((i >> 2) & 1) as f32];
                let line = Stroke::new(1.0, if sim_sel { Color32::from_rgb(200, 160, 60) } else { Color32::from_gray(70) });
                for (a, bb) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    if let (Some(p), Some(q)) = (proj(c(a)), proj(c(bb))) {
                        painter.line_segment([p, q], line);
                    }
                }
                // face handles, coloured by axis (sim x = X red, sim y = Z up blue, sim z = Y green)
                let dragged = self.bounds_drag.map(|d| d.0);
                for &(k, p) in &handles {
                    let col = [Color32::from_rgb(235, 60, 70), Color32::from_rgb(60, 130, 255), Color32::from_rgb(60, 200, 80)][k / 2];
                    let big = hot == Some(k) || dragged == Some(k);
                    painter.circle(p, if big { 7.0 } else { 5.0 }, col, Stroke::new(1.5, Color32::WHITE));
                }
                if sim_sel {
                    let ui_size = [s[0], s[2], s[1]];
                    let txt = format!("Bounds {:.2} × {:.2} × {:.2} m", ui_size[0], ui_size[1], ui_size[2]);
                    let hint = if self.bounds_drag.is_some() { txt } else { format!("{txt} · drag a dot to resize") };
                    painter.text(rect.left_bottom() + vec2(90.0, -12.0), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::proportional(12.0), TEXT);
                }
            }
            // emitter shape + move gizmo
            if let Some(ei) = em {
                let n = &self.graph.nodes[ei];
                let pos = ui2sim(n.c("Position"));
                if self.render_shapes && n.b("Show emitter") {
                    let size = b.sp.emitters.first().and_then(|e| e.shapes.first()).map_or(0.22, |s| s.half[0]);
                    let sc = n.c("Emitter color");
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
        // dragging a bounds handle moves that face along its axis; applied when released
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(k) = hot {
                self.bounds_drag = Some((k, box_o, box_s));
            }
        }
        if let Some((k, mut o, mut s)) = self.bounds_drag {
            let a = k / 2;
            let c = face(k);
            let mut tip = c;
            tip[a] += 0.1;
            if let (Some(p0), Some(p1)) = (proj(c), proj(tip)) {
                let dir = p1 - p0; // screen pixels per 10 cm along the axis
                let along = delta.dot(dir) / dir.length_sq().max(1e-3) * 0.1;
                let min = 8.0 * b.dom.h;
                if k % 2 == 1 {
                    s[a] = (s[a] + along).max(min);
                } else {
                    let d = along.min(s[a] - min);
                    o[a] += d;
                    s[a] -= d;
                }
                self.bounds_drag = Some((k, o, s));
            }
            if !resp.dragged() {
                if let Some(si) = self.graph.first(Kind::Simulation) {
                    set_domain_box(&mut self.graph.nodes[si], o, s);
                }
                self.bounds_drag = None;
            }
        }
        let orbit = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (resp.dragged_by(egui::PointerButton::Primary) && (self.tool == 1 || self.view_tab == 1) && self.bounds_drag.is_none());
        if self.bounds_drag.is_some() {
            // the handle drag owns the mouse
        } else if orbit && !self.lock_camera {
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
            if ui.button("Edit simulation bounds").clicked() {
                self.edit_bounds();
                ui.close_menu();
            }
            ui.checkbox(&mut self.lock_camera, "Lock camera");
            ui.separator();
            ui.checkbox(&mut self.hq, "HQ render (Ctrl+3)");
            ui.checkbox(&mut self.lit, "Lit");
            ui.checkbox(&mut self.ortho, "Orthographic camera");
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
                if let Some(sn) = self.graph.node_mut(Kind::Simulation) {
                    ui.label(small("Projection Quality"));
                    let mut pi = sn.ch("Projection Quality");
                    stepper(ui, &mut pi, &["Low", "Medium", "High", "Ultra", "Custom"]);
                    if pi != sn.ch("Projection Quality") {
                        sn.set("Projection Quality", Val::Ch(pi));
                    }
                    ui.end_row();
                }
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
            if ui.add(icon("↻", self.looping())).on_hover_text("Loop at render duration").clicked() {
                let lp = !self.looping();
                self.set_looping(lp);
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
            let mut lp = self.looping();
            if ui.checkbox(&mut lp, "Loop at render duration").changed() {
                self.set_looping(lp);
            }
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
            // FC_SELECT="Node title" and FC_SECTION="Tab name" pick which settings the shot shows
            if self.frames == 1 {
                if let Ok(t) = std::env::var("FC_SELECT") {
                    self.graph.selected = self.graph.nodes.iter().position(|n| n.title == t || (n.title.is_empty() && n.kind.name() == t));
                }
                if let (Ok(s), Some(i)) = (std::env::var("FC_SECTION"), self.graph.selected) {
                    if let Some(p) = self.graph.nodes[i].params.iter().find(|p| p.section == s) {
                        self.graph.active_section = p.section;
                    }
                }
            }
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
        // a new grid size restarts the simulation, but only once the mouse is released (no restart per pixel of a drag)
        let holding = ctx.input(|i| i.pointer.any_down());
        let resized = b.dom.n != self.sim.dom.n || (b.dom.h - self.sim.dom.h).abs() > 1e-7;
        if (resized && !holding) || std::mem::take(&mut self.graph.import_request) {
            self.sim = GpuSim::new(&self.gpu, b.dom);
            self.load_imports(&b);
        } else if !resized && b.dom.origin != self.sim.dom.origin {
            // moving the bounds doesn't restart the simulation
            self.sim.dom.origin = b.dom.origin;
        }
        if b.reduced && self.status.is_empty() {
            self.status = "Grid reduced to fit the graphics card (max 48 million voxels)".into();
        }
        if self.seek_to.is_some() {
            self.advance_seek();
        } else if self.playing {
            self.sim.step(&self.gpu, &b.sp);
            if b.looping && self.sim.frame >= b.duration {
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
        // data for the Shading node's histograms and the Render node's filename preview
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(widgets::PROJECT_ID), self.project_name()));
        let shading_open = self.graph.selected.is_some_and(|i| self.graph.nodes.get(i).is_some_and(|n| n.kind == Kind::Shading));
        if shading_open && ctx.cumulative_pass_nr() % 30 == 0 {
            let (temp, smoke) = self.sim.histograms(&self.gpu);
            ctx.data_mut(|d| d.insert_temp(egui::Id::new(widgets::HIST_ID), widgets::Histograms { temp, smoke, temp_max: 10000.0, smoke_max: 2.0 }));
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
        let mut s = GpuSim::new(&gpu, b.dom);
        for _ in 0..120 {
            s.step(&gpu, &b.sp);
        }
        let t = Target::new(&gpu, 360, 300);
        s.render(&gpu, &t, &b.cam, &b.shade, &b.post, 0, 1.0);
        let px = gpu.read_target(&t);
        let _ = std::fs::create_dir_all("shots");
        save_png(std::path::Path::new("shots/test.png"), &px, t.w, t.h).unwrap();
        let bright = px.chunks(4).filter(|p| p[0] > 200 && p[1] > 90).count();
        assert!(bright > 200, "expected visible flames, got {bright} bright pixels");

        let base = std::env::temp_dir().join("firecraft_test");
        for (flip, format) in [(true, 0), (false, 1), (false, 2)] {
            let mut cfg = ExportCfg::from_render(&b.render, "test");
            cfg.flipbook = flip;
            cfg.format = format;
            cfg.size = [32, 32];
            cfg.cols = 2;
            cfg.rows = 2;
            cfg.count = 4;
            cfg.dir = base.clone();
            let mut job = ExportJob::new(&gpu, &b, (g.clone(), true, false, 0.7), cfg).unwrap();
            while job.advance(&gpu, 4).unwrap().is_none() {}
        }
        let files = std::fs::read_dir(&base).unwrap().count();
        assert!(files >= 2 + 2 * 4, "expected exported files, got {files}");
        assert!(export_volume(&gpu, &s, &base.display().to_string()).unwrap().contains("volume"));
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
        // FC_PRESET="smoke column" keeps that preset's own camera
        match std::env::var("FC_PRESET") {
            Ok(name) => {
                let i = presets::PRESETS.iter().position(|p| p.name.to_lowercase().contains(&name.to_lowercase())).expect("no such preset");
                presets::apply(&mut g, i);
            }
            Err(_) => {
                presets::apply(&mut g, 0);
                let cam = g.node_mut(Kind::Camera).unwrap();
                cam.set("Distance", Val::F(1.9));
                cam.set("Target height", Val::F(0.3));
            }
        }
        // experiments: FC_SET="Node title|Setting=1.5;Other node|Vec=1,2,3"
        for item in std::env::var("FC_SET").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
            let (node, rest) = item.split_once('|').unwrap();
            let (name, val) = rest.split_once('=').unwrap();
            let i = g.nodes.iter().position(|n| n.title == node || (n.title.is_empty() && n.kind.name() == node)).unwrap_or_else(|| panic!("no node {node}: {:?}", g.nodes.iter().map(|n| n.title.clone()).collect::<Vec<_>>()));
            let nums: Vec<f32> = val.split(',').map(|x| x.trim().parse().unwrap()).collect();
            let v = match (nums.len(), &g.nodes[i].params.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no {name}")).val) {
                (3, _) => Val::V([nums[0], nums[1], nums[2]]),
                (_, Val::Ch(_)) => Val::Ch(nums[0] as usize),
                (_, Val::B(_)) => Val::B(nums[0] != 0.0),
                _ => Val::F(nums[0]),
            };
            g.nodes[i].set(name, v);
        }
        let step: f32 = std::env::var("FC_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.4);
        let b = build(&g, true, false, step);
        let mut s = GpuSim::new(&gpu, b.dom);
        let t = Target::new(&gpu, 720, 600);
        let frames: u32 = std::env::var("FC_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
        for f in 1..=frames {
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
            if f == 90 || f == frames {
                s.render(&gpu, &t, &b.cam, &b.shade, &b.post, f, 1.0);
                let px = gpu.read_target(&t);
                save_png(std::path::Path::new(&format!("shots/close_{}.png", if f == frames { "last".to_string() } else { f.to_string() })), &px, t.w, t.h).unwrap();
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
            let mut s = GpuSim::new(&gpu, b0.dom);
            for f in 0..pr.frame {
                let b = build(&modulate::apply(&g, f as f32 / FPS), true, false, 0.25);
                s.step(&gpu, &b.sp);
            }
            let b = build(&modulate::apply(&g, pr.frame as f32 / FPS), true, false, 0.25);
            let t = Target::new(&gpu, w, h);
            for pass in 0..12u32 {
                s.render(&gpu, &t, &b.cam, &b.shade, &b.post, pass, 1.0 / (pass + 1) as f32);
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
        let picks = [("Campfire", 30u32), ("Mushroom Cloud", 60), ("Nebula", 90), ("Toxic Cloud", 60), ("Gas Burner", 30), ("Ground Explosion", 0)];
        for (k, (name, start)) in picks.iter().enumerate() {
            let i = presets::PRESETS.iter().position(|p| p.name == *name).unwrap();
            let mut g = Graph::default();
            presets::apply(&mut g, i);
            std::fs::write(format!("shots/readme_{k}.fcproj"), g.save()).unwrap();
            g.node_mut(Kind::Simulation).unwrap().set("Resolution", Val::Ch(2));
            let b0 = build(&g, true, false, 0.3);
            let mut s = GpuSim::new(&gpu, b0.dom);
            let t = Target::new(&gpu, 360, 240);
            let total = start + 96;
            for f in 0..total {
                let b = build(&modulate::apply(&g, f as f32 / FPS), true, false, 0.3);
                s.step(&gpu, &b.sp);
                if f >= *start && (f - start) % 2 == 0 {
                    for pass in 0..4u32 {
                        s.render(&gpu, &t, &b.cam, &b.shade, &b.post, f * 4 + pass, 1.0 / (pass + 1) as f32);
                    }
                    let px = gpu.read_target(&t);
                    save_png(std::path::Path::new(&format!("shots/anim/{k}_{:03}.png", (f - start) / 2)), &px, t.w, t.h).unwrap();
                }
            }
        }
    }
}
