// Special setting widgets: gradient editor, render-pass mapping, histograms, previews, buttons.
use crate::graph::{ACCENT, DIM, GStop, Node, PASS_SOURCES, PASS_TARGETS, RPass, TEXT, Val};
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

/// Histograms published by the app each second: (temperature bins, smoke bins, temperature max K, smoke max).
pub const HIST_ID: &str = "firecraft-histograms";
/// Project name for filename previews.
pub const PROJECT_ID: &str = "firecraft-project-name";

#[derive(Clone, Default)]
pub struct Histograms {
    pub temp: Vec<f32>,
    pub smoke: Vec<f32>,
    pub temp_max: f32,
    pub smoke_max: f32,
}

// ---------- colour ----------

fn srgb_to_lin(c: f32) -> f32 {
    c.max(0.0).powf(2.2)
}
fn lin_to_srgb(c: f32) -> f32 {
    c.max(0.0).powf(1.0 / 2.2)
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0], c[1], c[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d < 1e-6 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if mx == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let (h, s, v) = (c[0].rem_euclid(1.0) * 6.0, c[1], c[2]);
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn to_oklab(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(srgb_to_lin);
    let l = (0.41222147 * r + 0.53633254 * g + 0.05144599 * b).cbrt();
    let m = (0.21190350 * r + 0.68069954 * g + 0.10739696 * b).cbrt();
    let s = (0.08830246 * r + 0.28171884 * g + 0.62997870 * b).cbrt();
    [0.21045426 * l + 0.79361779 * m - 0.00407205 * s, 1.97799850 * l - 2.42859221 * m + 0.45059371 * s, 0.02590404 * l + 0.78277177 * m - 0.80867577 * s]
}

fn from_oklab(c: [f32; 3]) -> [f32; 3] {
    let l = c[0] + 0.39633778 * c[1] + 0.21580376 * c[2];
    let m = c[0] - 0.10556135 * c[1] - 0.06385417 * c[2];
    let s = c[0] - 0.08948418 * c[1] - 1.29148555 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [4.07674166 * l - 3.30771159 * m + 0.23096993 * s, -1.26843800 * l + 2.60975740 * m - 0.34131940 * s, -0.00419609 * l - 0.70341861 * m + 1.70761470 * s].map(lin_to_srgb)
}

/// Colour of a gradient at `t` (0..1), interpolated in the chosen colour space
/// (0 RGB, 1 Linear RGB, 2 HSV, 3 OKLab).
pub fn eval_gradient(stops: &[GStop], space: usize, t: f32) -> [f32; 3] {
    if stops.is_empty() {
        return [t; 3];
    }
    let mut s: Vec<GStop> = stops.to_vec();
    s.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    if t <= s[0].pos {
        return s[0].col;
    }
    if t >= s[s.len() - 1].pos {
        return s[s.len() - 1].col;
    }
    let k = s.iter().position(|x| x.pos >= t).unwrap_or(1).max(1);
    let (a, b) = (s[k - 1], s[k]);
    let f = ((t - a.pos) / (b.pos - a.pos).max(1e-6)).clamp(0.0, 1.0);
    let lerp = |x: [f32; 3], y: [f32; 3]| [x[0] + (y[0] - x[0]) * f, x[1] + (y[1] - x[1]) * f, x[2] + (y[2] - x[2]) * f];
    match space {
        1 => lerp(a.col.map(srgb_to_lin), b.col.map(srgb_to_lin)).map(lin_to_srgb),
        2 => {
            let (mut ha, hb) = (rgb_to_hsv(a.col), rgb_to_hsv(b.col));
            if (hb[0] - ha[0]).abs() > 0.5 {
                ha[0] += if hb[0] > ha[0] { 1.0 } else { -1.0 };
            }
            hsv_to_rgb(lerp(ha, hb))
        }
        3 => from_oklab(lerp(to_oklab(a.col), to_oklab(b.col))),
        _ => lerp(a.col, b.col),
    }
}

/// 256-entry lookup table for the renderer.
pub fn bake_gradient(stops: &[GStop], space: usize) -> Vec<[f32; 3]> {
    (0..256).map(|i| eval_gradient(stops, space, i as f32 / 255.0)).collect()
}

fn c32(c: [f32; 3]) -> Color32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

// ---------- gradient editor ----------

/// Click below the bar to add a stop, drag stops to move them, right-click a stop to remove it.
pub fn gradient_editor(ui: &mut Ui, stops: &mut Vec<GStop>, node_id: u64) {
    let sel_id = egui::Id::new(("grad-sel", node_id));
    let mut sel: usize = ui.ctx().data(|d| d.get_temp(sel_id)).unwrap_or(0);
    ui.vertical(|ui| {
        let w = 330.0;
        let (rect, bar) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
        let painter = ui.painter();
        for x in 0..w as usize {
            let t = x as f32 / (w - 1.0);
            let c = eval_gradient(stops, 0, t);
            painter.line_segment([pos2(rect.min.x + x as f32, rect.min.y), pos2(rect.min.x + x as f32, rect.max.y)], Stroke::new(1.0, c32(c)));
        }
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Outside);
        if bar.clicked() {
            if let Some(p) = bar.interact_pointer_pos() {
                let t = ((p.x - rect.min.x) / w).clamp(0.0, 1.0);
                stops.push(GStop { pos: t, col: eval_gradient(stops, 0, t) });
                sel = stops.len() - 1;
            }
        }
        // stop markers
        let (mrect, _) = ui.allocate_exact_size(vec2(w, 12.0), Sense::hover());
        let mut remove = None;
        for (k, s) in stops.iter_mut().enumerate() {
            let x = mrect.min.x + s.pos * w;
            let hit = Rect::from_center_size(pos2(x, mrect.center().y), vec2(10.0, 12.0));
            let resp = ui.interact(hit, egui::Id::new(("grad-stop", node_id, k)), Sense::click_and_drag());
            if resp.dragged() {
                s.pos = (s.pos + resp.drag_delta().x / w).clamp(0.0, 1.0);
                sel = k;
            }
            if resp.clicked() {
                sel = k;
            }
            if resp.secondary_clicked() {
                remove = Some(k);
            }
            let tri = vec![pos2(x, mrect.min.y), pos2(x - 5.0, mrect.max.y), pos2(x + 5.0, mrect.max.y)];
            ui.painter().add(egui::Shape::convex_polygon(tri, c32(s.col), Stroke::new(1.0, if k == sel { ACCENT } else { Color32::from_gray(20) })));
            resp.on_hover_text("Drag to move · right-click to remove");
        }
        if let Some(k) = remove.filter(|_| stops.len() > 2) {
            stops.remove(k);
            sel = 0;
        }
        sel = sel.min(stops.len().saturating_sub(1));
        ui.horizontal(|ui| {
            if ui.button("+").on_hover_text("Add a stop").clicked() {
                stops.push(GStop { pos: 0.5, col: eval_gradient(stops, 0, 0.5) });
                sel = stops.len() - 1;
            }
            if let Some(s) = stops.get_mut(sel) {
                ui.label(egui::RichText::new("Stop").color(DIM));
                ui.color_edit_button_rgb(&mut s.col);
                ui.add(egui::DragValue::new(&mut s.pos).range(0.0..=1.0).speed(0.005).fixed_decimals(3));
            }
            if ui.button("Import").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("Gradient", &["fcgrad", "txt"]).pick_file() {
                    if let Ok(t) = std::fs::read_to_string(f) {
                        let read: Vec<GStop> = t
                            .lines()
                            .filter_map(|l| {
                                let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                                (v.len() == 4).then(|| GStop { pos: v[0], col: [v[1], v[2], v[3]] })
                            })
                            .collect();
                        if read.len() >= 2 {
                            *stops = read;
                        }
                    }
                }
            }
            ui.menu_button("Export", |ui| {
                if ui.button("Gradient file (.fcgrad)").clicked() {
                    ui.close_menu();
                    if let Some(f) = rfd::FileDialog::new().add_filter("Gradient", &["fcgrad"]).set_file_name("gradient.fcgrad").save_file() {
                        let t: String = stops.iter().map(|s| format!("{} {} {} {}\n", s.pos, s.col[0], s.col[1], s.col[2])).collect();
                        let _ = std::fs::write(f, t);
                    }
                }
                if ui.button("Image strip (.png)").clicked() {
                    ui.close_menu();
                    if let Some(f) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name("gradient.png").save_file() {
                        let px: Vec<u8> = (0..256).flat_map(|i| {
                            let c = eval_gradient(stops, 0, i as f32 / 255.0);
                            [c[0], c[1], c[2]].map(|x| (x.clamp(0.0, 1.0) * 255.0) as u8).into_iter().chain([255])
                        }).collect();
                        let rows: Vec<u8> = (0..16).flat_map(|_| px.clone()).collect();
                        let _ = image::save_buffer(f, &rows, 256, 16, image::ColorType::Rgba8);
                    }
                }
            });
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
}

// ---------- render passes ----------

pub fn passes_editor(ui: &mut Ui, passes: &mut Vec<RPass>) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add_sized([150.0, 18.0], egui::Label::new(egui::RichText::new("Source").color(DIM)));
            ui.add_sized([150.0, 18.0], egui::Label::new(egui::RichText::new("Destination").color(DIM)));
        });
        let mut remove = None;
        for (k, p) in passes.iter_mut().enumerate() {
            ui.push_id(k, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("src").width(110.0).selected_text(PASS_SOURCES[p.src]).show_ui(ui, |ui| {
                        for (i, s) in PASS_SOURCES.iter().enumerate() {
                            ui.selectable_value(&mut p.src, i, *s);
                        }
                    });
                    ui.toggle_value(&mut p.hdr, "HDR").on_hover_text("Keep the full brightness range (EXR export)");
                    ui.label(egui::RichText::new("→").color(DIM));
                    egui::ComboBox::from_id_salt("dst").width(70.0).selected_text(PASS_TARGETS[p.dst]).show_ui(ui, |ui| {
                        for (i, s) in PASS_TARGETS.iter().enumerate() {
                            ui.selectable_value(&mut p.dst, i, *s);
                        }
                    });
                    if ui.add(egui::Button::new("🗑").frame(false)).on_hover_text("Remove pass").clicked() {
                        remove = Some(k);
                    }
                });
                // channel mapping, as in the original's table
                let src_ch: &[&str] = match p.src {
                    0 => &["Red", "Green", "Blue", "Alpha"],
                    2 | 3 => &["Red", "Green", "Blue", "Luminance"],
                    _ => &["Value"],
                };
                let dst_ch: &[&str] = match p.dst {
                    0 => &["R", "G", "B", "A"],
                    1 => &["R", "G", "B"],
                    _ => &["R"],
                };
                for (i, d) in dst_ch.iter().enumerate() {
                    let s = src_ch.get(i).or(src_ch.last()).unwrap();
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.add_sized([120.0, 14.0], egui::Label::new(egui::RichText::new(*s).size(10.5).color(DIM)));
                        ui.label(egui::RichText::new(format!("— {d}")).size(10.5).color(DIM));
                    });
                }
            });
        }
        if let Some(k) = remove {
            passes.remove(k);
        }
        ui.menu_button("＋ Add Render Pass", |ui| {
            for (i, s) in PASS_SOURCES.iter().enumerate() {
                if ui.button(*s).clicked() {
                    passes.push(RPass { src: i, hdr: false, dst: if i == 0 { 0 } else { 2 } });
                    ui.close_menu();
                }
            }
        });
    });
}

// ---------- rows drawn without a value ----------

fn label_cell(ui: &mut Ui, label: &str) {
    ui.add_space(26.0);
    let (lr, _) = ui.allocate_exact_size(vec2(150.0, 18.0), Sense::hover());
    ui.painter().text(pos2(lr.min.x, lr.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.0), TEXT);
}

fn histogram(ui: &mut Ui, bins: &[f32], max_label: &str, range: Option<([f32; 2], f32)>, key: &str) {
    let log_id = egui::Id::new(("hist-log", key));
    let mut log: bool = ui.ctx().data(|d| d.get_temp(log_id)).unwrap_or(true);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(400.0, 100.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, Color32::BLACK);
        let peak = bins.iter().cloned().fold(0.0f32, f32::max).max(1.0);
        let n = bins.len().max(1);
        for (i, &b) in bins.iter().enumerate() {
            let v = if log { (1.0 + b).ln() / (1.0 + peak).ln() } else { b / peak };
            let x0 = rect.min.x + i as f32 / n as f32 * rect.width();
            let x1 = rect.min.x + (i + 1) as f32 / n as f32 * rect.width();
            let inside = range.is_none_or(|(r, max)| {
                let t = (i as f32 + 0.5) / n as f32 * max;
                t >= r[0] && t <= r[1]
            });
            p.rect_filled(Rect::from_min_max(pos2(x0, rect.max.y - v * rect.height()), pos2(x1, rect.max.y)), 0.0, if inside { Color32::WHITE } else { Color32::from_gray(150) });
        }
        p.text(rect.left_bottom() + vec2(0.0, 2.0), egui::Align2::LEFT_TOP, "0", FontId::proportional(9.0), DIM);
        p.text(rect.right_bottom() + vec2(0.0, 2.0), egui::Align2::RIGHT_TOP, max_label, FontId::proportional(9.0), DIM);
        ui.checkbox(&mut log, "Log Y");
    });
    ui.add_space(10.0);
    ui.ctx().data_mut(|d| d.insert_temp(log_id, log));
}

fn noise_preview(ui: &mut Ui, n: &Node) {
    let np = crate::noise::NoiseParams {
        freq: 100.0 / n.f("Scale").max(0.5),
        octaves: n.f("Octaves") as u32,
        lacunarity: n.f("Lacunarity"),
        gain: n.f("Gain"),
        curl: n.b("Curl noise"),
        seed: n.f("Seed"),
        speed: 0.0,
    };
    let key = format!("{:?}", np);
    let id = egui::Id::new(("noise-preview", n.id));
    let cached: Option<(String, egui::TextureHandle)> = ui.ctx().data(|d| d.get_temp(id));
    let tex = match cached {
        Some((k, t)) if k == key => t,
        _ => {
            let (w, h) = (160usize, 64usize);
            let px: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = ((i % w) as f32 / w as f32 * 2.0, (i / w) as f32 / h as f32 * 0.8);
                    let f = crate::noise::fbm(&np, [x, y, 0.0], 0.0);
                    let v = (0.5 + f[0] * 0.35).clamp(0.0, 1.0);
                    let g = (v * 255.0) as u8;
                    [g, g, g, 255]
                })
                .collect();
            let t = ui.ctx().load_texture("noise-preview", egui::ColorImage::from_rgba_unmultiplied([w, h], &px), egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(id, (key, t.clone())));
            t
        }
    };
    ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(200.0, 50.0)).corner_radius(CornerRadius::same(3)));
}

/// Draws a display-only row. Returns true when "Export Now" was pressed.
pub fn special_row(ui: &mut Ui, key: &str, label: &str, n: &mut Node) -> bool {
    let mut export = false;
    ui.horizontal(|ui| {
        ui.set_min_height(21.0);
        label_cell(ui, label);
        match key {
            "domain_info" => {
                // the bounds size in metres; changing it changes the voxel count (detail stays the same)
                let vc = n.c("Voxel count");
                let h = n.f("Voxel size") / 100.0;
                let mut size = vc.map(|x| x.round() * h);
                let mut changed = false;
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                            ui.label(egui::RichText::new(*axis).color(DIM));
                            let dv = egui::DragValue::new(&mut size[k]).speed(0.01).range(8.0 * h..=512.0 * h).suffix(" m").fixed_decimals(2);
                            changed |= ui.add_sized([80.0, 18.0], dv).on_hover_text("Size of the simulation bounds. Or drag the handles on the box in the viewport.").changed();
                        }
                    });
                    let c = vc.map(|x| x.max(1.0).round() as u64);
                    let total = c[0] * c[1] * c[2];
                    ui.label(egui::RichText::new(format!(
                        "{} x {} x {} = {:.2} million voxels · ~{:.1} GB GPU",
                        c[0], c[1], c[2], total as f64 / 1e6, total as f64 * 84.0 / 1e9
                    )).color(DIM).small());
                });
                if changed {
                    n.set("Voxel count", Val::V(size.map(|s| (s / h).round().clamp(8.0, 512.0))));
                }
            }
            "hist_temp" | "hist_smoke" => {
                let h: Histograms = ui.ctx().data(|d| d.get_temp(egui::Id::new(HIST_ID))).unwrap_or_default();
                ui.vertical(|ui| {
                    if key == "hist_temp" {
                        histogram(ui, &h.temp, &format!("{:.1}K", h.temp_max / 1000.0), Some((n.r("Flames remap range"), h.temp_max)), key);
                    } else {
                        histogram(ui, &h.smoke, &format!("{:.3}", h.smoke_max), Some((n.r("Smoke remap range"), h.smoke_max)), key);
                    }
                });
            }
            "noise_preview" => noise_preview(ui, n),
            "flipbook_grid" => {
                let (cols, rows) = (n.f("Flipbook columns").max(1.0) as usize, n.f("Flipbook rows").max(1.0) as usize);
                let frames = n.f("Number of Frames").max(1.0) as usize;
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let size = 230.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                        let p = ui.painter();
                        let (cw, ch) = (size / cols as f32, size / rows as f32);
                        for r in 0..rows {
                            for c in 0..cols {
                                let used = r * cols + c < frames;
                                let cell = Rect::from_min_size(rect.min + vec2(c as f32 * cw, r as f32 * ch), vec2(cw, ch)).shrink(0.5);
                                p.rect_filled(cell, 0.0, if used { Color32::from_rgb(38, 72, 44) } else { Color32::from_rgb(40, 42, 44) });
                            }
                        }
                        ui.vertical(|ui| {
                            if ui.button("+").on_hover_text("More rows").clicked() {
                                n.set("Flipbook rows", Val::F((rows + 1) as f32));
                            }
                            if ui.button("−").on_hover_text("Fewer rows").clicked() && rows > 1 {
                                n.set("Flipbook rows", Val::F((rows - 1) as f32));
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.add_sized([114.0, 18.0], egui::Button::new("−")).on_hover_text("Fewer columns").clicked() && cols > 1 {
                            n.set("Flipbook columns", Val::F((cols - 1) as f32));
                        }
                        if ui.add_sized([114.0, 18.0], egui::Button::new("+")).on_hover_text("More columns").clicked() {
                            n.set("Flipbook columns", Val::F((cols + 1) as f32));
                        }
                    });
                    let used = frames.min(cols * rows);
                    ui.label(egui::RichText::new(format!("{used} of {} tiles used · tile {} x {} px", cols * rows, n.f("Flipbook width") as usize / cols, n.f("Flipbook height") as usize / rows)).weak());
                });
            }
            "export_buttons" => {
                if ui.button("Export Now").clicked() {
                    export = true;
                }
                if ui.button("Open Folder").clicked() {
                    let dir = export_dir(n);
                    let _ = std::fs::create_dir_all(&dir);
                    open_folder(&dir);
                }
                let project: String = ui.ctx().data(|d| d.get_temp(egui::Id::new(PROJECT_ID))).unwrap_or_else(|| "Untitled".into());
                let first = n.passes("Mapping").first().map_or("render_all".to_string(), |p| safename(p.src));
                let ext = [".png", ".exr", ".tga"][n.ch("File format").min(2)];
                ui.label(egui::RichText::new(format!("{}{ext}", expand_filename(&n.s("Filename"), &project, &first))).color(DIM));
                if n.s("Directory").is_empty() {
                    ui.label(egui::RichText::new("⚠").color(Color32::from_rgb(240, 190, 60)))
                        .on_hover_text(format!("No directory set: files go to {}", export_dir(n).display()));
                }
            }
            _ => {}
        }
    });
    export
}

pub fn safename(src: usize) -> String {
    PASS_SOURCES[src.min(PASS_SOURCES.len() - 1)].to_lowercase().replace(' ', "_")
}

/// $(project), $(safename) and $(frame) in a filename pattern.
pub fn expand_filename(pattern: &str, project: &str, safename: &str) -> String {
    let p = if pattern.trim().is_empty() { "$(project)_$(safename)" } else { pattern };
    p.replace("$(project)", project).replace("$(safename)", safename)
}

pub fn export_dir(n: &Node) -> std::path::PathBuf {
    let d = n.s("Directory");
    if d.trim().is_empty() { crate::home::default_export_dir() } else { std::path::PathBuf::from(d) }
}

pub fn open_folder(dir: &std::path::Path) {
    let cmd = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}

#[allow(dead_code)]
fn _unused(_: Pos2) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gradients_interpolate_in_every_space() {
        let s = vec![GStop { pos: 0.0, col: [1.0, 0.0, 0.0] }, GStop { pos: 1.0, col: [0.0, 0.0, 1.0] }];
        for space in 0..4 {
            let lut = bake_gradient(&s, space);
            assert_eq!(lut.len(), 256);
            assert!((lut[0][0] - 1.0).abs() < 1e-3 && (lut[255][2] - 1.0).abs() < 1e-3, "space {space} keeps the end colours");
        }
        assert_eq!(expand_filename("$(project)_$(safename)", "Fire", "alpha"), "Fire_alpha");
    }
}
