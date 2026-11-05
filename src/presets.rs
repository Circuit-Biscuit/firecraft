// Ready-made looks. Each preset starts from the default graph and changes settings or adds nodes.
// Every preset differs in shape or motion, not just colour.
use crate::graph::{Graph, Kind, Val};
use Val::*;
use eframe::egui::pos2;

pub struct Preset {
    pub name: &'static str,
    pub category: &'static str,
    /// Frame shown in the preset's picture (its most telling moment).
    pub frame: u32,
}

const fn p(name: &'static str, category: &'static str, frame: u32) -> Preset {
    Preset { name, category, frame }
}

pub const PRESETS: &[Preset] = &[
    p("Campfire", "Fire", 150),
    p("Candle Flame", "Fire", 150),
    p("Wildfire Line", "Fire", 170),
    p("Fire Ring", "Fire", 150),
    p("Fire Tornado", "Fire", 220),
    p("Burning Wall", "Fire", 200),
    p("Oil Pool Fire", "Fire", 220),
    p("Gas Burner", "Fire", 120),
    p("Flamethrower", "Fire", 110),
    p("Burning Meteor", "Fire", 95),
    p("Smoke Column", "Smoke", 260),
    p("Steam Jet", "Smoke", 160),
    p("Smoke Ring", "Smoke", 120),
    p("Incense Wisps", "Smoke", 260),
    p("Smoke Under Ceiling", "Smoke", 240),
    p("Smoke Around Pillar", "Smoke", 220),
    p("Ground Fog", "Smoke", 260),
    p("Storm Cloud", "Smoke", 280),
    p("Twin Tornadoes", "Smoke", 260),
    p("Smoke Grenade", "Smoke", 240),
    p("Dry Ice Waterfall", "Smoke", 240),
    p("Ground Explosion", "Explosion", 45),
    p("Aerial Fireball", "Explosion", 45),
    p("Mushroom Cloud", "Explosion", 200),
    p("Muzzle Flash", "Explosion", 12),
    p("Billowing Puffs", "Smoke", 230),
    p("Dust Impact", "Explosion", 70),
    p("Artillery Blast", "Explosion", 40),
    p("Chain Explosion", "Explosion", 75),
    p("Magic Portal", "Magic", 180),
    p("Wandering Fireballs", "Magic", 160),
    p("Energy Orb", "Magic", 160),
    p("Nebula", "Magic", 300),
    p("Toxic Cloud", "Magic", 240),
    p("Spirit Swirl", "Magic", 220),
    p("Sparks Fountain", "Particles", 120),
    p("Campfire with Embers", "Particles", 150),
    p("Rocket Launch", "Particles", 160),
    p("Firework Burst", "Particles", 60),
    p("Volcanic Eruption", "Particles", 180),
    p("Pipe Leak", "Tech", 170),
    p("Wind Tunnel", "Tech", 200),
    p("Cooling Tower", "Tech", 260),
    p("Exhaust Vents", "Tech", 200),
];

// node indices in the default graph
const SHAPE: usize = 0;
const EMCHAOS: usize = 1;
const CHAOS: usize = 2;
const EMIT: usize = 3;
const SIM: usize = 4;
const AMB: usize = 6;
const GRAD: usize = 7;
const SUN: usize = 8;
const SKY: usize = 9;
const GROUND: usize = 10;
const SHADING: usize = 11;
const CAM: usize = 12;
const SCENE: usize = 13;

fn set(g: &mut Graph, i: usize, name: &str, v: Val) {
    g.nodes[i].set(name, v);
}

/// Add a node; preset nodes are laid out in tidy rows below the default graph
/// (the requested position is ignored so they never cover existing nodes).
fn add(g: &mut Graph, k: Kind, _x: f32, _y: f32, title: &str) -> usize {
    let n = g.nodes.len().saturating_sub(16);
    let i = g.add(k, pos2(40.0 + (n % 5) as f32 * 145.0, 360.0 + (n / 5) as f32 * 80.0));
    g.nodes[i].title = title.into();
    i
}

/// Turn the default fire into a smoke source.
fn smoke(g: &mut Graph, rate: f32, temp: f32, color: [f32; 3]) {
    set(g, EMIT, "Fuel rate", F(0.0));
    set(g, EMIT, "Smoke rate", F(rate));
    set(g, EMIT, "Temperature", F(temp));
    set(g, SIM, "Smoke dissipation", F(0.25));
    set(g, SHADING, "Smoke color", C(color));
    set(g, EMCHAOS, "Strength", F(0.5));
}

fn outdoors(g: &mut Graph) {
    g.nodes[SKY].enabled = true;
    g.nodes[GROUND].enabled = true;
    set(g, AMB, "Intensity", F(0.45));
}

fn gradient(g: &mut Graph, cold: [f32; 3], mid: [f32; 3], hot: [f32; 3]) {
    set(g, GRAD, "Cold color", C(cold));
    set(g, GRAD, "Mid color", C(mid));
    set(g, GRAD, "Hot color", C(hot));
}

/// A new force node wired into the simulation.
fn force(g: &mut Graph, k: Kind, y: f32, title: &str) -> usize {
    let f = add(g, k, 170.0, y, title);
    g.connect(f, "Force", SIM, "Forces");
    f
}

/// A modulator driving `param` of node `target`.
fn drive(g: &mut Graph, m: usize, target: usize, param: &str, axis: u8) {
    let id = g.nodes[m].id;
    if let Some(p) = g.nodes[target].params.iter_mut().find(|p| p.name == param) {
        p.driver = Some(id);
        p.axis = axis;
    }
}

/// Particle emitter wired into the simulation, sparks shown in the scene.
fn particles(g: &mut Graph, y: f32, title: &str) -> usize {
    let e = add(g, Kind::EmitterParticles, 170.0, y, title);
    g.connect(e, "Emitter", SIM, "Emitters");
    g.connect(SIM, "Particles", SCENE, "Particles");
    e
}

fn collider(g: &mut Graph, shape: usize, x: f32, y: f32) {
    let c = add(g, Kind::Collider, x, y, "");
    g.connect(shape, "Shape", c, "Shapes");
    g.connect(c, "Collider", SIM, "Colliders");
}


/// A burst shape (expanding shell) at a position, starting at `start` seconds.
fn burst(g: &mut Graph, x: f32, y: f32, pos: [f32; 3], radius: f32, dur: f32, start: f32) -> usize {
    let b = add(g, Kind::ShapeBurst, x, y, "");
    set(g, b, "Final radius", F(radius));
    set(g, b, "Duration", F(dur));
    set(g, b, "Start time", F(start));
    set(g, b, "Position", V(pos));
    b
}

/// Replace the emitter's shape with node `s`.
fn emit_from(g: &mut Graph, s: usize) {
    g.nodes[SHAPE].enabled = false;
    g.unplug(EMIT, "Shapes");
    g.connect(s, "Shape", EMIT, "Shapes");
}

pub fn apply(g: &mut Graph, i: usize) {
    *g = Graph::default();
    let name = PRESETS[i].name;
    match name {
        // ---------------- fire ----------------
        "Candle Flame" => {
            set(g, SHAPE, "Size", F(0.025));
            set(g, EMIT, "Fuel rate", F(700.0));
            set(g, EMIT, "Velocity spread", F(0.0));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.03]));
            set(g, CHAOS, "Strength", F(0.12));
            set(g, CHAOS, "Scale", F(2.0));
            set(g, EMCHAOS, "Strength", F(0.0));
            set(g, SIM, "Cooling", F(2.2));
            set(g, SIM, "Domain size", F(0.3));
            let l = add(g, Kind::LightPoint, 315.0, 220.0, "Candle Glow");
            set(g, l, "Position", V([0.0, 0.0, 0.08]));
            g.connect(l, "Light", SHADING, "Lights");
        }
        "Wildfire Line" => {
            outdoors(g);
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Size", F(0.12));
            set(g, SHAPE, "Scale", V([0.4, 9.0, 0.25]));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.1]));
            set(g, CHAOS, "Strength", F(2.0));
            set(g, EMIT, "Fuel rate", F(800.0));
            set(g, SIM, "Cooling", F(0.8));
            set(g, SIM, "Smoke from burning", F(0.15));
            set(g, SIM, "Smoke dissipation", F(1.0));
            set(g, SHADING, "Smoke color", C([0.06, 0.06, 0.06]));
            set(g, SIM, "Domain size", F(2.4));
        }
        "Fire Ring" => {
            set(g, SHAPE, "Shape type", Ch(3));
            set(g, SHAPE, "Size", F(0.45));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.4]));
            set(g, SHAPE, "Torus thickness", F(0.18));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.05]));
            set(g, SIM, "Domain size", F(1.6));
        }
        "Fire Tornado" => {
            let f = force(g, Kind::ForceVectorField, 140.0, "Spin");
            set(g, f, "Field type", Ch(2));
            set(g, f, "Strength", F(5.0));
            set(g, f, "Radius", F(0.45));
            set(g, f, "Position", V([0.0, 0.0, 0.8]));
            let u = force(g, Kind::ForceVectorField, 90.0, "Updraft");
            set(g, u, "Field type", Ch(3));
            set(g, u, "Strength", F(6.0));
            set(g, u, "Radius", F(0.45));
            set(g, u, "Position", V([0.0, 0.0, 0.7]));
            set(g, SHAPE, "Shape type", Ch(2));
            set(g, SHAPE, "Size", F(0.28));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.2]));
            set(g, EMIT, "Fuel rate", F(600.0));
            set(g, SIM, "Cooling", F(0.5));
            set(g, SIM, "Smoke from burning", F(0.2));
            set(g, SHADING, "Smoke color", C([0.1, 0.09, 0.08]));
            set(g, SIM, "Domain size", F(1.8));
        }
        "Burning Wall" => {
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Scale", V([0.12, 2.5, 2.2]));
            set(g, SHAPE, "Size", F(0.25));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.3]));
            set(g, SIM, "Smoke from burning", F(0.9));
            set(g, SIM, "Smoke dissipation", F(0.4));
            set(g, SHADING, "Smoke color", C([0.05, 0.05, 0.05]));
            set(g, SIM, "Domain size", F(2.2));
        }
        "Oil Pool Fire" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.55));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.12]));
            set(g, EMIT, "Fuel rate", F(700.0));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.15]));
            set(g, SIM, "Smoke from burning", F(1.4));
            set(g, SIM, "Smoke dissipation", F(0.15));
            set(g, SHADING, "Smoke color", C([0.03, 0.03, 0.03]));
            set(g, SIM, "Domain size", F(3.4));
        }
        "Gas Burner" => {
            let s = add(g, Kind::ShapeParticles, 40.0, 232.0, "Jets");
            set(g, s, "Count", F(10.0));
            set(g, s, "Spread", F(0.25));
            set(g, s, "Particle size", F(0.035));
            set(g, s, "Position", V([0.0, 0.0, -0.05]));
            emit_from(g, s);
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 4.0]));
            set(g, EMIT, "Velocity spread", F(0.0));
            set(g, EMIT, "Fuel rate", F(700.0));
            set(g, EMCHAOS, "Strength", F(0.0));
            set(g, CHAOS, "Strength", F(0.4));
            set(g, SIM, "Cooling", F(3.0));
            gradient(g, [0.0, 0.02, 0.35], [0.05, 0.3, 1.0], [0.7, 0.9, 1.0]);
            set(g, SIM, "Domain size", F(0.8));
        }
        "Flamethrower" => {
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Size", F(0.07));
            set(g, SHAPE, "Scale", V([1.0, 4.0, 1.0]));
            set(g, EMIT, "Position", V([0.0, -1.0, 0.35]));
            set(g, EMIT, "Initial velocity", V([0.0, 18.0, 1.0]));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Velocity spread", F(0.5));
            set(g, CHAOS, "Strength", F(3.0));
            set(g, SIM, "Buoyancy", F(0.6));
            set(g, SIM, "Smoke from burning", F(0.25));
            set(g, SIM, "Cooling", F(0.7));
            set(g, SHADING, "Smoke color", C([0.08, 0.08, 0.08]));
            set(g, SIM, "Domain size", F(3.0));
        }
        "Burning Meteor" => {
            set(g, SHAPE, "Size", F(0.12));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, EMIT, "Smoke rate", F(300.0));
            let m = add(g, Kind::ModCycle, 40.0, 360.0, "Flight");
            set(g, m, "Start value", F(1.3));
            set(g, m, "End value", F(-1.3));
            set(g, m, "Period", F(2.0));
            drive(g, m, EMIT, "Position", 1);
            set(g, EMIT, "Position", V([0.0, 1.3, 1.1]));
            set(g, SIM, "Buoyancy", F(0.2));
            set(g, SIM, "Smoke from burning", F(1.0));
            set(g, SIM, "Smoke dissipation", F(0.3));
            set(g, SHADING, "Smoke color", C([0.25, 0.22, 0.2]));
            let c = add(g, Kind::ColorBlackbody, 320.0, 103.0, "");
            g.nodes[GRAD].enabled = false;
            g.connect(c, "Color", SHADING, "Fire color");
            set(g, SIM, "Domain size", F(3.0));
        }
        // ---------------- smoke ----------------
        "Smoke Column" => {
            smoke(g, 300.0, 1000.0, [0.62, 0.62, 0.64]);
            set(g, SHAPE, "Size", F(0.25));
            set(g, SIM, "Domain size", F(2.8));
        }
        "Steam Jet" => {
            smoke(g, 700.0, 600.0, [0.88, 0.89, 0.92]);
            set(g, SHAPE, "Size", F(0.1));
            set(g, EMIT, "Position", V([0.0, -1.1, 0.5]));
            set(g, EMIT, "Initial velocity", V([0.0, 9.0, 0.0]));
            set(g, SIM, "Buoyancy", F(0.25));
            set(g, SIM, "Smoke dissipation", F(0.6));
            set(g, SIM, "Domain size", F(2.8));
        }
        "Smoke Ring" => {
            smoke(g, 900.0, 700.0, [0.78, 0.78, 0.82]);
            set(g, EMIT, "Duration of burst", F(0.25));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, SHAPE, "Shape type", Ch(3));
            set(g, SHAPE, "Size", F(0.35));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.5]));
            set(g, SHAPE, "Torus thickness", F(0.35));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.2]));
            let f = force(g, Kind::ForceToroidal, 140.0, "Ring Spin");
            set(g, f, "Strength", F(8.0));
            set(g, f, "Ring radius", F(0.28));
            set(g, f, "Tube radius", F(0.14));
            set(g, f, "Position", V([0.0, 0.0, 0.2]));
            set(g, f, "Axis", V([0.0, 0.0, -1.0]));
            set(g, CHAOS, "Strength", F(0.3));
            set(g, SIM, "Smoke dissipation", F(0.05));
            set(g, SIM, "Domain size", F(2.2));
        }
        "Incense Wisps" => {
            smoke(g, 140.0, 800.0, [0.82, 0.84, 0.88]);
            set(g, SHAPE, "Size", F(0.03));
            set(g, EMCHAOS, "Strength", F(0.0));
            set(g, CHAOS, "Strength", F(0.0));
            let f = force(g, Kind::ForceVorticles, 140.0, "Curls");
            set(g, f, "Strength", F(5.0));
            set(g, f, "Vortex size", F(0.08));
            set(g, f, "Area", F(0.4));
            set(g, f, "Position", V([0.0, 0.0, 0.6]));
            set(g, SIM, "Domain size", F(1.2));
        }
        "Smoke Under Ceiling" => {
            smoke(g, 400.0, 1400.0, [0.55, 0.55, 0.57]);
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Ceiling");
            set(g, s, "Shape type", Ch(1));
            set(g, s, "Size", F(0.5));
            set(g, s, "Scale", V([2.2, 2.2, 0.08]));
            set(g, s, "Position", V([0.0, 0.0, 0.9]));
            collider(g, s, 170.0, 360.0);
            set(g, SIM, "Domain size", F(1.8));
        }
        "Smoke Around Pillar" => {
            smoke(g, 450.0, 600.0, [0.7, 0.7, 0.73]);
            set(g, EMIT, "Position", V([0.0, -1.0, 0.4]));
            set(g, EMIT, "Initial velocity", V([0.0, 4.0, 0.0]));
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Pillar");
            set(g, s, "Shape type", Ch(2));
            set(g, s, "Size", F(0.16));
            set(g, s, "Scale", V([1.0, 1.0, 6.0]));
            set(g, s, "Position", V([0.0, 0.0, 0.5]));
            collider(g, s, 170.0, 360.0);
            set(g, SIM, "Buoyancy", F(0.15));
            set(g, SIM, "Domain size", F(2.6));
        }
        "Ground Fog" => {
            smoke(g, 200.0, 320.0, [0.82, 0.84, 0.88]);
            outdoors(g);
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Size", F(0.5));
            set(g, SHAPE, "Scale", V([2.5, 2.5, 0.12]));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.25]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, SIM, "Smoke weight", F(1.2));
            set(g, SIM, "Smoke dissipation", F(0.1));
            let f = force(g, Kind::ForceLine, 140.0, "Drift");
            set(g, f, "Direction", V([0.0, 1.0, 0.0]));
            set(g, f, "Strength", F(0.6));
            set(g, SIM, "Domain size", F(3.0));
        }
        "Storm Cloud" => {
            smoke(g, 250.0, 800.0, [0.5, 0.52, 0.58]);
            outdoors(g);
            set(g, SHAPE, "Size", F(0.6));
            set(g, SHAPE, "Scale", V([2.0, 2.0, 0.5]));
            set(g, EMIT, "Position", V([0.0, 0.0, 1.0]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            let f = force(g, Kind::ForceVorticles, 140.0, "Billows");
            set(g, f, "Strength", F(3.0));
            set(g, f, "Vortex size", F(0.4));
            set(g, f, "Area", F(1.5));
            set(g, f, "Position", V([0.0, 0.0, 1.2]));
            set(g, SIM, "Buoyancy", F(0.3));
            set(g, SIM, "Smoke dissipation", F(0.0));
            set(g, SIM, "Domain size", F(4.0));
        }
        "Twin Tornadoes" => {
            smoke(g, 350.0, 900.0, [0.62, 0.48, 0.36]);
            outdoors(g);
            let s2 = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Second Base");
            set(g, s2, "Size", F(0.12));
            set(g, s2, "Position", V([0.0, 0.6, 0.0]));
            set(g, SHAPE, "Size", F(0.12));
            set(g, SHAPE, "Position", V([0.0, -0.6, 0.0]));
            let bl = add(g, Kind::ShapeBlend, 110.0, 330.0, "Both Bases");
            g.unplug(EMIT, "Shapes");
            g.connect(SHAPE, "Shape", bl, "Shape A");
            g.connect(s2, "Shape", bl, "Shape B");
            g.connect(bl, "Shape", EMIT, "Shapes");
            for (k, y) in [(-0.6f32, 90.0), (0.6, 140.0)] {
                let f = force(g, Kind::ForceVectorField, y, "Twister");
                set(g, f, "Field type", Ch(2));
                set(g, f, "Strength", F(8.0));
                set(g, f, "Radius", F(0.3));
                set(g, f, "Position", V([0.0, k, 0.8]));
            }
            set(g, SIM, "Domain size", F(3.0));
        }
        "Smoke Grenade" => {
            smoke(g, 1000.0, 500.0, [0.95, 0.32, 0.12]);
            outdoors(g);
            set(g, SHAPE, "Size", F(0.12));
            set(g, EMIT, "Position", V([0.0, -0.8, -0.15]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 1.5]));
            set(g, SIM, "Smoke weight", F(0.15));
            set(g, SIM, "Wind", V([0.0, 0.5, 0.0]));
            set(g, SIM, "Smoke dissipation", F(0.1));
            set(g, SIM, "Domain size", F(3.0));
        }
        "Dry Ice Waterfall" => {
            smoke(g, 500.0, 300.0, [0.85, 0.92, 1.0]);
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Size", F(0.2));
            set(g, SHAPE, "Scale", V([1.0, 2.0, 0.4]));
            set(g, EMIT, "Position", V([0.0, -0.4, 1.0]));
            set(g, EMIT, "Initial velocity", V([0.0, 1.2, 0.0]));
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Shelf");
            set(g, s, "Shape type", Ch(1));
            set(g, s, "Size", F(0.4));
            set(g, s, "Scale", V([1.2, 1.5, 0.15]));
            set(g, s, "Position", V([0.0, -0.4, 0.8]));
            collider(g, s, 170.0, 360.0);
            set(g, SIM, "Buoyancy", F(0.0));
            set(g, SIM, "Smoke weight", F(2.0));
            set(g, SIM, "Smoke dissipation", F(0.2));
            set(g, SIM, "Domain size", F(2.2));
        }
        // ---------------- explosions ----------------
        "Ground Explosion" => {
            set(g, SHAPE, "Size", F(0.35));
            set(g, EMIT, "Duration of burst", F(0.25));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Smoke rate", F(250.0));
            set(g, EMIT, "Expansion", F(3.0));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.3]));
            set(g, SIM, "Smoke from burning", F(1.5));
            set(g, SIM, "Cooling", F(0.7));
            set(g, SIM, "Smoke dissipation", F(0.1));
            set(g, SHADING, "Smoke color", C([0.2, 0.2, 0.2]));
            outdoors(g);
            set(g, SIM, "Domain size", F(4.5));
        }
        "Aerial Fireball" => {
            set(g, SHAPE, "Size", F(0.3));
            set(g, EMIT, "Duration of burst", F(0.2));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Expansion", F(4.0));
            set(g, EMIT, "Position", V([0.0, 0.0, 1.4]));
            set(g, SIM, "Smoke from burning", F(0.2));
            set(g, SIM, "Buoyancy", F(0.4));
            set(g, SIM, "Cooling", F(0.5));
            set(g, SIM, "Domain size", F(4.8));
        }
        "Mushroom Cloud" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.4));
            set(g, EMIT, "Duration of burst", F(0.7));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Smoke rate", F(500.0));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.0]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 5.0]));
            let f = force(g, Kind::ForceToroidal, 140.0, "Rolling Cap");
            set(g, f, "Strength", F(6.0));
            set(g, f, "Ring radius", F(0.7));
            set(g, f, "Tube radius", F(0.35));
            set(g, f, "Position", V([0.0, 0.0, 2.0]));
            set(g, f, "Axis", V([0.0, 0.0, 1.0]));
            set(g, SIM, "Smoke from burning", F(1.5));
            set(g, SIM, "Smoke dissipation", F(0.05));
            set(g, SHADING, "Smoke color", C([0.45, 0.38, 0.32]));
            set(g, SIM, "Domain size", F(5.5));
        }
        "Muzzle Flash" => {
            set(g, SHAPE, "Size", F(0.15));
            set(g, EMIT, "Duration of burst", F(0.12));
            set(g, EMIT, "Time between bursts", F(0.8));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Expansion", F(2.5));
            set(g, EMIT, "Initial velocity", V([0.0, 10.0, 0.0]));
            set(g, EMIT, "Position", V([0.0, -0.8, 0.5]));
            set(g, SIM, "Cooling", F(1.2));
            set(g, SIM, "Domain size", F(1.6));
        }
        "Billowing Puffs" => {
            // puffs sent up one after another, merging into a billowing cloud
            outdoors(g);
            smoke(g, 1000.0, 1300.0, [0.72, 0.72, 0.74]);
            set(g, SHAPE, "Size", F(0.14));
            set(g, EMIT, "Duration of burst", F(0.18));
            set(g, EMIT, "Time between bursts", F(0.7));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 6.0]));
            set(g, SIM, "Buoyancy", F(1.6));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.1]));
            set(g, SIM, "Smoke dissipation", F(0.15));
            set(g, CHAOS, "Strength", F(0.6));
            set(g, SIM, "Domain size", F(2.8));
        }
        "Dust Impact" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.55));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.35]));
            set(g, EMIT, "Duration of burst", F(0.2));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Fuel rate", F(0.0));
            set(g, EMIT, "Smoke rate", F(1000.0));
            set(g, EMIT, "Temperature", F(700.0));
            set(g, EMIT, "Expansion", F(4.0));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.2]));
            set(g, SIM, "Smoke weight", F(0.6));
            set(g, SIM, "Smoke dissipation", F(0.05));
            set(g, SHADING, "Smoke color", C([0.6, 0.5, 0.38]));
            set(g, SIM, "Domain size", F(3.6));
        }
        "Artillery Blast" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.3));
            set(g, EMIT, "Duration of burst", F(0.15));
            set(g, EMIT, "Time between bursts", F(100.0));
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Smoke rate", F(500.0));
            set(g, EMIT, "Expansion", F(3.5));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.0]));
            set(g, SIM, "Smoke from burning", F(1.2));
            set(g, SHADING, "Smoke color", C([0.22, 0.21, 0.2]));
            let e = particles(g, 330.0, "Debris");
            set(g, e, "Count", F(24.0));
            set(g, e, "Speed", F(7.0));
            set(g, e, "Spread angle", F(75.0));
            set(g, e, "Gravity", F(-9.0));
            set(g, e, "Lifetime", F(1.0));
            set(g, e, "Fuel rate", F(250.0));
            set(g, e, "Spark brightness", F(2.5));
            set(g, SIM, "Domain size", F(4.5));
        }
        "Chain Explosion" => {
            let a = burst(g, 20.0, 200.0, [0.0, -1.0, 0.3], 0.45, 0.25, 0.0);
            let b = burst(g, 20.0, 250.0, [0.0, 0.0, 0.6], 0.45, 0.25, 0.4);
            let c = burst(g, 20.0, 300.0, [0.0, 1.0, 0.3], 0.45, 0.25, 0.8);
            let ab = add(g, Kind::ShapeBlend, 90.0, 220.0, "");
            g.connect(a, "Shape", ab, "Shape A");
            g.connect(b, "Shape", ab, "Shape B");
            let abc = add(g, Kind::ShapeBlend, 120.0, 280.0, "");
            g.connect(ab, "Shape", abc, "Shape A");
            g.connect(c, "Shape", abc, "Shape B");
            emit_from(g, abc);
            set(g, EMIT, "Fuel rate", F(1000.0));
            set(g, EMIT, "Smoke rate", F(200.0));
            set(g, EMIT, "Expansion", F(2.0));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, SIM, "Smoke from burning", F(0.8));
            set(g, SHADING, "Smoke color", C([0.15, 0.14, 0.13]));
            set(g, SIM, "Domain size", F(3.8));
        }
        // ---------------- magic ----------------
        "Magic Portal" => {
            gradient(g, [0.15, 0.0, 0.3], [0.6, 0.2, 1.0], [0.9, 0.8, 1.0]);
            set(g, SHAPE, "Shape type", Ch(3));
            set(g, SHAPE, "Size", F(0.5));
            set(g, SHAPE, "Torus thickness", F(0.12));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.3]));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.3]));
            set(g, CAM, "Pitch", F(55.0));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            let f = force(g, Kind::ForceVectorField, 140.0, "Portal Spin");
            set(g, f, "Field type", Ch(2));
            set(g, f, "Axis", V([0.0, 0.0, 1.0]));
            set(g, f, "Strength", F(6.0));
            set(g, f, "Radius", F(0.6));
            set(g, f, "Position", V([0.0, 0.0, 0.3]));
            set(g, SIM, "Buoyancy", F(0.15));
            set(g, SIM, "Domain size", F(1.8));
        }
        "Wandering Fireballs" => {
            let s = add(g, Kind::ShapeParticles, 40.0, 232.0, "Fireballs");
            set(g, s, "Count", F(6.0));
            set(g, s, "Spread", F(0.6));
            set(g, s, "Particle size", F(0.07));
            set(g, s, "Wander", F(1.5));
            set(g, s, "Position", V([0.0, 0.0, 0.6]));
            emit_from(g, s);
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, SIM, "Buoyancy", F(0.3));
            set(g, SIM, "Smoke from burning", F(0.3));
            set(g, SHADING, "Smoke color", C([0.1, 0.1, 0.1]));
            set(g, SIM, "Domain size", F(2.0));
        }
        "Energy Orb" => {
            gradient(g, [0.0, 0.1, 0.4], [0.2, 0.6, 1.0], [0.85, 0.95, 1.0]);
            set(g, SHAPE, "Size", F(0.26));
            set(g, SHAPE, "Shape type", Ch(0));
            let n = add(g, Kind::ShapeNoise, 110.0, 232.0, "Crackle");
            set(g, n, "Strength", F(0.9));
            set(g, n, "Scale", F(3.0));
            set(g, n, "Speed", F(4.0));
            g.unplug(EMIT, "Shapes");
            g.connect(SHAPE, "Shape", n, "Shape");
            g.connect(n, "Shape", EMIT, "Shapes");
            set(g, EMIT, "Position", V([0.0, 0.0, 0.5]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            let f = force(g, Kind::ForcePoint, 140.0, "Pull");
            set(g, f, "Strength", F(-6.0));
            set(g, f, "Radius", F(0.5));
            set(g, f, "Position", V([0.0, 0.0, 0.5]));
            set(g, SIM, "Buoyancy", F(0.1));
            set(g, SIM, "Domain size", F(1.4));
        }
        "Nebula" => {
            smoke(g, 120.0, 600.0, [0.6, 0.3, 0.85]);
            set(g, SHAPE, "Size", F(0.35));
            set(g, EMIT, "Position", V([0.0, 0.0, 0.8]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 0.0]));
            set(g, SIM, "Buoyancy", F(0.0));
            set(g, SIM, "Smoke dissipation", F(0.0));
            let f = force(g, Kind::ForceVorticles, 140.0, "Swirls");
            set(g, f, "Strength", F(3.0));
            set(g, f, "Vortex size", F(0.3));
            set(g, f, "Area", F(1.0));
            set(g, f, "Position", V([0.0, 0.0, 0.8]));
            let l = add(g, Kind::LightPoint, 315.0, 220.0, "Star");
            set(g, l, "Color", C([0.5, 0.8, 1.0]));
            set(g, l, "Intensity", F(5.0));
            set(g, l, "Position", V([0.0, 0.0, 0.8]));
            g.connect(l, "Light", SHADING, "Lights");
            g.nodes[SUN].enabled = false;
            set(g, AMB, "Intensity", F(0.05));
            set(g, SIM, "Domain size", F(2.4));
        }
        "Toxic Cloud" => {
            smoke(g, 350.0, 450.0, [0.35, 0.9, 0.2]);
            outdoors(g);
            set(g, SHAPE, "Size", F(0.15));
            set(g, EMIT, "Position", V([0.0, 0.0, -0.1]));
            set(g, SIM, "Smoke weight", F(0.8));
            set(g, SIM, "Smoke dissipation", F(0.05));
            let f = force(g, Kind::ForceVorticles, 140.0, "Roll");
            set(g, f, "Strength", F(2.0));
            set(g, f, "Vortex size", F(0.3));
            set(g, f, "Area", F(1.0));
            set(g, f, "Position", V([0.0, 0.0, 0.2]));
            set(g, SIM, "Domain size", F(3.0));
        }
        "Spirit Swirl" => {
            smoke(g, 250.0, 900.0, [0.35, 0.95, 1.0]);
            set(g, SHAPE, "Size", F(0.08));
            let f = force(g, Kind::ForceVectorField, 140.0, "Spiral");
            set(g, f, "Field type", Ch(2));
            set(g, f, "Strength", F(6.0));
            set(g, f, "Radius", F(0.35));
            set(g, f, "Position", V([0.0, 0.0, 0.7]));
            let l = add(g, Kind::LightPoint, 315.0, 220.0, "Glow");
            set(g, l, "Color", C([0.3, 0.9, 1.0]));
            set(g, l, "Intensity", F(4.0));
            set(g, l, "Position", V([0.0, 0.0, 0.5]));
            g.connect(l, "Light", SHADING, "Lights");
            g.nodes[SUN].enabled = false;
            set(g, AMB, "Intensity", F(0.05));
            set(g, SIM, "Domain size", F(1.6));
        }
        // ---------------- particles ----------------
        "Sparks Fountain" => {
            g.nodes[EMIT].enabled = false;
            let e = particles(g, 330.0, "Sparks");
            set(g, e, "Count", F(24.0));
            set(g, e, "Speed", F(4.5));
            set(g, e, "Spread angle", F(18.0));
            set(g, e, "Gravity", F(-7.0));
            set(g, e, "Particle size", F(0.045));
            set(g, e, "Fuel rate", F(250.0));
            set(g, e, "Spark brightness", F(8.0));
            set(g, SIM, "Domain size", F(2.4));
        }
        "Campfire with Embers" => {
            let e = particles(g, 330.0, "Embers");
            set(g, e, "Speed", F(1.5));
            set(g, e, "Spread angle", F(30.0));
            set(g, e, "Gravity", F(0.5));
            set(g, e, "Lifetime", F(2.0));
            set(g, e, "Particle size", F(0.02));
            set(g, e, "Fuel rate", F(0.0));
            set(g, e, "Spark brightness", F(3.0));
            set(g, SIM, "Smoke from burning", F(0.25));
            set(g, SHADING, "Smoke color", C([0.12, 0.12, 0.12]));
            set(g, SIM, "Domain size", F(1.6));
        }
        "Rocket Launch" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.1));
            set(g, EMIT, "Position", V([0.0, 0.0, 1.2]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, -10.0]));
            set(g, EMIT, "Fuel rate", F(900.0));
            set(g, EMIT, "Smoke rate", F(400.0));
            set(g, SIM, "Smoke from burning", F(1.2));
            set(g, SHADING, "Smoke color", C([0.8, 0.8, 0.82]));
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Launch Pad");
            set(g, s, "Shape type", Ch(1));
            set(g, s, "Size", F(0.5));
            set(g, s, "Scale", V([3.0, 3.0, 0.1]));
            set(g, s, "Position", V([0.0, 0.0, -0.3]));
            collider(g, s, 170.0, 380.0);
            let e = particles(g, 330.0, "Sparks");
            set(g, e, "Direction", V([0.0, 0.0, -1.0]));
            set(g, e, "Speed", F(8.0));
            set(g, e, "Spread angle", F(12.0));
            set(g, e, "Position", V([0.0, 0.0, 1.2]));
            set(g, e, "Fuel rate", F(100.0));
            set(g, SIM, "Domain size", F(3.6));
        }
        "Firework Burst" => {
            g.nodes[EMIT].enabled = false;
            gradient(g, [0.4, 0.0, 0.25], [1.0, 0.2, 0.7], [1.0, 0.85, 1.0]);
            let e = particles(g, 330.0, "Stars");
            set(g, e, "Count", F(24.0));
            set(g, e, "Speed", F(4.0));
            set(g, e, "Spread angle", F(180.0));
            set(g, e, "Gravity", F(-2.0));
            set(g, e, "Lifetime", F(1.4));
            set(g, e, "Particle size", F(0.05));
            set(g, e, "Position", V([0.0, 0.0, 1.2]));
            set(g, e, "Fuel rate", F(600.0));
            set(g, e, "Spark brightness", F(8.0));
            set(g, SIM, "Buoyancy", F(0.2));
            set(g, SIM, "Domain size", F(3.0));
        }
        "Volcanic Eruption" => {
            outdoors(g);
            set(g, SHAPE, "Size", F(0.25));
            set(g, EMIT, "Fuel rate", F(500.0));
            set(g, EMIT, "Smoke rate", F(600.0));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 4.0]));
            set(g, SIM, "Smoke from burning", F(1.5));
            set(g, SIM, "Smoke dissipation", F(0.05));
            set(g, SHADING, "Smoke color", C([0.28, 0.26, 0.25]));
            let e = particles(g, 330.0, "Lava Bombs");
            set(g, e, "Speed", F(6.5));
            set(g, e, "Spread angle", F(35.0));
            set(g, e, "Gravity", F(-9.0));
            set(g, e, "Fuel rate", F(500.0));
            set(g, e, "Spark brightness", F(2.0));
            set(g, SIM, "Domain size", F(4.2));
        }
        // ---------------- tech ----------------
        "Pipe Leak" => {
            smoke(g, 800.0, 500.0, [0.88, 0.9, 0.93]);
            set(g, SHAPE, "Size", F(0.06));
            set(g, EMIT, "Position", V([0.0, -0.1, 0.62]));
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 7.0]));
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Pipe");
            set(g, s, "Shape type", Ch(1));
            set(g, s, "Size", F(0.07));
            set(g, s, "Scale", V([1.0, 12.0, 1.0]));
            set(g, s, "Position", V([0.0, 0.0, 0.5]));
            collider(g, s, 170.0, 360.0);
            set(g, SIM, "Buoyancy", F(0.2));
            set(g, SIM, "Domain size", F(1.6));
        }
        "Wind Tunnel" => {
            smoke(g, 900.0, 300.0, [0.92, 0.93, 0.96]);
            set(g, SHAPE, "Shape type", Ch(1));
            set(g, SHAPE, "Size", F(0.5));
            set(g, SHAPE, "Scale", V([0.08, 0.04, 1.0]));
            set(g, EMIT, "Position", V([0.0, -0.8, 0.6]));
            set(g, EMIT, "Initial velocity", V([0.0, 6.0, 0.0]));
            set(g, EMIT, "Velocity spread", F(0.0));
            set(g, EMCHAOS, "Strength", F(0.6));
            set(g, EMCHAOS, "Scale", F(6.0));
            set(g, CHAOS, "Strength", F(0.0));
            let s = add(g, Kind::ShapePrimitive, 40.0, 360.0, "Sphere");
            set(g, s, "Size", F(0.28));
            set(g, s, "Position", V([0.0, -0.1, 0.6]));
            collider(g, s, 170.0, 360.0);
            set(g, SIM, "Buoyancy", F(0.0));
            set(g, SIM, "Smoke dissipation", F(0.5));
            set(g, SIM, "Domain size", F(2.0));
        }
        "Cooling Tower" => {
            smoke(g, 450.0, 900.0, [0.93, 0.94, 0.96]);
            outdoors(g);
            set(g, SHAPE, "Shape type", Ch(2));
            set(g, SHAPE, "Size", F(0.55));
            set(g, SHAPE, "Scale", V([1.0, 1.0, 0.1]));
            let m = add(g, Kind::ShapeModifier, 110.0, 232.0, "Rim");
            set(g, m, "Hollow", F(0.35));
            g.unplug(EMIT, "Shapes");
            g.connect(SHAPE, "Shape", m, "Shape");
            g.connect(m, "Shape", EMIT, "Shapes");
            set(g, EMIT, "Position", V([0.0, 0.0, 0.2]));
            set(g, SIM, "Wind", V([0.0, 0.5, 0.0]));
            set(g, SIM, "Smoke dissipation", F(0.15));
            set(g, SIM, "Domain size", F(4.0));
        }
        "Exhaust Vents" => {
            smoke(g, 350.0, 1200.0, [0.25, 0.25, 0.26]);
            outdoors(g);
            let s = add(g, Kind::ShapeParticles, 40.0, 232.0, "Vents");
            set(g, s, "Count", F(5.0));
            set(g, s, "Spread", F(0.9));
            set(g, s, "Particle size", F(0.06));
            set(g, s, "Wander", F(0.0));
            set(g, s, "Seed", F(7.0));
            let t = add(g, Kind::ShapeTransform, 110.0, 232.0, "In a Row");
            set(g, t, "Scale", V([0.05, 1.0, 0.05]));
            g.connect(s, "Shape", t, "Shape");
            let m = add(g, Kind::ShapeModifier, 150.0, 232.0, "Vent Size");
            set(g, m, "Size multiplier", F(20.0));
            g.connect(t, "Shape", m, "Shape");
            emit_from(g, m);
            set(g, EMIT, "Initial velocity", V([0.0, 0.0, 3.0]));
            set(g, SIM, "Wind", V([0.0, 0.6, 0.0]));
            set(g, SIM, "Domain size", F(3.0));
        }
        _ => {}
    }
    // frame the camera to the domain
    let domain = g.nodes[SIM].f("Domain size");
    let cam = &mut g.nodes[CAM];
    cam.set("Distance", F(domain * 2.6));
    cam.set("Target height", F(domain * 0.43));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preset_builds_a_valid_graph() {
        let mut g = Graph::default();
        for i in 0..PRESETS.len() {
            apply(&mut g, i);
            let text = g.save();
            assert!(Graph::load(&text).is_ok(), "{} saves and loads", PRESETS[i].name);
            assert!(g.wires.iter().all(|w| w.from < g.nodes.len() && w.to < g.nodes.len()));
        }
    }

    #[test]
    fn every_preset_has_a_built_in_picture() {
        for p in PRESETS {
            assert!(crate::preset_thumbs::PICTURES.iter().any(|(n, _)| *n == p.name), "{} needs a picture: cargo test render_preset_pictures -- --ignored", p.name);
        }
    }
}
