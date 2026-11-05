// The node settings → inputs of the 0.4.0 engine (gpu.rs, sim.wgsl, render.wgsl).
//
// This is 0.4.0's own `build` code, reading today's setting names. Every setting that
// existed in 0.4 is converted back with the exact inverse of `graph::legacy_set`, so a 0.4
// scene gives the engine the very same numbers it got in 0.4 (and the same picture).
// Settings the 0.4 engine has no counterpart for are not passed on.
use crate::graph::{Graph, Kind, NOISE_EMIT, NOISE_FIRE, Node, template};
use crate::sim::{Domain, FPS};
use crate::sim04::{Burst, Chaos, Emitter, Force, Particles, Shade, Shape, ShapeInst, SimParams};
use crate::{blackbody, render, ui2sim, ui2sim_size, widgets, V3};

/// Values converted back to 0.4 units, rounded to 6 decimals: that recovers 0.4's own
/// numbers exactly (a 1-in-a-million difference grows into a different flame in a fluid).
fn snap(x: f32) -> f32 {
    ((x as f64 * 1e6).round() / 1e6) as f32
}

/// Is this noise force the emitter's "Emission Chaos" (its 0.4 units differed from Fire Chaos)?
fn emission_noise(n: &Node) -> bool {
    n.title == "Emission Chaos"
}

/// Force: Noise in 0.4 units (inverse of legacy Strength / Scale / Speed).
fn noise04(n: &Node) -> Chaos {
    let e = emission_noise(n);
    Chaos {
        strength: snap(n.f("Amplitude") / if e { NOISE_EMIT } else { NOISE_FIRE }),
        scale: snap(if e { 8.0 } else { 25.0 } / n.f("Scale").max(0.01)),
        speed: snap(n.f("Animation speed") / 2.0),
    }
}

/// Shape regions produced by shape node `i` (0.4.0 `shapes_of`).
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
            let ch = Chaos { strength: n.f("Strength"), scale: n.f("Scale"), speed: n.f("Speed") };
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

fn first_shape(g: &Graph, i: usize, port: &str) -> Option<ShapeInst> {
    g.sources(i, port).into_iter().flat_map(|j| shapes_of(g, j, 0)).next()
}

/// Engine inputs for the graph. `prev`: emitter positions one frame earlier (Velocity transfer).
pub fn build04(g: &Graph, prev: Option<&[V3]>, lit: bool, step: f32) -> (SimParams, Shade) {
    let live = g.live();
    let find = |k: Kind| (0..g.nodes.len()).find(|&i| g.nodes[i].kind == k && live[i]);
    let first_of = |i: Option<usize>, port: &str, ks: &[Kind]| -> Option<&Node> {
        i.and_then(|i| g.inputs(&live, i, port).into_iter().map(|j| &g.nodes[j]).find(|n| ks.contains(&n.kind)))
    };
    let sim_i = find(Kind::Simulation);
    let simn = sim_i.map(|i| &g.nodes[i]).cloned().unwrap_or_else(|| template(Kind::Simulation));
    let pct = |n: &Node, name: &str| n.f(name) / 100.0;

    let mut emitters = vec![];
    let mut forces = vec![];
    let mut colliders = vec![];
    let mut emit_light = None;
    if let Some(si) = sim_i {
        let emitter_nodes: Vec<usize> = (0..g.nodes.len()).filter(|&i| matches!(g.nodes[i].kind, Kind::Emitter | Kind::EmitterParticles)).collect();
        for e in g.inputs(&live, si, "Emitters") {
            let n = &g.nodes[e];
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
            let mut vel = if n.kind == Kind::Emitter { ui2sim(n.c("Initial velocity")) } else { [0.0; 3] };
            // Velocity transfer: a moving emitter passes its motion to the gas (none when it stands still)
            if let (Some(prev), Some(k)) = (prev, emitter_nodes.iter().position(|&j| j == e)) {
                if let Some(p0) = prev.get(k) {
                    let tr = if n.kind == Kind::Emitter { pct(n, "Velocity transfer") } else { 0.0 };
                    for a in 0..3 {
                        vel[a] += (pos[a] - p0[a]) * FPS * tr;
                    }
                }
            }
            let mut chaos = None;
            for j in g.sources(e, "Forces") {
                let f = &g.nodes[j];
                if f.params.iter().any(|p| p.name == "Force activity") && !f.b("Force activity") {
                    continue;
                }
                match f.kind {
                    Kind::ForceNoise => chaos = Some(noise04(f)),
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
            if volume && emit_light.is_none() && n.b("Emit light") && n.f("Emissive light intensity") > 0.0 {
                emit_light = Some((shapes[0].center, render::mul(n.c("Emissive light color"), n.f("Emissive light intensity")), shapes[0].half[0] * 4.0));
            }
            emitters.push(Emitter {
                shapes,
                particles,
                fuel: n.f("Fuel rate"),
                smoke: n.f("Smoke rate"),
                temp_k: n.f("Temperature"),
                gradient: if volume { n.f("Emission gradient") } else { 15.0 },
                expansion: if volume { snap(pct(n, "Additional pressure rate")) } else { 0.0 },
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
            if n.params.iter().any(|p| p.name == "Force activity") && !n.b("Force activity") {
                continue;
            }
            let (kind, radius, dir, extra, scale, speed, strength) = match n.kind {
                Kind::ForceLine => (0, 1.0, ui2sim(n.c("Direction")), 0.0, 0.0, 0.0, n.f("Strength")),
                Kind::ForcePoint => (1, n.f("Radius"), [0.0, 1.0, 0.0], 0.0, 0.0, 0.0, n.f("Strength")),
                Kind::ForceToroidal => (2, n.f("Ring radius"), ui2sim(n.c("Axis")), n.f("Tube radius"), 0.0, 0.0, n.f("Strength")),
                Kind::ForceNoise => {
                    let c = noise04(n);
                    (3, 1.0, [0.0, 1.0, 0.0], 0.0, c.scale, c.speed, c.strength)
                }
                Kind::ForceVectorField => (4, n.f("Radius"), ui2sim(n.c("Axis")), n.ch("Field type") as f32, 0.0, 0.0, n.f("Strength")),
                Kind::ForceVorticles => (5, n.f("Area"), [0.0, 1.0, 0.0], 0.0, n.f("Vortex size"), n.f("Rise speed"), n.f("Strength")),
                _ => continue,
            };
            forces.push(Force {
                kind,
                strength,
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
    let grav = pct(&simn, "Gravity multiplier");
    let wind = ui2sim(simn.c("Wind direction").map(|d| d * simn.f("Wind speed")));
    let sp = SimParams {
        time_scale: snap(60.0 / simn.f("Timestep").max(1.0)),
        buoyancy: snap(simn.f("Buoyancy") / 300.0 * grav),
        smoke_weight: snap(pct(&simn, "Smoke weight") * grav),
        vorticity: snap(pct(&simn, "Vorticity intensity")),
        burn_rate: snap(pct(&simn, "Burn rate")),
        ignition_k: simn.f("Ignition temperature"),
        heat: snap(pct(&simn, "Generated heat")),
        smoke_yield: snap(pct(&simn, "Generated smoke")),
        cooling: snap(simn.f("Temperature dissipation (*)") * 1.15 / 32.0),
        smoke_fade: snap(simn.f("Smoke dissipation (*)") * 1.5 / 28.5),
        wind: [wind[0], wind[2]],
        iterations: match simn.ch("Projection Quality") {
            0 => 10,
            1 => 20,
            2 => 40,
            3 => 80,
            _ => simn.f("Projection iterations") as usize,
        },
        substeps: (simn.f("Substeps") as usize).max(1),
        forces,
        colliders,
        emitters,
    };

    // shading
    let shading_i = find(Kind::Shading);
    let shn = shading_i.map(|i| g.nodes[i].clone()).unwrap_or_else(|| template(Kind::Shading));
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
    if point.is_none() {
        point = emit_light;
    }
    // fire colour: the gradient's colours at its cold end, middle and hot end (0.4 used 3 colours)
    let [t_min_k, t_max_k] = shn.r("Flames remap range");
    let colorn = first_of(shading_i, "Fire color", &[Kind::FireGradient, Kind::ColorConstant, Kind::ColorBlackbody]).cloned().unwrap_or_else(|| template(Kind::FireGradient));
    let grad = match colorn.kind {
        Kind::ColorConstant => {
            let c = colorn.c("Color");
            [c.map(|v| v * 0.4), c, c.map(|v| (v * 1.3).min(1.0))]
        }
        Kind::ColorBlackbody => {
            let sat = colorn.f("Saturation");
            [blackbody(t_min_k, sat).map(|v| v * 0.5), blackbody((t_min_k + t_max_k) * 0.5, sat), blackbody(t_max_k, sat)]
        }
        _ => {
            let stops = colorn.grad("Color gradient");
            let space = colorn.ch("Interpolation color space");
            // a stop sitting exactly there gives its own colour (no rounding through a blend)
            [0.0, 0.5, 1.0].map(|k| stops.iter().find(|s| s.pos == k).map_or_else(|| widgets::eval_gradient(&stops, space, k), |s| s.col))
        }
    };
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
        brightness: if shn.b("Render flames") { snap(pct(&shn, "Flames density scale")) } else { 0.0 },
        smoke_color: shn.c("Smoke color"),
        absorption: if shn.b("Render smoke") { snap(pct(&shn, "Smoke density scale") * 3.0) } else { 0.0 },
        shadow: snap(pct(&shn, "Smoke shadow density")),
        detail: [0.0, 0.8, 1.6][simn.ch("Upscaling").min(2)],
        point,
        sparks,
        density: if has_volume { 1.0 } else { 0.0 },
        temp_mul: if has_volume { 1.0 } else { 0.0 },
        sky,
        ground,
        step,
        pass: 0,
        alpha: 0,
        invert_alpha: false,
    };
    (sp, shade)
}

/// The engine's grid for the Simulation node (same as the settings' Domain).
pub fn domain(g: &Graph) -> Domain {
    let simn = g.first(Kind::Simulation).map(|i| g.nodes[i].clone()).unwrap_or_else(|| template(Kind::Simulation));
    crate::domain_of(&simn).0
}

/// Settings the engine (or the app around it) acts on; everything else is shown dimmed.
pub fn engine_uses(kind: Kind, name: &str) -> bool {
    // read in `build04` above
    const ENGINE: &[&str] = &[
        "Additional pressure rate", "Amplitude", "Animation speed", "Area", "Axis", "Azimuth", "Bottom color", "Buoyancy", "Burn rate",
        "Color", "Color gradient", "Count", "Direction", "Duration", "Duration of burst", "Edge softness", "Elevation", "Emission gradient",
        "Emissive light color", "Emissive light intensity", "Emit light", "Emitter activity", "Field type", "Final radius",
        "Flames density scale", "Flames remap range", "Force activity", "Fuel emission", "Fuel rate", "Generated heat", "Generated smoke",
        "Gravity", "Gravity multiplier", "Height", "Hollow", "Ignition temperature", "Initial velocity", "Intensity",
        "Interpolation color space", "Lifetime", "Mask softness", "Mode", "Offset", "Particle size", "Position", "Projection Quality",
        "Projection iterations", "Radius", "Range", "Render flames", "Render smoke", "Render sparks", "Ring radius", "Rise speed",
        "Saturation", "Scale", "Seed", "Shape type", "Shell thickness", "Size", "Size multiplier", "Smoke color", "Smoke density scale",
        "Smoke dissipation (*)", "Smoke emission", "Smoke rate", "Smoke shadow density", "Smoke weight", "Solid", "Spark brightness",
        "Speed", "Spread", "Spread angle", "Start time", "Strength", "Substeps", "Temperature", "Temperature dissipation (*)",
        "Temperature emission", "Time between bursts", "Timestep", "Top color", "Torus thickness", "Tube radius", "Upscaling",
        "Velocity spread", "Velocity transfer", "Vortex size", "Vorticity intensity", "Wander", "Wind direction", "Wind speed",
    ];
    // handled by the app: the grid, the timeline, viewport gizmos and histograms
    const APP: &[&str] = &[
        "Voxel count", "Voxel size", "Zero position on X", "Zero position on Y", "Zero position on Z", "Bounds Position", "Bounds size",
        "Looping Mode", "Enable gizmo", "Show Force Mask", "Visual mode", "Arrow type", "Density", "Show emitter", "Emitter color",
        "Temperature Histogram", "Smoke Histogram", "Noise force preview",
    ];
    let force = kind.group() == Some("Force");
    match kind.group() {
        Some("Camera" | "Import" | "Export" | "Modulator") | None => true,
        _ if matches!(kind, Kind::Render) => true,
        // a force's Mode (volume/particles) and the noise Seed are 0.5 additions
        _ if force && matches!(name, "Mode" | "Seed") => false,
        _ => ENGINE.contains(&name) || APP.contains(&name),
    }
}
