// Node graph: data model, editor canvas and the node-details settings panel.
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use egui::epaint::CubicBezierShape;

pub const ACCENT: Color32 = Color32::from_rgb(0, 120, 215);
pub const GRAPH_BG: Color32 = Color32::from_rgb(37, 40, 41);
pub const NODE_BG: Color32 = Color32::from_rgb(26, 27, 29);
pub const TEXT: Color32 = Color32::from_rgb(197, 197, 197);
pub const DIM: Color32 = Color32::from_rgb(92, 94, 96);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    // Shape
    ShapePrimitive,
    ShapeBurst,
    ShapeParticles,
    ShapeNoise,
    ShapeTransform,
    ShapeBlend,
    ShapeModifier,
    // Emitter
    Emitter,
    EmitterParticles,
    // Collider
    Collider,
    // Force
    ForceLine,
    ForceToroidal,
    ForceNoise,
    ForcePoint,
    ForceVectorField,
    ForceVorticles,
    // Light
    LightAmbient,
    LightDirectional,
    LightPoint,
    // Camera
    Camera,
    CameraFree,
    // Import / Export
    Import,
    ExportImage,
    ExportFlipbook,
    ExportVolume,
    // Modulator
    ModConstant,
    ModOscillator,
    ModCycle,
    ModMath,
    ModTimeShift,
    ModCombine,
    ModAdsr,
    ModMidi,
    // Color
    FireGradient,
    ColorConstant,
    ColorBlackbody,
    // Scene parts
    Skybox,
    Ground,
    Scene,
    Simulation,
    Shading,
    VolumeProcessing,
    Render,
    // Notes
    Comment,
    Note,
}

pub const ALL: [Kind; 45] = [
    Kind::ShapePrimitive,
    Kind::ShapeBurst,
    Kind::ShapeParticles,
    Kind::ShapeNoise,
    Kind::ShapeTransform,
    Kind::ShapeBlend,
    Kind::ShapeModifier,
    Kind::Emitter,
    Kind::EmitterParticles,
    Kind::Collider,
    Kind::ForceLine,
    Kind::ForceToroidal,
    Kind::ForceNoise,
    Kind::ForcePoint,
    Kind::ForceVectorField,
    Kind::ForceVorticles,
    Kind::LightAmbient,
    Kind::LightDirectional,
    Kind::LightPoint,
    Kind::Camera,
    Kind::CameraFree,
    Kind::Import,
    Kind::ExportImage,
    Kind::ExportFlipbook,
    Kind::ExportVolume,
    Kind::ModConstant,
    Kind::ModOscillator,
    Kind::ModCycle,
    Kind::ModMath,
    Kind::ModTimeShift,
    Kind::ModCombine,
    Kind::ModAdsr,
    Kind::ModMidi,
    Kind::FireGradient,
    Kind::ColorConstant,
    Kind::ColorBlackbody,
    Kind::Skybox,
    Kind::Ground,
    Kind::Scene,
    Kind::Simulation,
    Kind::Shading,
    Kind::VolumeProcessing,
    Kind::Render,
    Kind::Comment,
    Kind::Note,
];

/// Groups of the add-node menu, in menu order. Kinds without a group are listed on their own.
pub const GROUPS: [&str; 10] = ["Shape", "Emitter", "Collider", "Force", "Light", "Camera", "Import", "Export", "Modulator", "Color"];

impl Kind {
    pub fn name(self) -> &'static str {
        use Kind::*;
        match self {
            ShapePrimitive => "Shape: Primitive",
            ShapeBurst => "Shape: Burst",
            ShapeParticles => "Shape: Particles",
            ShapeNoise => "Shape: Noise",
            ShapeTransform => "Shape: Transform",
            ShapeBlend => "Shape: Blend",
            ShapeModifier => "Shape: Modifier",
            Emitter => "Emitter: Volume",
            EmitterParticles => "Emitter: Particles",
            Collider => "Collider",
            ForceLine => "Force: Line",
            ForceToroidal => "Force: Toroidal",
            ForceNoise => "Force: Noise",
            ForcePoint => "Force: Point",
            ForceVectorField => "Force: Vector Field",
            ForceVorticles => "Force: Vorticles",
            LightAmbient => "Light: Ambient",
            LightDirectional => "Light: Directional",
            LightPoint => "Light: Point",
            Camera => "Camera: Orbit",
            CameraFree => "Camera: Free",
            Import => "Import",
            ExportImage => "Export: Image Sequence",
            ExportFlipbook => "Export: Flipbook",
            ExportVolume => "Export: Volume",
            ModConstant => "Mod: Constant",
            ModOscillator => "Mod: Oscillator",
            ModCycle => "Mod: Cycle",
            ModMath => "Mod: Math",
            ModTimeShift => "Mod: Time Shift",
            ModCombine => "Mod: Combine",
            ModAdsr => "Mod: ADSR",
            ModMidi => "Mod: MIDI",
            FireGradient => "Color: Gradient",
            ColorConstant => "Color: Constant",
            ColorBlackbody => "Color: Blackbody",
            Skybox => "Skybox",
            Ground => "Ground",
            Scene => "Scene",
            Simulation => "Simulation",
            Shading => "Shading",
            VolumeProcessing => "Volume Processing",
            Render => "Render",
            Comment => "Comment",
            Note => "Note",
        }
    }

    /// Kind from a saved project (also maps names used by older FireCraft versions).
    pub fn from_saved(s: &str) -> Option<(Kind, &'static str)> {
        match s {
            "FireShape" => return Some((Kind::ShapePrimitive, "Fire Shape")),
            "FireChaos" => return Some((Kind::ForceNoise, "Fire Chaos")),
            "EmissionChaos" => return Some((Kind::ForceNoise, "Emission Chaos")),
            _ => {}
        }
        ALL.iter().find(|k| format!("{k:?}") == s).map(|k| (*k, ""))
    }

    pub fn group(self) -> Option<&'static str> {
        use Kind::*;
        Some(match self {
            ShapePrimitive | ShapeBurst | ShapeParticles | ShapeNoise | ShapeTransform | ShapeBlend | ShapeModifier => "Shape",
            Emitter | EmitterParticles => "Emitter",
            Collider => "Collider",
            ForceLine | ForceToroidal | ForceNoise | ForcePoint | ForceVectorField | ForceVorticles => "Force",
            LightAmbient | LightDirectional | LightPoint => "Light",
            Camera | CameraFree => "Camera",
            Import => "Import",
            ExportImage | ExportFlipbook | ExportVolume => "Export",
            ModConstant | ModOscillator | ModCycle | ModMath | ModTimeShift | ModCombine | ModAdsr | ModMidi => "Modulator",
            FireGradient | ColorConstant | ColorBlackbody => "Color",
            _ => return None,
        })
    }

    /// Only one of these may exist in a project.
    pub fn singleton(self) -> bool {
        matches!(self, Kind::Scene | Kind::Simulation | Kind::Shading | Kind::VolumeProcessing | Kind::Render)
    }

    pub fn is_modulator(self) -> bool {
        self.group() == Some("Modulator")
    }

    pub fn is_note(self) -> bool {
        matches!(self, Kind::Comment | Kind::Note)
    }

    /// (inputs, outputs)
    pub fn ports(self) -> (&'static [&'static str], &'static [&'static str]) {
        use Kind::*;
        match self {
            ShapePrimitive | ShapeBurst | ShapeParticles => (&[], &["Shape"]),
            ShapeNoise | ShapeTransform | ShapeModifier => (&["Shape"], &["Shape"]),
            ShapeBlend => (&["Shape A", "Shape B"], &["Shape"]),
            Emitter | EmitterParticles => (&["Shapes", "Forces"], &["Emitter"]),
            Collider => (&["Shapes"], &["Collider"]),
            ForceLine | ForceToroidal | ForceNoise | ForcePoint | ForceVectorField | ForceVorticles => (&["Mask Shapes"], &["Force"]),
            Simulation => (&["Forces", "Emitters", "Colliders"], &["Volume", "Particles"]),
            VolumeProcessing => (&["Volume", "Mask Shapes"], &["Volume", "Grid"]),
            LightAmbient | LightDirectional | LightPoint => (&[], &["Light"]),
            FireGradient | ColorConstant | ColorBlackbody => (&[], &["Color"]),
            Skybox => (&["Sun"], &["Skybox"]),
            Ground => (&[], &["Ground"]),
            Shading => (&["Lights", "Fire color"], &["Shading"]),
            Camera | CameraFree => (&["Control"], &["Camera"]),
            Scene => (&["Shading", "Skybox", "Ground", "Volume", "Particles"], &["Scene"]),
            Render => (&["Camera", "Scene"], &["Image"]),
            Import => (&[], &["Emitter"]),
            ExportImage | ExportFlipbook => (&["Image"], &[]),
            ExportVolume => (&["Grid"], &[]),
            ModConstant | ModOscillator | ModCycle | ModAdsr | ModMidi => (&[], &["Value"]),
            ModMath | ModCombine => (&["A", "B"], &["Value"]),
            ModTimeShift => (&["Value"], &["Value"]),
            Comment | Note => (&[], &[]),
        }
    }

    pub fn color(self) -> Color32 {
        match self.group() {
            Some("Shape") => Color32::from_rgb(170, 175, 190),
            Some("Emitter") => Color32::from_rgb(89, 255, 200),
            Some("Collider") => Color32::from_rgb(255, 95, 95),
            Some("Force") => Color32::from_rgb(120, 170, 255),
            Some("Light") => Color32::from_rgb(255, 225, 120),
            Some("Camera") | Some("Import") | Some("Export") => Color32::from_rgb(110, 190, 255),
            Some("Modulator") => Color32::from_rgb(255, 120, 110),
            Some("Color") => Color32::from_rgb(160, 120, 255),
            _ => match self {
                Kind::Simulation => Color32::from_rgb(255, 110, 160),
                Kind::VolumeProcessing => Color32::from_rgb(255, 120, 60),
                Kind::Skybox | Kind::Ground => Color32::from_rgb(150, 150, 150),
                Kind::Shading => Color32::from_rgb(230, 200, 150),
                Kind::Note => Color32::from_rgb(230, 200, 90),
                _ => Color32::from_rgb(110, 190, 255),
            },
        }
    }

    pub fn has_power(self) -> bool {
        !matches!(self, Kind::Scene | Kind::Render | Kind::Camera | Kind::CameraFree | Kind::Comment | Kind::Note)
    }

    fn params(self) -> Vec<Param> {
        use Val::*;
        const SHAPES: &[&str] = &["Sphere", "Box", "Cylinder", "Torus"];
        const RES: &[&str] = &["Low", "Medium", "High", "Ultra"];
        const MODE: &[&str] = &["Add", "Replace"];
        const WAVES: &[&str] = &["Sine", "Square", "Triangle", "Saw"];
        const OPS: &[&str] = &["Add", "Subtract", "Multiply", "Divide", "Min", "Max", "Power"];
        const FIELDS: &[&str] = &["Push out", "Pull in", "Swirl", "Updraft"];
        const MIDI: &[&str] = &["Control change", "Note velocity"];
        const ZERO: &[&str] = &["Center", "Min", "Max"];
        const FMODE: &[&str] = &["Volume", "Particles", "Volume And Particles"];
        const ONOFF: &[&str] = &["No", "Yes"];
        let _ = ONOFF;
        // shared by every force: activity, mode, rotation, field masks and the viewport gizmo
        let force_common = |v_: &mut Vec<Param>, constant: f32, temperature: f32| {
            v_.insert(0, p("General", "Mode", Ch(0), FMODE));
            v_.insert(0, b("General", "Show Force Mask", false));
            v_.insert(0, b("General", "Tether Mask Shapes", false));
            v_.insert(0, b("General", "Force activity", true));
            if !v_.iter().any(|q| q.name == "Position") {
                v_.push(v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"));
            }
            v_.push(b("Transform", "Import control", false));
            v_.push(v("Transform", "Rotation", [0.0, 0.0, 0.0], -180.0, 180.0, "°"));
            v_.push(f("Masking", "Constant mask", constant, 0.0, 100.0, "%"));
            v_.push(f("Masking", "Velocity mask", 0.0, 0.0, 100.0, "%"));
            v_.push(f("Masking", "Temperature mask", temperature, 0.0, 100.0, "%"));
            v_.push(f("Masking", "Smoke mask", 0.0, 0.0, 100.0, "%"));
            v_.push(r("Masking", "Smoke Range", [0.0, 300.0], (0.0, 1000.0), "%"));
            v_.push(r("Masking", "Temperature Range", [0.0, 10000.0], (0.0, 10000.0), "K"));
            v_.push(f("Masking", "Mask softness", 0.3, 0.0, 1.0, ""));
            v_.push(b("Visual", "Enable gizmo", true));
            v_.push(p("Visual", "Visual mode", Ch(0), &["3D Grid", "Slice"]));
            v_.push(p("Visual", "Arrow type", Ch(0), &["Arrow", "Line"]));
            v_.push(f("Visual", "Density", 0.66, 0.1, 1.0, ""));
        };
        match self {
            Kind::ShapePrimitive => vec![
                b("General", "Shape Activity", true),
                p("General", "Shape type", Ch(0), SHAPES).lab("Type"),
                f("General", "Size", 0.24, 0.02, 4.0, "m").lab("Radius"),
                v("General", "Scale", [1.0, 1.0, 1.0], 0.05, 10.0, ""),
                f("General", "Torus thickness", 0.3, 0.05, 1.0, "").when("Shape type", 3),
                b("General", "Fill motion gaps", false),
                b("Transform", "Import control", false),
                v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
                v("Transform", "Rotation", [0.0, 0.0, 0.0], -180.0, 180.0, "°"),
            ],
            Kind::ShapeBurst => vec![
                f("Burst", "Final radius", 0.8, 0.05, 6.0, "m"),
                f("Burst", "Duration", 0.4, 0.02, 10.0, "s"),
                f("Burst", "Shell thickness", 0.25, 0.02, 1.0, ""),
                f("Burst", "Start time", 0.0, 0.0, 30.0, "s"),
                v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
            ],
            Kind::ShapeParticles => vec![
                f("Particles", "Count", 8.0, 1.0, 24.0, "").int(),
                f("Particles", "Spread", 0.4, 0.0, 4.0, "m"),
                f("Particles", "Particle size", 0.06, 0.01, 1.0, "m"),
                f("Particles", "Wander", 0.3, 0.0, 3.0, "m/s"),
                f("Particles", "Seed", 1.0, 0.0, 999.0, "").int(),
                v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
            ],
            Kind::ShapeNoise => vec![f("Noise", "Strength", 0.7, 0.0, 1.0, ""), f("Noise", "Scale", 1.0, 0.1, 8.0, ""), f("Noise", "Speed", 1.0, 0.0, 8.0, "")],
            Kind::ShapeTransform => vec![
                v("Transform", "Offset", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
                v("Transform", "Scale", [1.0, 1.0, 1.0], 0.05, 10.0, ""),
            ],
            Kind::ShapeBlend => vec![p("Blend", "Mode", Ch(0), &["Union (both shapes)", "A only", "B only"])],
            Kind::ShapeModifier => vec![
                f("Modifier", "Hollow", 0.0, 0.0, 0.95, ""),
                f("Modifier", "Size multiplier", 1.0, 0.05, 30.0, ""),
                f("Modifier", "Edge softness", 15.0, 0.0, 50.0, ""),
            ],
            Kind::Emitter => vec![
                b("Activity", "Emitter activity", true),
                f("Activity", "Duration of burst", 0.5, 0.0, 10.0, "s"),
                f("Activity", "Time between bursts", 0.0, 0.0, 100.0, "s"),
                b("Transform", "Import control", false),
                v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
                v("Transform", "Rotation", [0.0, 0.0, 0.0], -180.0, 180.0, "°"),
                p("Emission", "Fuel emission", Ch(0), MODE),
                f("Emission", "Fuel rate", 250.0, 0.0, 1000.0, "%/s"),
                p("Emission", "Smoke emission", Ch(0), MODE),
                f("Emission", "Smoke rate", 0.0, 0.0, 1000.0, "%/s"),
                p("Emission", "Temperature emission", Ch(1), MODE),
                f("Emission", "Temperature", 4100.0, 300.0, 6000.0, "K"),
                f("Emission", "Emission gradient", 15.0, 0.0, 50.0, ""),
                f("Pressure", "Additional pressure rate", 0.0, 0.0, 1000.0, "%/s"),
                f("Pressure", "Pressure random intensity", 0.0, 0.0, 100.0, "%"),
                f("Pressure", "Pressure random scale", 0.1, 0.01, 2.0, "m"),
                f("Pressure", "Pressure random seed", 0.0, 0.0, 999.0, "").int(),
                f("Pressure", "Pressure random speed", 0.0, 0.0, 10.0, ""),
                f("Forces", "Velocity transfer", 100.0, 0.0, 200.0, "%"),
                v("Forces", "Initial velocity", [0.0, 0.0, 1.0], -25.0, 25.0, "m/s"),
                f("Forces", "Velocity spread", 1.0, 0.0, 3.0, ""),
                b("Visuals", "Show emitter", false),
                b("Visuals", "Shadow caster", false),
                c("Visuals", "Emitter color", [0.5, 0.5, 0.5]),
                b("Visuals", "Emit light", true),
                c("Visuals", "Emissive light color", [0.55, 0.5, 1.0]),
                f("Visuals", "Emissive light intensity", 0.0, 0.0, 20.0, ""),
            ],
            Kind::EmitterParticles => vec![
                b("Activity", "Emitter activity", true),
                f("Particles", "Count", 12.0, 1.0, 24.0, "").int(),
                f("Particles", "Lifetime", 1.2, 0.05, 10.0, "s"),
                f("Particles", "Speed", 2.0, 0.0, 20.0, "m/s"),
                f("Particles", "Spread angle", 35.0, 0.0, 180.0, "°"),
                v("Particles", "Direction", [0.0, 0.0, 1.0], -1.0, 1.0, ""),
                f("Particles", "Gravity", -2.0, -20.0, 20.0, "m/s²"),
                f("Particles", "Particle size", 0.05, 0.01, 0.5, "m"),
                f("Particles", "Seed", 1.0, 0.0, 999.0, "").int(),
                v("Transform", "Position", [0.0, 0.0, 0.0], -4.0, 4.0, "m"),
                f("Emission", "Fuel rate", 400.0, 0.0, 1000.0, "%/s"),
                f("Emission", "Smoke rate", 0.0, 0.0, 1000.0, "%/s"),
                f("Emission", "Temperature", 3500.0, 300.0, 6000.0, "K"),
                b("Visuals", "Render sparks", true),
                f("Visuals", "Spark brightness", 1.0, 0.0, 10.0, ""),
            ],
            Kind::Collider => vec![b("Collider", "Solid", true)],
            Kind::ForceLine => {
                let mut v_ = vec![f("Force", "Strength", 1.0, -20.0, 20.0, ""), v("Force", "Direction", [0.0, 1.0, 0.0], -1.0, 1.0, "")];
                force_common(&mut v_, 100.0, 0.0);
                v_
            }
            Kind::ForceToroidal => {
                let mut v_ = vec![
                    f("Force", "Strength", 3.0, -20.0, 20.0, ""),
                    f("Force", "Ring radius", 0.4, 0.02, 4.0, "m"),
                    f("Force", "Tube radius", 0.15, 0.01, 2.0, "m"),
                    v("Force", "Axis", [0.0, 0.0, 1.0], -1.0, 1.0, ""),
                    v("Transform", "Position", [0.0, 0.0, 0.3], -4.0, 4.0, "m"),
                ];
                force_common(&mut v_, 100.0, 0.0);
                v_
            }
            Kind::ForceNoise => {
                let mut v_ = vec![
                    f("General", "Scale", 25.0, 1.0, 400.0, "cm"),
                    f("General", "Seed", 330.0, 0.0, 9999.0, "").int(),
                    f("General", "Octaves", 1.0, 1.0, 8.0, "").int(),
                    f("General", "Lacunarity", 2.0, 1.0, 4.0, ""),
                    f("General", "Gain", 0.5, 0.0, 1.0, ""),
                    f("General", "Amplitude", 19.333, 0.0, 200.0, ""),
                    v("General", "Bias", [0.0, 0.0, 0.0], -50.0, 50.0, ""),
                    f("General", "Animation speed", 2.0, 0.0, 20.0, ""),
                    b("General", "Curl noise", true),
                    ui("General", "Noise force preview", "noise_preview"),
                ];
                force_common(&mut v_, 0.0, 100.0);
                v_
            }
            Kind::ForcePoint => {
                let mut v_ = vec![
                    f("Force", "Strength", -3.0, -50.0, 50.0, ""),
                    f("Force", "Radius", 0.6, 0.02, 6.0, "m"),
                    v("Transform", "Position", [0.0, 0.0, 0.6], -4.0, 4.0, "m"),
                ];
                force_common(&mut v_, 100.0, 0.0);
                v_
            }
            Kind::ForceVectorField => {
                let mut v_ = vec![
                    p("Field", "Field type", Ch(2), FIELDS),
                    f("Field", "Strength", 2.0, -20.0, 20.0, ""),
                    f("Field", "Radius", 0.6, 0.02, 6.0, "m"),
                    v("Field", "Axis", [0.0, 0.0, 1.0], -1.0, 1.0, ""),
                    v("Transform", "Position", [0.0, 0.0, 0.4], -4.0, 4.0, "m"),
                ];
                force_common(&mut v_, 100.0, 0.0);
                v_
            }
            Kind::ForceVorticles => {
                let mut v_ = vec![
                    f("Vorticles", "Strength", 2.0, 0.0, 20.0, ""),
                    f("Vorticles", "Vortex size", 0.12, 0.01, 2.0, "m"),
                    f("Vorticles", "Area", 0.6, 0.05, 6.0, "m"),
                    f("Vorticles", "Rise speed", 1.0, 0.0, 10.0, ""),
                    v("Transform", "Position", [0.0, 0.0, 0.5], -4.0, 4.0, "m"),
                ];
                force_common(&mut v_, 100.0, 0.0);
                v_
            }
            Kind::LightAmbient => vec![f("Light", "Intensity", 0.15, 0.0, 2.0, ""), c("Light", "Color", [0.6, 0.7, 0.9])],
            Kind::LightDirectional => vec![
                f("Light", "Intensity", 1.2, 0.0, 4.0, ""),
                c("Light", "Color", [1.0, 0.95, 0.85]),
                f("Light", "Elevation", 50.0, 0.0, 90.0, "°"),
                f("Light", "Azimuth", 30.0, -180.0, 180.0, "°"),
            ],
            Kind::LightPoint => vec![
                f("Light", "Intensity", 2.0, 0.0, 20.0, ""),
                c("Light", "Color", [1.0, 0.6, 0.3]),
                f("Light", "Range", 1.0, 0.05, 10.0, "m"),
                v("Transform", "Position", [0.0, 0.0, 0.4], -4.0, 4.0, "m"),
            ],
            Kind::FireGradient => vec![
                p("Gradient", "Interpolation color space", Ch(0), &["RGB", "Linear RGB", "HSV", "OKLab"]),
                g("Gradient", "Color gradient", vec![
                    GStop { pos: 0.0, col: [0.45, 0.025, 0.0] },
                    GStop { pos: 0.5, col: [1.0, 0.2, 0.012] },
                    GStop { pos: 1.0, col: [1.0, 0.62, 0.22] },
                ]),
            ],
            Kind::ColorConstant => vec![c("Color", "Color", [1.0, 0.45, 0.1])],
            Kind::ColorBlackbody => vec![f("Blackbody", "Saturation", 1.0, 0.0, 2.0, "")],
            Kind::Skybox => vec![c("Sky", "Top color", [0.06, 0.12, 0.28]), c("Sky", "Bottom color", [0.2, 0.22, 0.26])],
            Kind::Ground => vec![c("Ground", "Color", [0.3, 0.3, 0.3]), f("Ground", "Height", -0.2, -2.0, 2.0, "m")],
            Kind::Shading => vec![
                b("Lighting", "Advanced parameters", false),
                f("Lighting", "Shadowing sharpness", 100.0, 0.0, 100.0, "%"),
                f("Lighting", "Lighting anisotropy", 50.0, -100.0, 100.0, "%"),
                f("Lighting", "Ambient occlusion", 0.0, 0.0, 100.0, "%"),
                c("Lighting", "Extinction color", [1.0, 1.0, 1.0]),
                f("Lighting", "Smoke shadow density", 100.0, 0.0, 400.0, "%"),
                f("Lighting", "Smoke shadow intensity", 50.0, 0.0, 100.0, "%"),
                f("Lighting", "Shadow steps", 64.0, 8.0, 128.0, "").int().adv(),
                f("Lighting", "Shadow max distance", 100.0, 10.0, 100.0, "%").adv(),
                f("Scattering", "Direct light contribution", 25.0, 0.0, 400.0, "%"),
                f("Scattering", "Flames contribution", 0.0, 0.0, 400.0, "%"),
                f("Scattering", "Particles contribution", 0.0, 0.0, 400.0, "%"),
                f("Scattering", "Scattering radius", 0.15, 0.01, 2.0, "m"),
                f("Scattering", "Scattering occlusion", 0.75, 0.0, 1.0, ""),
                b("Emissive masking", "Masking Active", false),
                f("Emissive masking", "Density reference", 0.3, 0.0, 2.0, ""),
                f("Emissive masking", "Width", 0.3, 0.01, 2.0, ""),
                f("Emissive masking", "Masking ramp", 0.4, 0.05, 4.0, ""),
                f("Emissive masking", "Masking intensity", 100.0, 0.0, 100.0, "%"),
                f("Emissive masking", "Low frequencies", 50.0, 0.0, 100.0, "%"),
                f("Emissive masking", "Medium frequencies", 50.0, 0.0, 100.0, "%"),
                f("Emissive masking", "High frequencies", 50.0, 0.0, 100.0, "%"),
                b("Flames", "Render flames", true),
                ui("Flames", "Temperature Histogram", "hist_temp"),
                r("Flames", "Flames remap range", [800.0, 4300.0], (0.0, 10000.0), "K").lab("Coloring remap range"),
                f("Flames", "Flames remap ramp", 1.0, 0.1, 8.0, "").lab("Coloring remap ramp"),
                b("Flames", "Clamp the remapping", true),
                f("Flames", "Shaping by flames", 0.0, 0.0, 2000.0, "%"),
                f("Flames", "Shaping by density", 0.0, 0.0, 1000.0, "%"),
                p("Flames", "Flames coloring", Ch(0), &["Exposed Color Gradient", "Color Gradient", "Blackbody", "Single Color"]),
                c("Flames", "Fire color", [1.0, 0.45, 0.12]),
                b("Flames", "Flames translucency", false),
                f("Flames", "Flames density min limit", 0.0, 0.0, 100.0, "%"),
                f("Flames", "Flames density scale", 100.0, 0.0, 1000.0, "%"),
                f("Flames", "Flames opacity", 0.0, 0.0, 100.0, "%"),
                b("Smoke", "Render smoke", true),
                b("Smoke", "Remap by temperature", false),
                ui("Smoke", "Smoke Histogram", "hist_smoke"),
                r("Smoke", "Smoke remap range", [0.0, 1.0], (0.0, 10000.0), "").lab("Coloring remap range"),
                f("Smoke", "Smoke remap ramp", 1.0, 0.1, 8.0, "").lab("Coloring remap ramp"),
                p("Smoke", "Smoke coloring", Ch(0), &["Single Color", "Color Gradient"]),
                c("Smoke", "Smoke color", [0.1, 0.1, 0.1]),
                f("Smoke", "Smoke density min limit", 0.0, 0.0, 100.0, "%"),
                f("Smoke", "Smoke density clamp", 1000.0, 0.0, 1000.0, "%"),
                f("Smoke", "Smoke density scale", 100.0, 0.0, 1000.0, "%"),
                b("Fuel", "Render fuel", false),
                c("Fuel", "Fuel color", [0.3, 0.55, 1.0]).when("Render fuel", 1),
                f("Fuel", "Fuel density scale", 100.0, 0.0, 1000.0, "%").when("Render fuel", 1),
                b("Rendering", "Raymarch sharpening", true),
                f("Rendering", "Smoke sharpening", 250.0, 100.0, 500.0, "%").when("Raymarch sharpening", 1),
                f("Rendering", "Flames sharpening", 250.0, 100.0, 500.0, "%").when("Raymarch sharpening", 1),
                f("Rendering", "Temperature sharpening", 250.0, 100.0, 500.0, "%").when("Raymarch sharpening", 1),
            ],
            Kind::Camera => vec![
                f("Camera", "Field of view", 40.0, 10.0, 100.0, "°"),
                f("Camera", "Distance", 2.6, 0.5, 40.0, "m"),
                f("Camera", "Yaw", 90.0, -180.0, 180.0, "°"),
                f("Camera", "Pitch", 8.0, -89.0, 89.0, "°"),
                f("Camera", "Target height", 0.43, -1.0, 8.0, "m"),
            ],
            Kind::CameraFree => vec![
                f("Camera", "Field of view", 40.0, 10.0, 100.0, "°"),
                v("Camera", "Position", [2.6, 0.0, 0.6], -40.0, 40.0, "m"),
                v("Camera", "Look at", [0.0, 0.0, 0.45], -40.0, 40.0, "m"),
            ],
            Kind::Simulation => vec![
                p("Domain", "Resolution", Ch(2), RES),
                f("Domain", "Domain size", 1.0, 0.3, 8.0, "m"),
                f("Domain", "Time scale", 1.0, 0.1, 3.0, ""),
                f("Domain", "Pressure iterations", 10.0, 4.0, 120.0, ""),
                f("Domain", "Substeps", 2.0, 1.0, 8.0, ""),
                f("Fluid", "Buoyancy", 1.0, 0.0, 4.0, ""),
                f("Fluid", "Smoke weight", 0.1, 0.0, 2.0, ""),
                f("Fluid", "Vorticity", 0.0, 0.0, 2.0, ""),
                f("Combustion", "Burn rate", 1.0, 0.0, 5.0, ""),
                f("Combustion", "Ignition temperature", 500.0, 300.0, 3000.0, "K"),
                f("Combustion", "Heat from burning", 1.0, 0.0, 4.0, ""),
                f("Combustion", "Smoke from burning", 0.0, 0.0, 4.0, ""),
                f("Dissipation", "Cooling", 1.15, 0.0, 5.0, ""),
                f("Dissipation", "Smoke dissipation", 1.5, 0.0, 5.0, ""),
                v("Wind", "Wind", [0.0, 0.0, 0.0], -3.0, 3.0, "m/s"),
            ],
            Kind::VolumeProcessing => vec![
                p("Post Process", "Volume post process", Ch(3), &["None", "Motion Blur", "Sharpen", "Dilate"]),
                f("Post Process", "Post process strength", 50.0, 0.0, 100.0, "%"),
                b("Post Modulation 1", "Active", false),
                p("Post Modulation 1", "Modulated channel", Ch(0), &["Smoke", "Flames"]).when("Active", 1),
                f("Post Modulation 1", "Noise scale", 10.0, 1.0, 200.0, "cm").when("Active", 1),
                f("Post Modulation 1", "Modulation intensity", 50.0, 0.0, 100.0, "%").when("Active", 1),
                f("Post Modulation 1", "Modulation speed", 1.0, 0.0, 10.0, "").when("Active", 1),
                f("Slicing Mask", "X- wall offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "X- wall width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "X+ wall offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "X+ wall width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Y- wall offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Y- wall width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Y+ wall offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Y+ wall width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Z- ground offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Z- ground width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Z+ roof offset", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Z+ roof width", 0.0, 0.0, 100.0, "%"),
                f("Slicing Mask", "Mask ramp", 1.0, 0.1, 8.0, ""),
            ],
            Kind::Scene => vec![
                f("Tonemapping", "Gamma", 2.2, 1.0, 3.0, ""),
                f("Tonemapping", "Exposure", 1.0, 0.0, 8.0, ""),
                f("Tonemapping", "Color vibrance", 0.0, -1.0, 1.0, ""),
                p("Tonemapping", "HDR Tonemapping", Ch(2), &["None", "Reinhard", "ACES", "Filmic", "Uchimura", "AgX"]),
                f("Tonemapping", "Tonemap dithering", 0.0, 0.0, 100.0, "%"),
                f("Tonemapping", "Contrast", 0.0, -1.0, 1.0, ""),
                f("Tonemapping", "Brightness", 0.0, -1.0, 1.0, ""),
                f("Tonemapping", "Saturation", 0.0, -1.0, 1.0, ""),
                p("Colors", "Channels swizzling", Ch(0), &["RGB", "RBG", "GRB", "GBR", "BRG", "BGR"]),
                f("Colors", "Bloom Intensity", 0.0, 0.0, 10.0, ""),
                f("Colors", "Bloom Radius", 0.2, 0.0, 1.0, ""),
                f("Vignetting", "Width", 300.0, 0.0, 2000.0, "pixels").int(),
                f("Vignetting", "Offset", 100.0, 0.0, 2000.0, "pixels").int(),
                f("Vignetting", "Roundness", 50.0, 0.0, 100.0, "%"),
                f("Vignetting", "Intensity", 0.0, 0.0, 100.0, "%"),
                p("Style", "Rendering style", Ch(0), &[
                    "Realistic", "Pixelized", "Edge Detect Filter", "Chromatic Aberration", "Kuwahara", "Anisotropic Kuwahara",
                    "Colored Thin Strokes", "Thick Strokes", "High Contrast Strokes", "Colored Thick Strokes",
                ]),
                f("Style", "Style size", 4.0, 1.0, 32.0, "pixels").int().when_not("Rendering style", 0),
                f("Style", "RGB quantization", 0.0, 0.0, 64.0, "levels").int(),
                f("Style", "Hue quantization", 0.0, 0.0, 64.0, "levels").int(),
                f("Style", "Value quantization", 0.0, 0.0, 64.0, "levels").int(),
                f("Style", "Saturation quantization", 0.0, 0.0, 64.0, "levels").int(),
                f("Style", "Quantization dither", 0.0, 0.0, 100.0, "%"),
                p("Style", "Dithering mode", Ch(0), &["Bayer", "Noise", "None"]),
                f("Style", "Translucency limit", 0.0, 0.0, 100.0, "%"),
                f("Style", "Translucency sharpening", 0.0, 0.0, 100.0, "%"),
                p("Style", "Remapping style", Ch(0), &["None", "Grayscale", "Sepia", "Thermal", "Night Vision", "Infrared"]),
            ],
            Kind::Render => vec![
                f("Timeline", "Duration", 64.0, 1.0, 2000.0, "f").int().hide(),
                passes("Render Passes", "Mapping", vec![RPass { src: 0, hdr: true, dst: 0 }, RPass { src: 1, hdr: false, dst: 2 }]),
                p("Export", "Export Mode", Ch(0), &["Flipbook", "Sequence"]).seg(),
                f("Export", "Flipbook width", 2048.0, 64.0, 8192.0, "px").int().when("Export Mode", 0),
                f("Export", "Flipbook height", 2048.0, 64.0, 8192.0, "px").int().when("Export Mode", 0),
                f("Export", "Flipbook columns", 8.0, 1.0, 32.0, "").int().when("Export Mode", 0),
                f("Export", "Flipbook rows", 8.0, 1.0, 32.0, "").int().when("Export Mode", 0),
                ui("Export", "Flipbook layout", "flipbook_grid").when("Export Mode", 0),
                f("Export", "Image width", 1024.0, 32.0, 8192.0, "px").int().when("Export Mode", 1),
                f("Export", "Image height", 1024.0, 32.0, 8192.0, "px").int().when("Export Mode", 1),
                f("Export", "First Frame", 0.0, 0.0, 10000.0, "").int(),
                f("Export", "Number of Frames", 64.0, 1.0, 4096.0, "").int(),
                f("Export", "Frame Stride", 1.0, 1.0, 64.0, "").int(),
                f("Export", "Numbering offset", 0.0, -10000.0, 10000.0, "").int(),
                b("Export", "Absolute Frames", false),
                text("Export", "Directory", ""),
                text("Export", "Filename", "$(project)_$(safename)"),
                p("Export", "File format", Ch(0), &[".png", ".exr", ".tga"]),
                ui("Export", "Export to file", "export_buttons"),
                p("Render Settings", "Alpha Blending Mode", Ch(0), &["Premultiplied Alpha", "Straight Alpha", "Opaque"]),
                b("Render Settings", "Invert Render Alpha", false),
                p("Render Settings", "Shapes Rendering", Ch(0), &["Holdout", "Visible", "Hidden"]),
                p("Render Settings", "Shapes Masking", Ch(0), &["No Masking", "Mask Inside", "Mask Outside"]),
                b("Render Settings", "Invert Shapes Alpha", false),
                b("Render Settings", "Volume Occludes Particles", false),
                b("Render Settings", "Particles Occlude Volume", false),
                b("Render Settings", "Render Volume", true),
                b("Render Settings", "Render Particles", true),
            ],
            Kind::Import => vec![f("Import", "Density scale", 1.0, 0.0, 10.0, ""), f("Import", "Temperature scale", 1.0, 0.0, 10.0, "")],
            Kind::ExportImage => vec![
                f("Output", "Width", 1024.0, 32.0, 8192.0, "px").int(),
                f("Output", "Height", 1024.0, 32.0, 8192.0, "px").int(),
                f("Output", "Frames", 64.0, 1.0, 2000.0, "f").int(),
                f("Output", "Frame step", 1.0, 1.0, 8.0, "f").int(),
            ],
            Kind::ExportFlipbook => vec![
                f("Output", "Tile width", 256.0, 16.0, 2048.0, "px").int(),
                f("Output", "Tile height", 256.0, 16.0, 2048.0, "px").int(),
                f("Output", "Columns", 8.0, 1.0, 16.0, "").int(),
                f("Output", "Frames", 64.0, 1.0, 256.0, "f").int(),
                f("Output", "Frame step", 1.0, 1.0, 8.0, "f").int(),
            ],
            Kind::ExportVolume => vec![b("Output", "Include fuel", true)],
            Kind::ModConstant => vec![f("Value", "Value", 1.0, -1000.0, 1000.0, "")],
            Kind::ModOscillator => vec![
                p("Wave", "Waveform", Ch(0), WAVES),
                f("Wave", "Frequency", 0.5, 0.0, 20.0, "Hz"),
                f("Wave", "Amplitude", 1.0, -1000.0, 1000.0, ""),
                f("Wave", "Offset", 0.0, -1000.0, 1000.0, ""),
                f("Wave", "Phase", 0.0, 0.0, 1.0, ""),
            ],
            Kind::ModCycle => vec![
                f("Cycle", "Start value", 0.0, -1000.0, 1000.0, ""),
                f("Cycle", "End value", 1.0, -1000.0, 1000.0, ""),
                f("Cycle", "Period", 2.0, 0.01, 100.0, "s"),
                b("Cycle", "Repeat", true),
            ],
            Kind::ModMath => vec![p("Math", "Operation", Ch(0), OPS), f("Math", "A (if not wired)", 0.0, -1000.0, 1000.0, ""), f("Math", "B (if not wired)", 1.0, -1000.0, 1000.0, "")],
            Kind::ModTimeShift => vec![f("Time", "Delay", 0.5, -30.0, 30.0, "s"), f("Time", "Speed", 1.0, 0.0, 10.0, "")],
            Kind::ModCombine => vec![f("Combine", "Mix", 0.5, 0.0, 1.0, "")],
            Kind::ModAdsr => vec![
                f("Envelope", "Attack", 0.2, 0.0, 10.0, "s"),
                f("Envelope", "Decay", 0.3, 0.0, 10.0, "s"),
                f("Envelope", "Sustain level", 0.6, 0.0, 1.0, ""),
                f("Envelope", "Hold", 1.0, 0.0, 30.0, "s"),
                f("Envelope", "Release", 0.6, 0.0, 10.0, "s"),
                f("Envelope", "Peak", 1.0, -1000.0, 1000.0, ""),
                f("Envelope", "Repeat every", 0.0, 0.0, 60.0, "s"),
            ],
            Kind::ModMidi => vec![
                f("MIDI", "Device", 0.0, 0.0, 15.0, "").int(),
                f("MIDI", "Channel", 1.0, 1.0, 16.0, "").int(),
                p("MIDI", "Message", Ch(0), MIDI),
                f("MIDI", "Number", 1.0, 0.0, 127.0, "").int(),
                f("MIDI", "Min", 0.0, -1000.0, 1000.0, ""),
                f("MIDI", "Max", 1.0, -1000.0, 1000.0, ""),
            ],
            Kind::Comment | Kind::Note => vec![],
        }
    }
}

/// One colour stop of a gradient (position 0..1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GStop {
    pub pos: f32,
    pub col: [f32; 3],
}

/// One render pass of the Render node: what to render (`src`) and how to store it (`dst`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RPass {
    pub src: usize,
    pub hdr: bool,
    pub dst: usize,
}

pub const PASS_SOURCES: [&str; 8] = ["Render All", "Alpha", "Emission", "Scattering", "Temperature", "Density", "Fuel", "Depth"];
pub const PASS_TARGETS: [&str; 3] = ["RGBA", "RGB", "R"];

#[derive(Clone, PartialEq, Debug)]
pub enum Val {
    F(f32),
    B(bool),
    C([f32; 3]),
    V([f32; 3]),
    Ch(usize),
    /// min / max
    R([f32; 2]),
    G(Vec<GStop>),
    S(String),
    P(Vec<RPass>),
    /// a display-only row (histogram, preview, buttons…), drawn by `special_row`
    Ui(&'static str),
}

#[derive(Clone)]
pub struct Param {
    pub name: &'static str,
    pub section: &'static str,
    pub unit: &'static str,
    pub val: Val,
    pub def: Val,
    pub range: (f32, f32),
    pub choices: &'static [&'static str],
    pub fav: bool,
    pub rand: bool,
    /// Modulator node (id) driving this setting, and which axis for X/Y/Z settings (3 = all).
    pub driver: Option<u64>,
    pub axis: u8,
    /// Name shown in the panel (several settings share a label, e.g. two "Coloring remap range").
    pub label: &'static str,
    /// Only shown when another setting of the node has this choice index (bool: 0/1); `neg` inverts.
    pub show_if: Option<(&'static str, usize, bool)>,
    /// Only shown when the node's "Advanced parameters" is on.
    pub adv: bool,
    /// Decimal places in the panel (0 = whole numbers).
    pub dec: u8,
    /// Not shown in the node's settings (edited elsewhere, e.g. the timeline).
    pub hidden: bool,
    /// Two-option choice drawn as a pair of buttons.
    pub seg: bool,
}

impl Param {
    fn lab(mut self, l: &'static str) -> Self {
        self.label = l;
        self
    }
    fn when(mut self, n: &'static str, v: usize) -> Self {
        self.show_if = Some((n, v, false));
        self
    }
    fn when_not(mut self, n: &'static str, v: usize) -> Self {
        self.show_if = Some((n, v, true));
        self
    }
    fn adv(mut self) -> Self {
        self.adv = true;
        self
    }
    fn int(mut self) -> Self {
        self.dec = 0;
        self
    }
    fn hide(mut self) -> Self {
        self.hidden = true;
        self
    }
    fn seg(mut self) -> Self {
        self.seg = true;
        self
    }
}

fn mk(section: &'static str, name: &'static str, val: Val, range: (f32, f32), unit: &'static str) -> Param {
    Param {
        name,
        section,
        unit,
        def: val.clone(),
        val,
        range,
        choices: &[],
        fav: false,
        rand: false,
        driver: None,
        axis: 3,
        label: name,
        show_if: None,
        adv: false,
        dec: 3,
        hidden: false,
        seg: false,
    }
}
fn r(s: &'static str, n: &'static str, v: [f32; 2], range: (f32, f32), unit: &'static str) -> Param {
    mk(s, n, Val::R(v), range, unit)
}
fn g(s: &'static str, n: &'static str, stops: Vec<GStop>) -> Param {
    mk(s, n, Val::G(stops), (0.0, 1.0), "")
}
fn text(s: &'static str, n: &'static str, v: &str) -> Param {
    mk(s, n, Val::S(v.into()), (0.0, 1.0), "")
}
fn ui(s: &'static str, n: &'static str, key: &'static str) -> Param {
    mk(s, n, Val::Ui(key), (0.0, 1.0), "")
}
fn passes(s: &'static str, n: &'static str, v: Vec<RPass>) -> Param {
    mk(s, n, Val::P(v), (0.0, 1.0), "")
}
fn f(s: &'static str, n: &'static str, v: f32, lo: f32, hi: f32, unit: &'static str) -> Param {
    mk(s, n, Val::F(v), (lo, hi), unit)
}
fn b(s: &'static str, n: &'static str, v: bool) -> Param {
    mk(s, n, Val::B(v), (0.0, 1.0), "")
}
fn c(s: &'static str, n: &'static str, v: [f32; 3]) -> Param {
    mk(s, n, Val::C(v), (0.0, 1.0), "")
}
fn v(s: &'static str, n: &'static str, v: [f32; 3], lo: f32, hi: f32, unit: &'static str) -> Param {
    mk(s, n, Val::V(v), (lo, hi), unit)
}
fn p(s: &'static str, n: &'static str, v: Val, choices: &'static [&'static str]) -> Param {
    Param { choices, ..mk(s, n, v, (0.0, 1.0), "") }
}

#[derive(Clone)]
pub struct Node {
    pub id: u64,
    pub kind: Kind,
    pub pos: Pos2,
    pub enabled: bool,
    pub params: Vec<Param>,
    pub text: String,
    /// User name for the node (empty = the node type's name).
    pub title: String,
}

impl Node {
    pub fn title(&self) -> &str {
        if self.title.is_empty() { self.kind.name() } else { &self.title }
    }
    fn get(&self, name: &str) -> Option<&Val> {
        self.params.iter().find(|p| p.name == name).map(|p| &p.val)
    }
    pub fn f(&self, name: &str) -> f32 {
        match self.get(name) { Some(Val::F(v)) => *v, _ => 0.0 }
    }
    pub fn b(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Val::B(true)))
    }
    pub fn c(&self, name: &str) -> [f32; 3] {
        match self.get(name) { Some(Val::C(v)) | Some(Val::V(v)) => *v, _ => [0.0; 3] }
    }
    pub fn ch(&self, name: &str) -> usize {
        match self.get(name) { Some(Val::Ch(v)) => *v, _ => 0 }
    }
    pub fn r(&self, name: &str) -> [f32; 2] {
        match self.get(name) { Some(Val::R(v)) => *v, _ => [0.0, 1.0] }
    }
    pub fn grad(&self, name: &str) -> Vec<GStop> {
        match self.get(name) { Some(Val::G(v)) => v.clone(), _ => vec![] }
    }
    pub fn s(&self, name: &str) -> String {
        match self.get(name) { Some(Val::S(v)) => v.clone(), _ => String::new() }
    }
    pub fn passes(&self, name: &str) -> Vec<RPass> {
        match self.get(name) { Some(Val::P(v)) => v.clone(), _ => vec![] }
    }
    /// Set a setting given in 0.4 units: the few settings that kept their name but changed
    /// units are converted (see `UNIT_CHANGED`); everything else is a plain `set`.
    pub fn set_old(&mut self, name: &str, val: Val) {
        if UNIT_CHANGED.contains(&(self.kind, name)) {
            legacy_set(self, name, &val);
        } else {
            self.set(name, val);
        }
    }
    /// Set a setting by name; names from older versions are translated (see `legacy_set`).
    pub fn set(&mut self, name: &str, val: Val) {
        if let Some(p) = self.params.iter_mut().find(|p| p.name == name) {
            p.val = val;
        } else {
            let known = legacy_set(self, name, &val);
            debug_assert!(known, "{} has no setting '{name}'", self.kind.name());
        }
    }
    /// Is this setting shown in the panel right now?
    pub fn visible(&self, p: &Param) -> bool {
        if p.hidden || (p.adv && !self.b("Advanced parameters")) {
            return false;
        }
        match p.show_if {
            Some((n, v, neg)) => {
                let cur = match self.get(n) {
                    Some(Val::Ch(i)) => *i,
                    Some(Val::B(b)) => *b as usize,
                    _ => v,
                };
                (cur == v) != neg
            }
            None => true,
        }
    }
    /// Domain width in metres (Simulation node).
    pub fn domain_width(&self) -> f32 {
        self.f("Domain size")
    }
    pub fn size(&self) -> Vec2 {
        if self.kind == Kind::Comment {
            return vec2(178.0, 44.0);
        }
        if self.kind == Kind::Note {
            return vec2(170.0, 96.0);
        }
        let (i, o) = self.kind.ports();
        vec2(122.0, 24.0 + i.len().max(o.len()) as f32 * 17.5 + 4.0)
    }
    fn port_pos(&self, output: bool, idx: usize) -> Pos2 {
        let x = if output { self.pos.x + self.size().x - 8.0 } else { self.pos.x + 8.0 };
        pos2(x, self.pos.y + 24.0 + idx as f32 * 17.5 + 8.0)
    }
}

/// Grid sizes of the old Low / Medium / High / Ultra choice, in UI axes (X, Y, Z-up).

/// Settings that were renamed or re-expressed in newer versions: map them onto today's settings.
/// Returns false if the name is unknown. Used by presets and when opening older projects.
/// Settings that kept their 0.4 name but count in new units (0.4 used plain multipliers).
/// Values from 0.4 projects and the presets go through `legacy_set` to be converted.
pub const UNIT_CHANGED: [(Kind, &str); 1] = [(Kind::ForceNoise, "Scale")];

/// Force: Noise Amplitude per unit of 0.4's "Strength" (Emission Chaos / every other noise force).
/// Chosen so the defaults (50 and 19.333) give back 0.4's 0.3 and 2.0 exactly.
pub const NOISE_EMIT: f64 = 500.0 / 3.0;
pub const NOISE_FIRE: f64 = 9.6665;

pub fn legacy_set(n: &mut Node, name: &str, val: &Val) -> bool {
    let fv = match val { Val::F(x) => *x, Val::Ch(i) => *i as f32, _ => 0.0 };
    let put = |n: &mut Node, k: &str, v: Val| {
        if let Some(p) = n.params.iter_mut().find(|p| p.name == k) {
            p.val = v;
        }
    };
    match (n.kind, name) {
        (Kind::Shading, "Absorption") => put(n, "Smoke density scale", Val::F(fv / 3.0 * 100.0)),
        (Kind::Shading, "Shadow strength") => put(n, "Smoke shadow density", Val::F(fv * 100.0)),
        (Kind::Shading, "Fine detail") => {}
        (Kind::FireGradient, "Cold color" | "Mid color" | "Hot color") => {
            let Val::C(c) = val else { return true };
            let want = match name { "Cold color" => 0.0, "Mid color" => 0.5, _ => 1.0 };
            let mut stops = n.grad("Color gradient");
            if let Some(s) = stops.iter_mut().min_by(|a, b| (a.pos - want).abs().total_cmp(&(b.pos - want).abs())) {
                s.col = *c;
            }
            put(n, "Color gradient", Val::G(stops));
        }
        (Kind::FireGradient | Kind::ColorConstant | Kind::ColorBlackbody, "Min temperature" | "Max temperature" | "Brightness") => {}
        (Kind::ForceNoise, "Strength") => {
            let emission = n.title == "Emission Chaos";
            put(n, "Amplitude", Val::F((fv as f64 * if emission { NOISE_EMIT } else { NOISE_FIRE }) as f32));
        }
        (Kind::ForceNoise, "Scale") => {
            let emission = n.title == "Emission Chaos";
            put(n, "Scale", Val::F(if emission { 8.0 } else { 25.0 } / fv.max(0.01)));
        }
        (Kind::ForceNoise, "Speed") => put(n, "Animation speed", Val::F(fv * 2.0)),
        (Kind::Emitter, "Expansion") => put(n, "Additional pressure rate", Val::F(fv * 100.0)),
        (Kind::Emitter, "Show shape") => {}
        (Kind::Emitter, "Shape color") => put(n, "Emitter color", val.clone()),
        (Kind::Render, "Width") => put(n, "Image width", val.clone()),
        (Kind::Render, "Height") => put(n, "Image height", val.clone()),
        (Kind::Render, "Flipbook columns") => {
            put(n, "Flipbook columns", val.clone());
            put(n, "Flipbook rows", val.clone());
        }
        (Kind::Render, "Frame step") => put(n, "Frame Stride", val.clone()),
        (Kind::VolumeProcessing, "Smoke density" | "Temperature multiplier") => {}
        _ => return false,
    }
    true
}

#[derive(Clone, Copy, PartialEq)]
pub struct Wire {
    pub from: usize,
    pub out: usize,
    pub to: usize,
    pub inp: usize,
}

fn port_type(name: &str) -> &str {
    match name {
        "Shapes" | "Mask Shapes" | "Shape" | "Shape A" | "Shape B" => "shape",
        "Colliders" | "Collider" => "collider",
        "A" | "B" | "Value" => "value",
        "Forces" | "Force" => "force",
        "Emitters" | "Emitter" => "emitter",
        "Lights" | "Light" | "Sun" => "light",
        "Fire color" | "Color" => "color",
        other => other,
    }
}

fn is_multi(name: &str) -> bool {
    matches!(name, "Shapes" | "Mask Shapes" | "Colliders" | "Forces" | "Emitters" | "Lights")
}

/// Edit commands requested from menus inside the graph; the app runs them (it owns undo).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
}

#[derive(Clone)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    /// Node shown in the settings panel.
    pub selected: Option<usize>,
    /// All selected nodes (includes `selected`).
    pub picked: Vec<usize>,
    pub cmd: Option<Cmd>,
    /// Graph position for pasting (last right-click), if any.
    pub paste_at: Option<Pos2>,
    box_start: Option<Pos2>,
    pan: Vec2,
    zoom: f32,
    fitted: bool,
    drag_wire: Option<(usize, usize)>,
    menu_pos: Pos2,
    next_id: u64,
    pub details_tab: usize,
    pub active_section: &'static str,
    rng: u32,
    /// Export node whose "Export now" was pressed (the app runs it).
    pub export_request: Option<usize>,
    /// Import settings changed: the app restarts the simulation to load the file.
    pub import_request: bool,
    menu_search: String,
    rename_focus: bool,
}

impl Default for Graph {
    fn default() -> Self {
        let mut g = Graph {
            nodes: vec![],
            wires: vec![],
            selected: None,
            picked: vec![],
            cmd: None,
            paste_at: None,
            box_start: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
            drag_wire: None,
            menu_pos: Pos2::ZERO,
            next_id: 1,
            details_tab: 0,
            active_section: "",
            rng: 0x9e37_79b9,
            export_request: None,
            import_request: false,
            menu_search: String::new(),
            rename_focus: false,
        };
        // layout mirrors the reference picture
        use Kind::*;
        let n = [
            (ShapePrimitive, 40.0, 232.0, true),
            (ForceNoise, 40.0, 287.0, true),
            (ForceNoise, 170.0, 195.0, true),
            (Emitter, 170.0, 250.0, true),
            (Simulation, 320.0, 213.0, true),
            (VolumeProcessing, 462.0, 273.0, true),
            (LightAmbient, 315.0, 38.0, true),
            (FireGradient, 320.0, 103.0, true),
            (LightDirectional, 320.0, 158.0, true),
            (Skybox, 462.0, 163.0, false),
            (Ground, 462.0, 218.0, false),
            (Shading, 462.0, 89.0, true),
            (Camera, 605.0, 101.0, true),
            (Scene, 605.0, 156.0, true),
            (Render, 730.0, 101.0, true),
        ];
        for (k, x, y, on) in n {
            let i = g.add(k, pos2(x, y));
            g.nodes[i].enabled = on;
        }
        g.nodes[0].title = "Fire Shape".into();
        g.nodes[1].title = "Emission Chaos".into();
        g.nodes[1].set("Amplitude", Val::F(50.0));
        g.nodes[1].set("Scale", Val::F(8.0));
        g.nodes[1].set("Seed", Val::F(267.0));
        g.nodes[1].set("Octaves", Val::F(1.0));
        g.nodes[1].set("Mode", Val::Ch(2));
        g.nodes[1].set("Constant mask", Val::F(100.0));
        g.nodes[1].set("Temperature mask", Val::F(0.0));
        g.nodes[2].title = "Fire Chaos".into();
        g.nodes[7].title = "Fire Color Gradient".into();
        g.nodes[12].title = "Camera".into();
        let c = g.add(Comment, pos2(106.0, 127.0));
        g.nodes[c].text = "Use CTRL + 3 for HQ Render Mode".into();
        let w = |from, out, to, inp| Wire { from, out, to, inp };
        g.wires = vec![
            w(0, 0, 3, 0),  // shape -> emitter
            w(1, 0, 3, 1),  // emission chaos -> emitter forces
            w(2, 0, 4, 0),  // fire chaos -> sim forces
            w(3, 0, 4, 1),  // emitter -> sim
            w(4, 0, 5, 0),  // sim -> processing
            w(6, 0, 11, 0), // ambient -> shading
            w(8, 0, 11, 0), // directional -> shading
            w(7, 0, 11, 1), // gradient -> shading
            w(8, 0, 9, 0),  // sun -> skybox
            w(11, 0, 13, 0),
            w(9, 0, 13, 1),
            w(10, 0, 13, 2),
            w(5, 0, 13, 3),
            w(4, 1, 13, 4),
            w(12, 0, 14, 0),
            w(13, 0, 14, 1),
        ];
        g.selected = Some(3);
        g.picked = vec![3];
        g
    }
}

/// A node with default settings, for values when nothing is wired in.
pub fn template(kind: Kind) -> Node {
    Node { id: 0, kind, pos: Pos2::ZERO, enabled: true, params: kind.params(), text: String::new(), title: String::new() }
}

impl Graph {
    pub fn add(&mut self, kind: Kind, pos: Pos2) -> usize {
        let text = match kind {
            Kind::Comment => "Comment",
            Kind::Note => "Note",
            _ => "",
        };
        self.nodes.push(Node { id: self.next_id, kind, pos, enabled: true, params: kind.params(), text: text.into(), title: String::new() });
        self.next_id += 1;
        self.nodes.len() - 1
    }

    pub fn delete(&mut self, i: usize) {
        self.nodes.remove(i);
        self.wires.retain(|w| w.from != i && w.to != i);
        for w in &mut self.wires {
            if w.from > i { w.from -= 1 }
            if w.to > i { w.to -= 1 }
        }
        self.selected = None;
        self.picked.retain(|&p| p != i);
        for p in &mut self.picked {
            if *p > i { *p -= 1 }
        }
    }

    pub fn duplicate(&mut self, i: usize) {
        self.pick(i, false);
        self.duplicate_selection();
    }

    pub fn is_picked(&self, i: usize) -> bool {
        self.picked.contains(&i)
    }

    /// Select node `i`; `add` toggles it in a multi-selection (Ctrl+click).
    pub fn pick(&mut self, i: usize, add: bool) {
        if add {
            if let Some(k) = self.picked.iter().position(|&p| p == i) {
                self.picked.remove(k);
                self.selected = self.picked.last().copied();
                return;
            }
            self.picked.push(i);
        } else if !self.is_picked(i) {
            self.picked = vec![i];
        }
        self.selected = Some(i);
    }

    pub fn select_all(&mut self) {
        self.picked = (0..self.nodes.len()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.picked.clear();
        self.selected = None;
    }

    pub fn delete_selection(&mut self) {
        let mut ids = self.picked.clone();
        if ids.is_empty() {
            ids.extend(self.selected);
        }
        ids.sort_unstable();
        ids.dedup();
        for i in ids.into_iter().rev() {
            if i < self.nodes.len() {
                self.delete(i);
            }
        }
        self.clear_selection();
    }

    /// Selected nodes and the wires between them, as project text (for the clipboard).
    pub fn copy_selection(&self) -> Option<String> {
        let mut ids = self.picked.clone();
        if ids.is_empty() {
            ids.extend(self.selected);
        }
        ids.retain(|&i| i < self.nodes.len());
        if ids.is_empty() {
            return None;
        }
        let nodes: Vec<Node> = ids.iter().map(|&i| self.nodes[i].clone()).collect();
        let wires: Vec<Wire> = self
            .wires
            .iter()
            .filter_map(|w| {
                let f = ids.iter().position(|&i| i == w.from)?;
                let t = ids.iter().position(|&i| i == w.to)?;
                Some(Wire { from: f, out: w.out, to: t, inp: w.inp })
            })
            .collect();
        Some(save_parts(&nodes, &wires))
    }

    /// Add nodes from project text next to where they came from (or at `paste_at`) and select them.
    pub fn paste(&mut self, text: &str) -> bool {
        let Ok(src) = Graph::load(text) else { return false };
        if src.nodes.is_empty() {
            return false;
        }
        let min = src.nodes.iter().fold(pos2(f32::MAX, f32::MAX), |m, n| pos2(m.x.min(n.pos.x), m.y.min(n.pos.y)));
        let offset = match self.paste_at.take() {
            Some(at) => at - min,
            None => vec2(30.0, 30.0),
        };
        let base = self.nodes.len();
        for mut n in src.nodes {
            n.id = self.next_id;
            self.next_id += 1;
            n.pos += offset;
            self.nodes.push(n);
        }
        for w in src.wires {
            self.wires.push(Wire { from: w.from + base, out: w.out, to: w.to + base, inp: w.inp });
        }
        self.picked = (base..self.nodes.len()).collect();
        self.selected = Some(base);
        true
    }

    pub fn duplicate_selection(&mut self) {
        if let Some(t) = self.copy_selection() {
            self.paste(&t);
        }
    }

    /// Replace nodes and wires with a saved state (undo/redo), keeping the view.
    pub fn restore(&mut self, text: &str) {
        if let Ok(src) = Graph::load(text) {
            self.nodes = src.nodes;
            for n in &mut self.nodes {
                n.id = self.next_id;
                self.next_id += 1;
            }
            self.wires = src.wires;
            let len = self.nodes.len();
            self.picked.retain(|&i| i < len);
            self.selected = self.selected.filter(|&i| i < len);
        }
    }

    /// Wire output `out` of node `from` into input `inp` of node `to` (by port names).
    pub fn connect(&mut self, from: usize, out: &str, to: usize, inp: &str) {
        let o = self.nodes[from].kind.ports().1.iter().position(|p| *p == out);
        let i = self.nodes[to].kind.ports().0.iter().position(|p| *p == inp);
        if let (Some(o), Some(i)) = (o, i) {
            if !is_multi(inp) {
                self.wires.retain(|w| !(w.to == to && w.inp == i));
            }
            self.wires.push(Wire { from, out: o, to, inp: i });
        }
    }

    /// Remove every wire into input `inp` of node `to`.
    pub fn unplug(&mut self, to: usize, inp: &str) {
        if let Some(i) = self.nodes[to].kind.ports().0.iter().position(|p| *p == inp) {
            self.wires.retain(|w| !(w.to == to && w.inp == i));
        }
    }

    /// Nodes wired into input `port` of node `i`, enabled ones only (ignores whether they reach Render).
    pub fn sources(&self, i: usize, port: &str) -> Vec<usize> {
        let Some(idx) = self.nodes[i].kind.ports().0.iter().position(|p| *p == port) else { return vec![] };
        self.wires.iter().filter(|w| w.to == i && w.inp == idx && self.nodes[w.from].enabled).map(|w| w.from).collect()
    }

    pub fn first(&self, kind: Kind) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == kind)
    }

    pub fn node_mut(&mut self, kind: Kind) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.kind == kind)
    }

    /// Nodes that feed a Render node (directly or through others) and are switched on.
    pub fn live(&self) -> Vec<bool> {
        let mut reached = vec![false; self.nodes.len()];
        let mut stack: Vec<usize> = (0..self.nodes.len()).filter(|&i| self.nodes[i].kind == Kind::Render).collect();
        while let Some(i) = stack.pop() {
            if reached[i] {
                continue;
            }
            reached[i] = true;
            stack.extend(self.wires.iter().filter(|w| w.to == i).map(|w| w.from));
        }
        reached.iter().enumerate().map(|(i, r)| *r && self.nodes[i].enabled).collect()
    }

    /// Live nodes wired into input `port` of node `i`.
    pub fn inputs(&self, live: &[bool], i: usize, port: &str) -> Vec<usize> {
        let Some(idx) = self.nodes[i].kind.ports().0.iter().position(|p| *p == port) else { return vec![] };
        self.wires.iter().filter(|w| w.to == i && w.inp == idx && live[w.from]).map(|w| w.from).collect()
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    pub fn randomize(&mut self) {
        let mut nodes = std::mem::take(&mut self.nodes);
        for p in nodes.iter_mut().flat_map(|n| n.params.iter_mut()).filter(|p| p.rand) {
            let (lo, hi) = p.range;
            p.val = match &p.val {
                Val::F(_) => Val::F(lo + (hi - lo) * self.rand()),
                Val::B(_) => Val::B(self.rand() > 0.5),
                Val::C(_) => Val::C([self.rand(), self.rand(), self.rand()]),
                Val::V(_) => Val::V([lo + (hi - lo) * self.rand(), lo + (hi - lo) * self.rand(), lo + (hi - lo) * self.rand()]),
                Val::Ch(_) => Val::Ch(((self.rand() * p.choices.len() as f32) as usize).min(p.choices.len() - 1)),
                Val::R(_) => {
                    let (a, b) = (lo + (hi - lo) * self.rand(), lo + (hi - lo) * self.rand());
                    Val::R([a.min(b), a.max(b)])
                }
                other => other.clone(),
            };
        }
        self.nodes = nodes;
    }

    // ---------- project files ----------

    pub fn save(&self) -> String {
        save_parts(&self.nodes, &self.wires)
    }

    pub fn load(text: &str) -> Result<Graph, String> {
        let mut g = Graph::default();
        g.nodes.clear();
        g.wires.clear();
        g.selected = None;
        let mut drivers: Vec<(usize, &'static str)> = vec![];
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("");
        if !header.starts_with("firecraft-project") {
            return Err("not a FireCraft project".into());
        }
        // 0.4 projects: version 1 without the new noise settings
        let old_units = header.trim() == "firecraft-project 1" && !text.contains("\tAmplitude\t");
        for line in lines {
            let c: Vec<&str> = line.split('\t').collect();
            let num = |i: usize| c.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
            match c[0] {
                "node" => {
                    let (kind, legacy_title) = Kind::from_saved(c.get(1).copied().unwrap_or("")).ok_or("unknown node type")?;
                    let i = g.add(kind, pos2(num(2), num(3)));
                    g.nodes[i].enabled = num(4) != 0.0;
                    g.nodes[i].title = legacy_title.into();
                }
                "title" => {
                    if let Some(n) = g.nodes.last_mut() {
                        n.title = c.get(1).unwrap_or(&"").to_string();
                    }
                }
                "text" => {
                    if let Some(n) = g.nodes.last_mut() {
                        n.text = c.get(1).unwrap_or(&"").replace("\\n", "\n");
                    }
                }
                "p" => {
                    let ni = g.nodes.len().saturating_sub(1);
                    let Some(n) = g.nodes.last_mut() else { continue };
                    let pname = c.get(1).copied().unwrap_or("");
                    let raw = c.get(2).copied().unwrap_or("");
                    let nums: Vec<f32> = raw.split(' ').filter_map(|s| s.parse().ok()).collect();
                    let a3 = |d: &[f32]| [d.first().copied().unwrap_or(0.0), d.get(1).copied().unwrap_or(0.0), d.get(2).copied().unwrap_or(0.0)];
                    if old_units && UNIT_CHANGED.contains(&(n.kind, pname)) {
                        legacy_set(n, pname, &Val::F(nums.first().copied().unwrap_or(0.0)));
                        continue;
                    }
                    let Some(p) = n.params.iter_mut().find(|p| p.name == pname) else {
                        // a setting from an older version
                        let v = if nums.len() >= 3 { Val::V(a3(&nums)) } else { Val::F(nums.first().copied().unwrap_or(0.0)) };
                        legacy_set(n, pname, &v);
                        continue;
                    };
                    p.val = match &p.val {
                        Val::F(_) => Val::F(nums.first().copied().unwrap_or(0.0)),
                        Val::B(_) => Val::B(nums.first().copied().unwrap_or(0.0) != 0.0),
                        Val::C(_) => Val::C(a3(&nums)),
                        Val::V(_) => Val::V(a3(&nums)),
                        Val::Ch(_) => Val::Ch(nums.first().copied().unwrap_or(0.0) as usize),
                        Val::R(_) => Val::R([nums.first().copied().unwrap_or(0.0), nums.get(1).copied().unwrap_or(1.0)]),
                        Val::G(_) => Val::G(
                            raw.split(';')
                                .filter_map(|s| {
                                    let v: Vec<f32> = s.split(' ').filter_map(|x| x.parse().ok()).collect();
                                    (v.len() == 4).then(|| GStop { pos: v[0], col: [v[1], v[2], v[3]] })
                                })
                                .collect(),
                        ),
                        Val::S(_) => Val::S(raw.to_string()),
                        Val::P(_) => Val::P(
                            raw.split(';')
                                .filter_map(|s| {
                                    let v: Vec<usize> = s.split(' ').filter_map(|x| x.parse().ok()).collect();
                                    (v.len() == 3).then(|| RPass { src: v[0].min(PASS_SOURCES.len() - 1), hdr: v[1] != 0, dst: v[2].min(PASS_TARGETS.len() - 1) })
                                })
                                .collect(),
                        ),
                        Val::Ui(k) => Val::Ui(*k),
                    };
                    p.fav = num(3) != 0.0;
                    p.rand = num(4) != 0.0;
                    // driver saved as a node index; resolved to an id once all nodes exist
                    if let Some(d) = c.get(5).and_then(|s| s.parse::<u64>().ok()) {
                        p.driver = Some(d);
                        p.axis = num(6) as u8;
                        drivers.push((ni, p.name));
                    }
                }
                "wire" => {
                    let w = Wire { from: num(1) as usize, out: num(2) as usize, to: num(3) as usize, inp: num(4) as usize };
                    if w.from < g.nodes.len() && w.to < g.nodes.len() {
                        g.wires.push(w);
                    }
                }
                _ => {}
            }
        }
        for (ni, name) in drivers {
            let ids: Vec<u64> = g.nodes.iter().map(|n| n.id).collect();
            if let Some(p) = g.nodes[ni].params.iter_mut().find(|p| p.name == name) {
                p.driver = p.driver.and_then(|d| ids.get(d as usize).copied());
            }
        }
        Ok(g)
    }

    // ---------- canvas ----------

    pub fn canvas(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let bg = ui.interact(rect, ui.id().with("graph-bg"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, GRAPH_BG);
        if !self.fitted {
            self.zoom = (rect.width() / 860.0).clamp(0.55, 1.4);
            self.pan = vec2(-20.0 * self.zoom, 0.0);
            self.fitted = true;
        }
        let pointer = ui.ctx().pointer_latest_pos();
        // zoom around the cursor
        if let Some(m) = pointer.filter(|m| rect.contains(*m)) {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old = self.zoom;
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.3, 2.5);
                let rel = m - rect.min - self.pan;
                self.pan = m - rect.min - rel * (self.zoom / old);
            }
        }
        let shift = ui.input(|i| i.modifiers.shift);
        if bg.drag_started() && shift {
            self.box_start = pointer;
        }
        if bg.dragged() && self.box_start.is_none() {
            self.pan += bg.drag_delta();
        }
        if bg.clicked() {
            self.clear_selection();
        }
        let (zoom, pan, origin) = (self.zoom, self.pan, rect.min);
        let to_screen = move |p: Pos2| origin + pan + p.to_vec2() * zoom;
        let to_graph = move |s: Pos2| ((s - origin - pan) / zoom).to_pos2();

        if bg.secondary_clicked() {
            self.menu_pos = to_graph(pointer.unwrap_or(rect.center()));
        }
        bg.context_menu(|ui| {
            ui.set_min_width(190.0);
            if ui.add(egui::Button::new("Paste").shortcut_text("Ctrl+V")).clicked() {
                self.paste_at = Some(self.menu_pos);
                self.cmd = Some(Cmd::Paste);
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Select All").shortcut_text("Ctrl+A")).clicked() {
                self.cmd = Some(Cmd::SelectAll);
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Create Comment").shortcut_text("Ctrl+G")).clicked() {
                let i = self.add(Kind::Comment, self.menu_pos);
                self.pick(i, false);
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Create Note").shortcut_text("H")).clicked() {
                let i = self.add(Kind::Note, self.menu_pos);
                self.pick(i, false);
                ui.close_menu();
            }
            for (label, key, cmd) in [("Undo", "Ctrl+Z", Cmd::Undo), ("Redo", "Ctrl+Y", Cmd::Redo)] {
                if ui.add(egui::Button::new(label).shortcut_text(key)).clicked() {
                    self.cmd = Some(cmd);
                    ui.close_menu();
                }
            }
            ui.separator();
            let search = ui.add(egui::TextEdit::singleline(&mut self.menu_search).hint_text("🔍 Search").desired_width(176.0));
            search.request_focus();
            let q = self.menu_search.to_lowercase();
            let mut add = None;
            let exists: Vec<Kind> = self.nodes.iter().map(|n| n.kind).collect();
            let item = |ui: &mut Ui, k: Kind, add: &mut Option<Kind>| {
                let taken = k.singleton() && exists.contains(&k);
                let label = egui::RichText::new(format!("■  {}", k.name()));
                let resp = ui.add_enabled(!taken, egui::Button::new(label.color(if taken { DIM } else { TEXT })).frame(false));
                // colour the little square like the node icon
                if !taken {
                    let r = resp.rect;
                    ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x + 4.0, r.center().y - 4.0), vec2(8.0, 8.0)), CornerRadius::same(2), k.color());
                }
                if resp.clicked() {
                    *add = Some(k);
                }
            };
            if !q.is_empty() {
                for k in ALL.iter().filter(|k| !k.is_note() && k.name().to_lowercase().contains(&q)) {
                    item(ui, *k, &mut add);
                }
            } else {
                for gname in GROUPS {
                    let members: Vec<Kind> = ALL.iter().copied().filter(|k| k.group() == Some(gname)).collect();
                    if members.len() == 1 {
                        item(ui, members[0], &mut add);
                    } else {
                        ui.menu_button(gname, |ui| {
                            for k in &members {
                                item(ui, *k, &mut add);
                            }
                        });
                    }
                }
                for k in ALL.iter().filter(|k| k.group().is_none() && !k.is_note()) {
                    item(ui, *k, &mut add);
                }
            }
            if let Some(k) = add {
                let i = self.add(k, self.menu_pos);
                self.pick(i, false);
                self.menu_search.clear();
                ui.close_menu();
            }
        });

        // dot grid
        let step = 20.0 * zoom;
        if step > 6.0 {
            let start = rect.min + vec2(pan.x.rem_euclid(step), pan.y.rem_euclid(step));
            let mut y = start.y;
            while y < rect.max.y {
                let mut x = start.x;
                while x < rect.max.x {
                    painter.circle_filled(pos2(x, y), 0.8, Color32::from_rgb(55, 58, 60));
                    x += step;
                }
                y += step;
            }
        }

        let live = self.live();
        // wires
        for w in &self.wires {
            let a = to_screen(self.nodes[w.from].port_pos(true, w.out));
            let b = to_screen(self.nodes[w.to].port_pos(false, w.inp));
            let col = if live[w.from] { Color32::from_rgb(125, 128, 130) } else { Color32::from_rgb(70, 72, 74) };
            draw_wire(&painter, a, b, zoom, col);
        }

        let font = |s: f32| FontId::proportional(s * zoom);
        let mut action: Option<(usize, &str)> = None;
        let mut start_wire = None;
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            let r = Rect::from_min_size(to_screen(n.pos), n.size() * zoom);
            let resp = ui.interact(r, egui::Id::new(("node", n.id)), Sense::click_and_drag());
            let ctrl = ui.input(|inp| inp.modifiers.command);
            if resp.clicked() {
                self.pick(i, ctrl);
            } else if resp.drag_started() || resp.secondary_clicked() {
                self.pick(i, false);
            }
            if resp.dragged() {
                let d = resp.drag_delta() / zoom;
                for &k in &self.picked {
                    if let Some(n) = self.nodes.get_mut(k) {
                        n.pos += d;
                    }
                }
                if !self.is_picked(i) {
                    self.nodes[i].pos += d;
                }
            }
            resp.context_menu(|ui| {
                for (label, key, cmd) in [
                    ("Cut", "Ctrl+X", Cmd::Cut),
                    ("Copy", "Ctrl+C", Cmd::Copy),
                    ("Paste", "Ctrl+V", Cmd::Paste),
                    ("Duplicate", "Ctrl+D", Cmd::Duplicate),
                    ("Delete", "Del", Cmd::Delete),
                ] {
                    if ui.add(egui::Button::new(label).shortcut_text(key)).clicked() {
                        self.cmd = Some(cmd);
                        ui.close_menu();
                    }
                }
                ui.separator();
                for a in ["Rename", "Turn on/off", "Reset settings"] {
                    if ui.button(a).clicked() {
                        action = Some((i, a));
                        ui.close_menu();
                    }
                }
            });
            let n = &self.nodes[i];
            let sel = self.is_picked(i) || self.selected == Some(i);
            let r = Rect::from_min_size(to_screen(n.pos), n.size() * zoom);
            if n.kind == Kind::Note {
                painter.rect_filled(r, CornerRadius::same(3), Color32::from_rgb(78, 70, 40));
                if sel {
                    painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
                }
                let mut text = self.nodes[i].text.clone();
                let te = ui.put(
                    r.shrink(6.0 * zoom),
                    egui::TextEdit::multiline(&mut text).frame(false).font(font(9.5)).text_color(Color32::from_rgb(240, 230, 190)),
                );
                if te.changed() {
                    self.nodes[i].text = text;
                }
                continue;
            }
            if n.kind == Kind::Comment {
                painter.rect_filled(r, CornerRadius::same(3), Color32::from_rgb(44, 47, 49));
                let head = Rect::from_min_size(r.min, vec2(r.width(), 18.0 * zoom));
                painter.rect_filled(head, CornerRadius::same(3), Color32::from_rgb(54, 57, 59));
                if sel {
                    painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
                }
                let mut text = self.nodes[i].text.clone();
                let te = ui.put(
                    head.shrink2(vec2(5.0 * zoom, 1.0)),
                    egui::TextEdit::singleline(&mut text).frame(false).font(font(9.5)).text_color(TEXT),
                );
                if te.changed() {
                    self.nodes[i].text = text;
                }
                continue;
            }
            let txt = if n.enabled && live[i] { TEXT } else { DIM };
            painter.rect_filled(r, CornerRadius::same(3), NODE_BG);
            painter.rect_stroke(
                r,
                CornerRadius::same(3),
                Stroke::new(if sel { 1.5 } else { 1.0 }, if sel { ACCENT } else { Color32::from_rgb(46, 48, 50) }),
                egui::StrokeKind::Inside,
            );
            // header: icon, title, power switch
            let icon = Rect::from_center_size(r.min + vec2(11.0, 12.0) * zoom, Vec2::splat(9.0 * zoom));
            painter.rect_filled(icon, CornerRadius::same(2), if n.enabled { n.kind.color() } else { DIM });
            painter.text(r.min + vec2(20.0, 12.0) * zoom, egui::Align2::LEFT_CENTER, n.title(), font(9.5), txt);
            if n.kind.has_power() {
                let c = pos2(r.max.x - 11.0 * zoom, r.min.y + 12.0 * zoom);
                let pr = ui.interact(Rect::from_center_size(c, Vec2::splat(12.0 * zoom)), egui::Id::new(("pow", n.id)), Sense::click());
                let col = if pr.hovered() { TEXT } else { Color32::from_rgb(120, 122, 124) };
                painter.circle_stroke(c, 4.0 * zoom, Stroke::new(1.0, col));
                painter.line_segment([c - vec2(0.0, 5.5 * zoom), c - vec2(0.0, 1.5 * zoom)], Stroke::new(1.5, NODE_BG));
                painter.line_segment([c - vec2(0.0, 5.0 * zoom), c - vec2(0.0, 1.0 * zoom)], Stroke::new(1.0, col));
                if pr.on_hover_text("Turn node on/off").clicked() {
                    action = Some((i, "Turn on/off"));
                }
            }
            // ports
            let (ins, outs) = n.kind.ports();
            for (k, name) in ins.iter().enumerate() {
                let pp = to_screen(n.port_pos(false, k));
                painter.circle_filled(pp, 2.6 * zoom, Color32::from_rgb(150, 152, 154));
                painter.text(pp + vec2(7.0 * zoom, 0.0), egui::Align2::LEFT_CENTER, *name, font(8.5), Color32::from_rgb(140, 142, 144));
                let pr = ui.interact(Rect::from_center_size(pp, Vec2::splat(11.0 * zoom)), egui::Id::new(("in", n.id, k)), Sense::drag());
                if pr.drag_started() {
                    // pick up an existing wire from this input
                    if let Some(wi) = self.wires.iter().rposition(|w| w.to == i && w.inp == k) {
                        let w = self.wires.remove(wi);
                        start_wire = Some((w.from, w.out));
                    }
                }
            }
            for (k, name) in outs.iter().enumerate() {
                let pp = to_screen(n.port_pos(true, k));
                painter.circle_filled(pp, 2.6 * zoom, Color32::from_rgb(200, 202, 204));
                painter.text(pp - vec2(7.0 * zoom, 0.0), egui::Align2::RIGHT_CENTER, *name, font(8.5), Color32::from_rgb(140, 142, 144));
                let pr = ui.interact(Rect::from_center_size(pp, Vec2::splat(11.0 * zoom)), egui::Id::new(("out", n.id, k)), Sense::drag());
                if pr.drag_started() {
                    start_wire = Some((i, k));
                }
            }
        }
        if start_wire.is_some() {
            self.drag_wire = start_wire;
        }
        if let Some((i, a)) = action {
            match a {
                "Turn on/off" | "Toggle on/off" => self.nodes[i].enabled = !self.nodes[i].enabled,
                "Rename" => {
                    // the name field sits at the top of the node's settings
                    self.pick(i, false);
                    self.details_tab = 0;
                    self.rename_focus = true;
                }
                _ => self.nodes[i].params = self.nodes[i].kind.params(),
            }
        }

        // wire being dragged
        if let Some((from, out)) = self.drag_wire {
            if let Some(m) = pointer {
                let a = to_screen(self.nodes[from].port_pos(true, out));
                draw_wire(&painter, a, m, zoom, ACCENT);
            }
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(m) = pointer {
                    let ty = port_type(self.nodes[from].kind.ports().1[out]);
                    'find: for (j, n) in self.nodes.iter().enumerate() {
                        for (k, name) in n.kind.ports().0.iter().enumerate() {
                            if j != from && to_screen(n.port_pos(false, k)).distance(m) < 10.0 * zoom && port_type(name) == ty {
                                if !is_multi(name) {
                                    self.wires.retain(|w| !(w.to == j && w.inp == k));
                                }
                                let w = Wire { from, out, to: j, inp: k };
                                if !self.wires.contains(&w) {
                                    self.wires.push(w);
                                }
                                break 'find;
                            }
                        }
                    }
                }
                self.drag_wire = None;
            }
        }

        // shift+drag box selection
        if let (Some(a), Some(m)) = (self.box_start, pointer) {
            let sel = Rect::from_two_pos(a, m);
            painter.rect(sel, 0.0, ACCENT.gamma_multiply(0.12), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
            if !bg.dragged() {
                self.picked = (0..self.nodes.len())
                    .filter(|&i| sel.intersects(Rect::from_min_size(to_screen(self.nodes[i].pos), self.nodes[i].size() * zoom)))
                    .collect();
                self.selected = self.picked.first().copied();
                self.box_start = None;
            }
        }

        // Ctrl+G comment, H note under the mouse (other shortcuts are app-wide)
        if pointer.is_some_and(|m| rect.contains(m)) && !ui.ctx().wants_keyboard_input() {
            let (comment, note) = ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::G), !i.modifiers.command && i.key_pressed(egui::Key::H)));
            if comment || note {
                let i = self.add(if comment { Kind::Comment } else { Kind::Note }, to_graph(pointer.unwrap()));
                self.pick(i, false);
            }
        }
    }

    // ---------- node details ----------

    pub fn details(&mut self, ui: &mut Ui) {
        // modulators that settings can be driven by (read by the settings' right-click menu)
        let mods: Vec<(u64, String)> = self.nodes.iter().filter(|n| n.kind.is_modulator()).map(|n| (n.id, n.title().to_string())).collect();
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("firecraft-modulators"), mods));
        match self.details_tab {
            1 => {
                let mut any = false;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for n in self.nodes.iter_mut() {
                        if !n.params.iter().any(|p| p.fav) {
                            continue;
                        }
                        any = true;
                        ui.label(egui::RichText::new(n.title()).strong());
                        let id = n.id;
                        for p in n.params.iter_mut().filter(|p| p.fav) {
                            param_row(ui, p, id);
                        }
                        ui.add_space(6.0);
                    }
                    if !any {
                        ui.label(egui::RichText::new("Click the ☆ next to any setting to pin it here.").weak());
                    }
                });
            }
            2 => {
                if let Some(n) = self.nodes.iter_mut().find(|n| n.kind == Kind::Render) {
                    let id = n.id;
                    for p in n.params.iter_mut().filter(|p| p.section == "Timeline") {
                        param_row(ui, p, id);
                    }
                }
                ui.label(egui::RichText::new("Playback runs at 60 frames per second.").weak());
            }
            3 => {
                ui.horizontal(|ui| {
                    if ui.button("🎲 Randomize now").clicked() {
                        self.randomize();
                    }
                    ui.label(egui::RichText::new("Tick the circle left of a setting to include it.").weak());
                });
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for n in self.nodes.iter_mut() {
                        if !n.params.iter().any(|p| p.rand) {
                            continue;
                        }
                        ui.label(egui::RichText::new(n.title()).strong());
                        let id = n.id;
                        for p in n.params.iter_mut().filter(|p| p.rand) {
                            param_row(ui, p, id);
                        }
                    }
                });
            }
            _ => self.node_details(ui),
        }
    }

    fn node_details(&mut self, ui: &mut Ui) {
        let Some(i) = self.selected.filter(|&i| i < self.nodes.len()) else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Select a node in the graph to see its settings").weak()));
            return;
        };
        let rename_focus = std::mem::take(&mut self.rename_focus);
        let n = &mut self.nodes[i];
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(3), n.kind.color());
            if n.kind.is_note() {
                ui.label(egui::RichText::new(n.kind.name()).size(14.0).color(TEXT));
            } else {
                // the node's name; edit to rename it
                let mut title = n.title().to_string();
                let te = ui.add(egui::TextEdit::singleline(&mut title).font(FontId::proportional(14.0)).desired_width(220.0).frame(false));
                if rename_focus {
                    te.request_focus();
                }
                if te.changed() {
                    n.title = if title == n.kind.name() { String::new() } else { title };
                }
                if !n.title.is_empty() {
                    ui.label(egui::RichText::new(n.kind.name()).size(10.5).color(DIM));
                }
            }
            if n.kind.has_power() {
                ui.checkbox(&mut n.enabled, "On");
            }
        });
        ui.add_space(6.0);
        // node-specific actions
        match n.kind {
            Kind::Render => {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if ui.button("⬆  Export").on_hover_text("Export with the settings in the Export tab").clicked() {
                        self.export_request = Some(i);
                    }
                });
            }
            Kind::ExportImage | Kind::ExportFlipbook | Kind::ExportVolume => {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if ui.button("⬇  Export now").clicked() {
                        self.export_request = Some(i);
                    }
                    ui.label(egui::RichText::new("Files go to the folder set in the Export tab.").weak());
                });
            }
            Kind::Import => {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label("File");
                    let shown = if n.text.is_empty() { "(none)".to_string() } else { n.text.clone() };
                    ui.label(egui::RichText::new(shown).color(TEXT));
                    if ui.button("Browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("FireCraft volume", &["vol"]).pick_file() {
                            n.text = f.display().to_string();
                            self.import_request = true;
                        }
                    }
                    if !n.text.is_empty() && ui.button("Reload").clicked() {
                        self.import_request = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("Wire into Simulation › Emitters: the volume becomes the starting state when the simulation restarts. Pick a density.vol from a volume export.").weak());
                });
            }
            Kind::ModMidi => {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new(crate::modulate::midi_status()).weak());
                });
            }
            k if k.is_modulator() => {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("Right-click any setting › Drive with modulator to connect this.").weak());
                });
            }
            _ => {}
        }
        ui.add_space(4.0);
        let mut sections: Vec<&'static str> = vec![];
        for p in &n.params {
            if !p.hidden && !sections.contains(&p.section) {
                sections.push(p.section);
            }
        }
        if n.kind.is_note() {
            ui.text_edit_multiline(&mut n.text);
            return;
        }
        if sections.is_empty() {
            ui.label(egui::RichText::new("This node has no settings; it gathers its inputs.").weak());
            return;
        }
        if !sections.contains(&self.active_section) {
            self.active_section = sections[0];
        }
        // tabs, wrapped into rows of up to 8 like the original
        let per_row = sections.len().min(8).max(1);
        for row in sections.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                let w = ((ui.available_width() - 20.0) / per_row as f32 - 2.0).min(150.0);
                for s in row {
                    let on = *s == self.active_section;
                    let btn = egui::Button::new(egui::RichText::new(*s).color(if on { Color32::WHITE } else { TEXT }))
                        .fill(if on { ACCENT } else { Color32::from_rgb(40, 42, 44) })
                        .corner_radius(0.0)
                        .min_size(vec2(w, 20.0));
                    if ui.add(btn).clicked() {
                        self.active_section = s;
                    }
                }
            });
        }
        ui.add_space(4.0);
        let id = n.id;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // only the chosen tab's settings
            let s = self.active_section;
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(s).color(Color32::from_rgb(160, 162, 164)));
            });
            let rows: Vec<usize> = (0..n.params.len()).filter(|&k| n.params[k].section == s && n.visible(&n.params[k])).collect();
            for k in rows {
                if let Val::Ui(key) = n.params[k].val {
                    if crate::widgets::special_row(ui, key, n.params[k].label, n) {
                        self.export_request = Some(i);
                    }
                } else {
                    param_row(ui, &mut n.params[k], id);
                }
            }
        });
    }
}


/// Project text for a set of nodes and the wires between them.
pub fn save_parts(nodes: &[Node], wires: &[Wire]) -> String {
    let mut s = String::from("firecraft-project 2\n");
    for n in nodes {
        s += &format!("node\t{:?}\t{}\t{}\t{}\n", n.kind, n.pos.x, n.pos.y, n.enabled as u8);
        if !n.title.is_empty() {
            s += &format!("title\t{}\n", n.title.replace(['\t', '\n'], " "));
        }
        if !n.text.is_empty() {
            s += &format!("text\t{}\n", n.text.replace('\t', " ").replace('\n', "\\n"));
        }
        for p in &n.params {
            let v = match &p.val {
                Val::F(v) => format!("{v}"),
                Val::B(v) => format!("{}", *v as u8),
                Val::C(a) | Val::V(a) => format!("{} {} {}", a[0], a[1], a[2]),
                Val::Ch(v) => format!("{v}"),
                Val::R(a) => format!("{} {}", a[0], a[1]),
                Val::G(stops) => stops.iter().map(|s| format!("{} {} {} {}", s.pos, s.col[0], s.col[1], s.col[2])).collect::<Vec<_>>().join(";"),
                Val::S(t) => t.replace(['\t', '\n'], " "),
                Val::P(ps) => ps.iter().map(|q| format!("{} {} {}", q.src, q.hdr as u8, q.dst)).collect::<Vec<_>>().join(";"),
                Val::Ui(_) => continue,
            };
            let drv = p.driver.and_then(|d| nodes.iter().position(|m| m.id == d)).map_or(String::new(), |ix| format!("\t{ix}\t{}", p.axis));
            s += &format!("p\t{}\t{}\t{}\t{}{}\n", p.name, v, p.fav as u8, p.rand as u8, drv);
        }
    }
    for w in wires {
        s += &format!("wire\t{}\t{}\t{}\t{}\n", w.from, w.out, w.to, w.inp);
    }
    s
}

fn draw_wire(painter: &egui::Painter, a: Pos2, b: Pos2, zoom: f32, col: Color32) {
    let d = ((b.x - a.x).abs() * 0.5).max(30.0 * zoom);
    painter.add(CubicBezierShape::from_points_stroke(
        [a, a + vec2(d, 0.0), b - vec2(d, 0.0), b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.3 * zoom.max(0.7), col),
    ));
}

/// One settings row: randomize toggle, dotted label, value, reset, favourite.
pub fn param_row(ui: &mut Ui, p: &mut Param, node_id: u64) {
    ui.horizontal(|ui| {
        ui.set_min_height(21.0);
        ui.add_space(6.0);
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        ui.painter().circle_stroke(r.center(), 5.0, Stroke::new(1.0, Color32::from_rgb(80, 82, 84)));
        if p.rand {
            ui.painter().circle_filled(r.center(), 3.0, ACCENT);
        }
        if resp.on_hover_text("Include in Randomize").clicked() {
            p.rand = !p.rand;
        }
        let (lr, label_resp) = ui.allocate_exact_size(vec2(150.0, 18.0), Sense::click());
        let mut menus = vec![label_resp.on_hover_text(if p.driver.is_some() { "Driven by a modulator (right-click to change)" } else { "Right-click for options" })];
        let name_col = if p.driver.is_some() { Color32::from_rgb(255, 140, 120) } else { TEXT };
        let shown = if p.driver.is_some() { format!("∿ {}", p.label) } else { p.label.to_string() };
        let galley = ui.painter().layout_no_wrap(shown, FontId::proportional(12.0), name_col);
        let tw = galley.size().x;
        ui.painter().galley(pos2(lr.min.x, lr.center().y - galley.size().y / 2.0), galley, TEXT);
        let mut x = lr.min.x + tw + 3.0;
        while x < lr.max.x - 4.0 {
            ui.painter().circle_filled(pos2(x, lr.center().y + 5.0), 0.6, Color32::from_rgb(90, 92, 94));
            x += 3.0;
        }
        let (lo, hi) = p.range;
        let speed = (hi - lo) / 400.0;
        let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
        let dec = p.dec as usize;
        let speed = if dec == 0 { speed.max(0.2) } else { speed };
        ui.push_id((node_id, p.name), |ui| match &mut p.val {
            Val::F(v) => {
                menus.push(ui.add_sized([210.0, 19.0], egui::DragValue::new(v).range(lo..=hi).speed(speed).fixed_decimals(dec).suffix(unit)));
            }
            Val::R(a) => {
                let (mut x, mut y) = (a[0], a[1]);
                menus.push(ui.add_sized([103.0, 19.0], egui::DragValue::new(&mut x).range(lo..=hi).speed(speed).fixed_decimals(dec).suffix(format!("{unit} min"))));
                menus.push(ui.add_sized([103.0, 19.0], egui::DragValue::new(&mut y).range(lo..=hi).speed(speed).fixed_decimals(dec).prefix("max ").suffix(unit.clone())));
                *a = [x.min(y), y.max(x)];
            }
            Val::G(stops) => {
                crate::widgets::gradient_editor(ui, stops, node_id);
            }
            Val::S(t) => {
                if p.name == "Directory" {
                    if ui.add(egui::Button::new("🗁").frame(false)).on_hover_text("Choose folder").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            *t = d.display().to_string();
                        }
                    }
                    ui.add(egui::TextEdit::singleline(t).hint_text("Documents/FireCraft Exports").desired_width(290.0));
                } else {
                    ui.add(egui::TextEdit::singleline(t).desired_width(210.0));
                }
            }
            Val::P(ps) => {
                crate::widgets::passes_editor(ui, ps);
            }
            Val::Ui(_) => {}
            Val::B(v) => {
                ui.checkbox(v, "");
            }
            Val::C(c) => {
                ui.color_edit_button_rgb(c);
            }
            Val::V(a) => {
                for (k, l) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(egui::RichText::new(*l).color(Color32::from_rgb(150, 152, 154)));
                    menus.push(ui.add_sized([86.0, 19.0], egui::DragValue::new(&mut a[k]).range(lo..=hi).speed(speed).fixed_decimals(dec).suffix(unit.clone())));
                }
            }
            Val::Ch(i) if p.seg => {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (k, c) in p.choices.iter().enumerate() {
                    let on = *i == k;
                    let b = egui::Button::new(egui::RichText::new(*c).color(if on { Color32::WHITE } else { TEXT }))
                        .fill(if on { Color32::from_rgb(20, 55, 82) } else { Color32::from_rgb(50, 52, 53) })
                        .corner_radius(0.0)
                        .min_size(vec2(105.0, 19.0));
                    if ui.add(b).clicked() {
                        *i = k;
                    }
                }
            }
            Val::Ch(i) => {
                *i = (*i).min(p.choices.len().saturating_sub(1));
                egui::ComboBox::from_id_salt("ch").width(200.0).selected_text(p.choices[*i]).show_ui(ui, |ui| {
                    for (k, c) in p.choices.iter().enumerate() {
                        ui.selectable_value(i, k, *c);
                    }
                });
            }
        });
        let reset = egui::Button::new(egui::RichText::new("↺").color(Color32::from_rgb(140, 142, 144))).frame(false);
        if ui.add(reset).on_hover_text("Reset to default").clicked() {
            p.val = p.def.clone();
        }
        let star = egui::Button::new(egui::RichText::new(if p.fav { "★" } else { "☆" }).color(if p.fav { TEXT } else { Color32::from_rgb(110, 112, 114) })).frame(false);
        if ui.add(star).on_hover_text("Add to Favorites").clicked() {
            p.fav = !p.fav;
        }
        for r in menus {
            r.context_menu(|ui| param_menu(ui, p));
        }
    });
}

fn param_menu(ui: &mut Ui, p: &mut Param) {
    let clip_id = egui::Id::new("firecraft-param-clipboard");
    if matches!(p.val, Val::F(_) | Val::V(_) | Val::B(_) | Val::Ch(_)) {
        let mods: Vec<(u64, String)> = ui.ctx().data(|d| d.get_temp(egui::Id::new("firecraft-modulators"))).unwrap_or_default();
        ui.menu_button("Drive with modulator", |ui| {
            if mods.is_empty() {
                ui.label(egui::RichText::new("Add a Modulator node first").weak());
            }
            for (id, name) in &mods {
                if matches!(p.val, Val::V(_)) {
                    ui.menu_button(name, |ui| {
                        for (ax, l) in ["X", "Y", "Z", "All"].iter().enumerate() {
                            if ui.button(*l).clicked() {
                                p.driver = Some(*id);
                                p.axis = ax as u8;
                                ui.close_menu();
                            }
                        }
                    });
                } else if ui.selectable_label(p.driver == Some(*id), name).clicked() {
                    p.driver = Some(*id);
                    p.axis = 3;
                    ui.close_menu();
                }
            }
        });
        if p.driver.is_some() && ui.button("Remove modulator").clicked() {
            p.driver = None;
            ui.close_menu();
        }
        ui.separator();
    }
    if ui.button("Reset to default").clicked() {
        p.val = p.def.clone();
        ui.close_menu();
    }
    ui.separator();
    if ui.add(egui::Button::new("Copy value")).clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(clip_id, p.val.clone()));
        ui.close_menu();
    }
    let clip: Option<Val> = ui.ctx().data(|d| d.get_temp(clip_id));
    let fits = clip.as_ref().is_some_and(|c| std::mem::discriminant(c) == std::mem::discriminant(&p.val));
    if ui.add_enabled(fits, egui::Button::new("Paste value")).clicked() {
        if let Some(c) = clip {
            p.val = c;
        }
        ui.close_menu();
    }
    ui.separator();
    if ui.button(if p.fav { "Remove from Favorites" } else { "Add to Favorites" }).clicked() {
        p.fav = !p.fav;
        ui.close_menu();
    }
    if ui.button(if p.rand { "Exclude from Randomize" } else { "Include in Randomize" }).clicked() {
        p.rand = !p.rand;
        ui.close_menu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_units_are_converted() {
        // a 0.4 project: noise Scale 1.0 was the Fire Chaos default (25 cm today)
        let old = "firecraft-project 1\nnode\tFireChaos\t0\t0\t1\np\tScale\t1\np\tStrength\t2\n";
        let g = Graph::load(old).unwrap();
        assert_eq!(g.nodes[0].f("Scale"), 25.0);
        assert!((g.nodes[0].f("Amplitude") - 19.333).abs() < 1e-4);
        let g2 = Graph::load(&g.save()).unwrap();
        assert_eq!(g2.nodes[0].f("Scale"), 25.0);
    }

    #[test]
    fn default_graph_is_live_and_round_trips() {
        let g = Graph::default();
        let live = g.live();
        let em = g.first(Kind::Emitter).unwrap();
        let sk = g.first(Kind::Skybox).unwrap();
        assert!(live[em] && !live[sk], "emitter feeds render, skybox is off");
        let sim = g.first(Kind::Simulation).unwrap();
        assert_eq!(g.inputs(&live, sim, "Emitters"), vec![em]);
        let g2 = Graph::load(&g.save()).unwrap();
        assert_eq!(g2.nodes.len(), g.nodes.len());
        assert_eq!(g2.wires.len(), g.wires.len());
        assert_eq!(g2.nodes[em].f("Fuel rate"), 250.0);

        // copy/paste keeps the wire between two copied nodes; restore undoes it
        let mut g3 = Graph::default();
        let before = g3.save();
        let shape = g3.first(Kind::ShapePrimitive).unwrap();
        g3.picked = vec![shape, em];
        let clip = g3.copy_selection().unwrap();
        assert!(g3.paste(&clip));
        assert_eq!(g3.nodes.len(), g.nodes.len() + 2);
        assert!(g3.wires.iter().any(|w| w.from == g.nodes.len() && w.to == g.nodes.len() + 1));
        g3.delete_selection();
        assert_eq!(g3.save(), before);
        g3.select_all();
        g3.delete_selection();
        assert!(g3.nodes.is_empty());
        g3.restore(&before);
        assert_eq!(g3.save(), before);
    }
}
