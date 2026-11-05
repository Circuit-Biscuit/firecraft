// Modulator nodes: values over time that drive settings ("Drive with modulator").
use crate::graph::{Graph, Kind, Val};

/// Value of modulator node `i` at time `t` (seconds).
pub fn eval(g: &Graph, i: usize, t: f32, depth: u32) -> f32 {
    let n = &g.nodes[i];
    if !n.enabled || depth > 16 {
        return 0.0;
    }
    let input = |port: &str, tt: f32| g.sources(i, port).first().map(|&j| eval(g, j, tt, depth + 1));
    match n.kind {
        Kind::ModConstant => n.f("Value"),
        Kind::ModOscillator => {
            let x = (t * n.f("Frequency") + n.f("Phase")).fract();
            let w = match n.ch("Waveform") {
                1 => if x < 0.5 { 1.0 } else { -1.0 },
                2 => 1.0 - 4.0 * (x - 0.5).abs(),
                3 => 2.0 * x - 1.0,
                _ => (x * std::f32::consts::TAU).sin(),
            };
            n.f("Offset") + n.f("Amplitude") * w
        }
        Kind::ModCycle => {
            let period = n.f("Period").max(0.01);
            let x = if n.b("Repeat") { (t / period).fract() } else { (t / period).min(1.0) };
            n.f("Start value") + (n.f("End value") - n.f("Start value")) * x
        }
        Kind::ModMath => {
            let a = input("A", t).unwrap_or(n.f("A (if not wired)"));
            let b = input("B", t).unwrap_or(n.f("B (if not wired)"));
            match n.ch("Operation") {
                1 => a - b,
                2 => a * b,
                3 => if b.abs() > 1e-6 { a / b } else { 0.0 },
                4 => a.min(b),
                5 => a.max(b),
                6 => a.powf(b),
                _ => a + b,
            }
        }
        Kind::ModTimeShift => input("Value", t * n.f("Speed") - n.f("Delay")).unwrap_or(0.0),
        Kind::ModCombine => {
            let a = input("A", t).unwrap_or(0.0);
            let b = input("B", t).unwrap_or(0.0);
            a + (b - a) * n.f("Mix")
        }
        Kind::ModAdsr => {
            let rep = n.f("Repeat every");
            let tt = if rep > 0.0 { t % rep } else { t };
            let (a, d, s, hold, r) = (n.f("Attack"), n.f("Decay"), n.f("Sustain level"), n.f("Hold"), n.f("Release"));
            let env = if tt < a {
                tt / a.max(1e-4)
            } else if tt < a + d {
                1.0 - (1.0 - s) * (tt - a) / d.max(1e-4)
            } else if tt < a + d + hold {
                s
            } else {
                (s * (1.0 - (tt - a - d - hold) / r.max(1e-4))).max(0.0)
            };
            env * n.f("Peak")
        }
        Kind::ModMidi => {
            midi::open(n.f("Device") as u32);
            let ch = (n.f("Channel") as usize).clamp(1, 16) - 1;
            let raw = midi::value(ch, n.ch("Message") == 1, n.f("Number") as usize);
            n.f("Min") + (n.f("Max") - n.f("Min")) * raw
        }
        _ => 0.0,
    }
}

/// Copy of the graph with every modulator-driven setting replaced by its value at time `t`.
pub fn apply(g: &Graph, t: f32) -> Graph {
    let mut out = g.clone();
    for ni in 0..g.nodes.len() {
        for pi in 0..g.nodes[ni].params.len() {
            let Some(d) = g.nodes[ni].params[pi].driver else { continue };
            let Some(mi) = g.nodes.iter().position(|m| m.id == d && m.kind.is_modulator()) else { continue };
            let v = eval(g, mi, t, 0);
            let p = &mut out.nodes[ni].params[pi];
            let (lo, hi) = p.range;
            p.val = match &p.val {
                Val::F(_) => Val::F(v.clamp(lo, hi)),
                Val::B(_) => Val::B(v > 0.5),
                Val::Ch(_) => Val::Ch((v.round().max(0.0) as usize).min(p.choices.len().saturating_sub(1))),
                Val::V(a) => {
                    let mut a = *a;
                    for (k, c) in a.iter_mut().enumerate() {
                        if p.axis == 3 || p.axis as usize == k {
                            *c = v.clamp(lo, hi);
                        }
                    }
                    Val::V(a)
                }
                other => other.clone(),
            };
        }
    }
    out
}

pub fn midi_status() -> String {
    midi::status()
}

#[cfg(windows)]
mod midi {
    // Windows multimedia MIDI input (winmm), read into a table of the latest values.
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU8, Ordering};

    static VALUES: [[AtomicU8; 256]; 16] = [const { [const { AtomicU8::new(0) }; 256] }; 16];
    static OPEN: Mutex<Option<(u32, usize)>> = Mutex::new(None);

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn midiInGetNumDevs() -> u32;
        fn midiInOpen(handle: *mut usize, device: u32, callback: usize, instance: usize, flags: u32) -> u32;
        fn midiInStart(handle: usize) -> u32;
    }

    extern "system" fn callback(_h: usize, msg: u32, _inst: usize, p1: usize, _p2: usize) {
        const MIM_DATA: u32 = 0x3C3;
        if msg != MIM_DATA {
            return;
        }
        let status = (p1 & 0xff) as u8;
        let (d1, d2) = (((p1 >> 8) & 0x7f) as usize, ((p1 >> 16) & 0x7f) as u8);
        let ch = (status & 0x0f) as usize;
        match status & 0xf0 {
            0xB0 => VALUES[ch][d1].store(d2, Ordering::Relaxed),        // control change
            0x90 => VALUES[ch][128 + d1].store(d2, Ordering::Relaxed),  // note on (velocity)
            0x80 => VALUES[ch][128 + d1].store(0, Ordering::Relaxed),   // note off
            _ => {}
        }
    }

    pub fn open(device: u32) {
        let mut open = OPEN.lock().unwrap();
        if open.is_some_and(|(d, _)| d == device) {
            return;
        }
        // SAFETY: plain winmm calls; the callback only touches atomics.
        unsafe {
            if device >= midiInGetNumDevs() {
                return;
            }
            let mut h = 0usize;
            const CALLBACK_FUNCTION: u32 = 0x30000;
            if midiInOpen(&mut h, device, callback as usize, 0, CALLBACK_FUNCTION) == 0 {
                midiInStart(h);
                *open = Some((device, h));
            }
        }
    }

    pub fn value(ch: usize, note: bool, number: usize) -> f32 {
        let k = number.min(127) + if note { 128 } else { 0 };
        VALUES[ch.min(15)][k].load(Ordering::Relaxed) as f32 / 127.0
    }

    pub fn status() -> String {
        // SAFETY: simple query.
        let n = unsafe { midiInGetNumDevs() };
        match (n, *OPEN.lock().unwrap()) {
            (0, _) => "No MIDI input devices found. Plug one in; the value stays at Min until then.".into(),
            (n, Some((d, _))) => format!("{n} MIDI device(s); listening on device {d}."),
            (n, None) => format!("{n} MIDI device(s) found."),
        }
    }
}

#[cfg(not(windows))]
mod midi {
    pub fn open(_d: u32) {}
    pub fn value(_c: usize, _n: bool, _k: usize) -> f32 {
        0.0
    }
    pub fn status() -> String {
        "MIDI input is available on Windows.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test]
    fn oscillator_drives_a_setting() {
        let mut g = Graph::default();
        let osc = g.add(Kind::ModOscillator, pos2(0.0, 0.0));
        g.nodes[osc].set("Amplitude", Val::F(2.0));
        g.nodes[osc].set("Offset", Val::F(3.0));
        g.nodes[osc].set("Frequency", Val::F(1.0));
        let id = g.nodes[osc].id;
        let em = g.first(Kind::Emitter).unwrap();
        let p = g.nodes[em].params.iter_mut().find(|p| p.name == "Additional pressure rate").unwrap();
        p.driver = Some(id);
        let at = |t: f32| apply(&g, t).nodes[em].f("Additional pressure rate");
        assert!((at(0.25) - 5.0).abs() < 1e-3, "sine peak");
        assert!((at(0.75) - 1.0).abs() < 1e-3, "sine trough");
        // survives save/load
        let g2 = Graph::load(&g.save()).unwrap();
        let d = g2.nodes[em].params.iter().find(|p| p.name == "Additional pressure rate").unwrap().driver;
        assert_eq!(d, Some(g2.nodes[osc].id));
    }
}
