// Full-window home screen: presets gallery, recent projects and a short guide.
use crate::gpu::Target;
use crate::graph::{ACCENT, DIM, TEXT};
use crate::presets::PRESETS;
use eframe::egui::{self, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::collections::HashMap;
use std::path::PathBuf;

const BG: Color32 = Color32::from_rgb(32, 34, 35);
const BOX: Color32 = Color32::from_rgb(38, 41, 42);
const CARD: Color32 = Color32::from_rgb(14, 14, 15);

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Home,
    Presets,
    Projects,
}

pub enum Action {
    Return,
    NewProject,
    OpenDialog,
    Open(PathBuf),
    Preset(usize),
}

pub struct Home {
    pub page: Page,
    search: String,
    /// Live-rendered preset thumbnails, filled in one per frame by the app.
    pub thumbs: Vec<Option<(Target, egui::TextureId)>>,
    /// Pre-rendered pictures built into the app (decoded once, on first view).
    pictures: Vec<Option<egui::TextureHandle>>,
    pub recents: Vec<PathBuf>,
    proj_thumbs: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

/// Per-user settings folder: %APPDATA%\\FireCraft, ~/Library/Application Support/FireCraft
/// or ~/.config/FireCraft. (An installed app's own folder may be read-only.)
pub fn config_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home)
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
    };
    base.join("FireCraft")
}

/// Where exports go by default: Documents/FireCraft Exports.
pub fn default_export_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Documents").join("FireCraft Exports")
}

fn recents_file() -> PathBuf {
    config_dir().join("recent.txt")
}

/// Picture saved next to a project file.
pub fn thumb_path(project: &std::path::Path) -> PathBuf {
    PathBuf::from(format!("{}.png", project.display()))
}

impl Home {
    pub fn new() -> Self {
        let recents = std::fs::read_to_string(recents_file())
            .unwrap_or_default()
            .lines()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();
        Home {
            page: Page::Presets,
            search: String::new(),
            thumbs: (0..PRESETS.len()).map(|_| None).collect(),
            pictures: (0..PRESETS.len()).map(|_| None).collect(),
            recents,
            proj_thumbs: HashMap::new(),
        }
    }

    pub fn add_recent(&mut self, p: PathBuf) {
        self.recents.retain(|r| r != &p);
        self.recents.insert(0, p.clone());
        self.recents.truncate(12);
        self.proj_thumbs.remove(&p);
        let f = recents_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let text: Vec<String> = self.recents.iter().map(|p| p.display().to_string()).collect();
        let _ = std::fs::write(f, text.join("\n"));
    }

    /// Picture for preset `i`: the built-in one, else a live render if one has been made.
    fn picture(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureId> {
        if self.pictures[i].is_none() {
            if let Some((_, bytes)) = crate::preset_thumbs::PICTURES.iter().find(|(n, _)| *n == PRESETS[i].name) {
                if let Ok(img) = image::load_from_memory(bytes) {
                    let img = img.to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    self.pictures[i] = Some(ctx.load_texture(PRESETS[i].name, ci, egui::TextureOptions::LINEAR));
                }
            }
        }
        self.pictures[i].as_ref().map(|t| t.id()).or_else(|| self.thumbs[i].as_ref().map(|(_, id)| *id))
    }

    /// A preset without a built-in picture that still needs a live render (normally none).
    pub fn next_missing_thumb(&self) -> Option<usize> {
        if self.page == Page::Projects {
            return None;
        }
        let has_picture = |i: usize| crate::preset_thumbs::PICTURES.iter().any(|(n, _)| *n == PRESETS[i].name);
        (0..PRESETS.len()).find(|&i| self.thumbs[i].is_none() && !has_picture(i))
    }

    pub fn ui(&mut self, ctx: &egui::Context, version: &str) -> Option<Action> {
        let mut action = None;
        egui::TopBottomPanel::top("home_top").exact_height(30.0).frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(8, 3))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                logo(ui, 20.0);
                if ui.add(egui::Button::new("🏠").frame(false)).clicked() {
                    self.page = Page::Home;
                }
                if ui.add(egui::Button::new(RichText::new("↩ Return to Project").size(11.0)).fill(BOX)).clicked() {
                    action = Some(Action::Return);
                }
            });
        });
        egui::SidePanel::left("home_side").exact_width(214.0).resizable(false).frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 18, right: 6, top: 16, bottom: 12 })).show(ctx, |ui| {
            let w = ui.available_width();
            group(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, DIM), egui::StrokeKind::Inside);
                    ui.painter().circle_stroke(r.center() - vec2(0.0, 2.0), 3.5, Stroke::new(1.0, TEXT));
                    ui.label(RichText::new("Local user").color(TEXT));
                });
            });
            ui.add_space(10.0);
            if ui.add_sized([w, 26.0], egui::Button::new(RichText::new("+  New Project").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                action = Some(Action::NewProject);
            }
            ui.add_space(4.0);
            let open = egui::Button::new("Open Project").fill(BG).stroke(Stroke::new(1.0, Color32::from_rgb(70, 73, 75)));
            if ui.add_sized([w, 26.0], open).clicked() {
                action = Some(Action::OpenDialog);
            }
            ui.add_space(14.0);
            group(ui, |ui| {
                for (p, icon, label) in [(Page::Home, "🏠", "Home"), (Page::Presets, "🔥", "Presets"), (Page::Projects, "🗁", "My Projects")] {
                    let sel = self.page == p;
                    let b = egui::Button::new(RichText::new(format!("{icon}   {label}")).color(TEXT))
                        .fill(if sel { Color32::from_rgb(52, 55, 57) } else { BOX })
                        .min_size(vec2(ui.available_width(), 26.0));
                    if ui.add(b.wrap_mode(egui::TextWrapMode::Truncate)).clicked() {
                        self.page = p;
                    }
                }
            });
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.label(RichText::new(format!("Version {version}")).size(10.0).color(DIM));
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 0 })).show(ctx, |ui| {
            match self.page {
                Page::Presets => self.presets_page(ui, &mut action),
                Page::Projects => self.projects_page(ui, &mut action),
                Page::Home => self.home_page(ui, &mut action),
            }
        });
        action
    }

    fn presets_page(&mut self, ui: &mut Ui, action: &mut Option<Action>) {
        ui.label(RichText::new("Presets").size(14.0).color(TEXT));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} presets", PRESETS.len())).color(DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Search").desired_width(280.0));
            });
        });
        ui.add_space(12.0);
        let q = self.search.to_lowercase();
        let list: Vec<usize> = (0..PRESETS.len())
            .filter(|&i| q.is_empty() || PRESETS[i].name.to_lowercase().contains(&q))
            .collect();
        if list.is_empty() {
            ui.label(RichText::new("No presets match.").color(DIM));
        }
        let ctx = ui.ctx().clone();
        let texs: Vec<Option<egui::TextureId>> = list.iter().map(|&p| self.picture(&ctx, p)).collect();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if let Some(i) = card_grid(ui, list.len(), |k| (texs[k], PRESETS[list[k]].name.to_string(), String::new())) {
                *action = Some(Action::Preset(list[i]));
            }
        });
    }

    fn projects_page(&mut self, ui: &mut Ui, action: &mut Option<Action>) {
        ui.label(RichText::new("My Projects").size(14.0).color(TEXT));
        ui.add_space(12.0);
        if self.recents.is_empty() {
            ui.label(RichText::new("Projects you save or open appear here.").color(DIM));
            return;
        }
        self.load_project_thumbs(ui.ctx());
        let recents = self.recents.clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if let Some(i) = card_grid(ui, recents.len(), |k| {
                let p = &recents[k];
                let tex = self.proj_thumbs.get(p).and_then(|t| t.as_ref()).map(|t| t.id());
                let name = p.file_stem().map_or("Project".into(), |s| s.to_string_lossy().to_string());
                (tex, name, p.parent().map_or(String::new(), |d| d.display().to_string()))
            }) {
                *action = Some(Action::Open(recents[i].clone()));
            }
        });
    }

    fn home_page(&mut self, ui: &mut Ui, action: &mut Option<Action>) {
        ui.label(RichText::new("Welcome to FireCraft").size(20.0).color(TEXT));
        ui.label(RichText::new("Real-time fire, smoke and explosions. Start from a preset or a blank project.").color(DIM));
        ui.add_space(14.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if !self.recents.is_empty() {
                ui.label(RichText::new("Recent projects").size(13.0).color(TEXT));
                ui.add_space(6.0);
                self.load_project_thumbs(ui.ctx());
                let recents: Vec<PathBuf> = self.recents.iter().take(6).cloned().collect();
                if let Some(i) = card_grid(ui, recents.len(), |k| {
                    let p = &recents[k];
                    let tex = self.proj_thumbs.get(p).and_then(|t| t.as_ref()).map(|t| t.id());
                    (tex, p.file_stem().map_or("Project".into(), |s| s.to_string_lossy().to_string()), String::new())
                }) {
                    *action = Some(Action::Open(recents[i].clone()));
                }
                ui.add_space(12.0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Featured presets").size(13.0).color(TEXT));
                if ui.link("See all").clicked() {
                    self.page = Page::Presets;
                }
            });
            ui.add_space(6.0);
            let featured: Vec<usize> = ["Campfire", "Mushroom Cloud", "Smoke Ring", "Magic Portal", "Firework Burst", "Fire Tornado"]
                .iter()
                .filter_map(|n| PRESETS.iter().position(|p| p.name == *n))
                .collect();
            let ctx = ui.ctx().clone();
            let texs: Vec<Option<egui::TextureId>> = featured.iter().map(|&p| self.picture(&ctx, p)).collect();
            if let Some(i) = card_grid(ui, featured.len(), |k| (texs[k], PRESETS[featured[k]].name.to_string(), String::new())) {
                *action = Some(Action::Preset(featured[i]));
            }
        });
    }

    fn load_project_thumbs(&mut self, ctx: &egui::Context) {
        for p in &self.recents {
            self.proj_thumbs.entry(p.clone()).or_insert_with(|| {
                let img = image::open(thumb_path(p)).ok()?.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                Some(ctx.load_texture(p.display().to_string(), ci, egui::TextureOptions::LINEAR))
            });
        }
    }
}

/// The FireCraft logo (assets/logo.svg rendered to PNG at build time of the assets).
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/logo-256.png");

/// RGBA pixels of the logo, for the window icon and in-app images.
pub fn logo_rgba() -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory(LOGO_PNG).expect("logo png").to_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

/// Draw the logo `size` points tall (the texture is created once and shared).
pub fn logo(ui: &mut Ui, size: f32) {
    let id = egui::Id::new("firecraft-logo");
    let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let (px, w, h) = logo_rgba();
        let t = ui.ctx().load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px), egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
        t
    });
    ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(size, size)));
}

fn group(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(BOX).corner_radius(CornerRadius::same(3)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// Grid of 16:10 picture cards with a title; returns the clicked index.
fn card_grid(ui: &mut Ui, count: usize, mut item: impl FnMut(usize) -> (Option<egui::TextureId>, String, String)) -> Option<usize> {
    let gap = 28.0;
    let avail = ui.available_width() - 4.0;
    let cols = ((avail + gap) / (240.0 + gap)).floor().clamp(1.0, 6.0) as usize;
    let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
    let th = cw * 0.62;
    let mut clicked = None;
    for row in 0..count.div_ceil(cols) {
        let (r, _) = ui.allocate_exact_size(vec2(avail, th + 44.0), Sense::hover());
        for c in 0..cols {
            let k = row * cols + c;
            if k >= count {
                break;
            }
            let (tex, name, sub) = item(k);
            let card = Rect::from_min_size(pos2(r.min.x + c as f32 * (cw + gap), r.min.y), vec2(cw, th));
            let resp = ui.interact(card, ui.id().with(("card", k, row)), Sense::click());
            let p = ui.painter();
            p.rect_filled(card, CornerRadius::same(4), CARD);
            match tex {
                Some(id) => {
                    p.image(id, card, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    p.text(card.center(), egui::Align2::CENTER_CENTER, "Rendering…", FontId::proportional(11.0), DIM);
                }
            }
            if resp.hovered() {
                p.rect_stroke(card, CornerRadius::same(4), Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
            }
            p.text(pos2(card.min.x, card.max.y + 8.0), egui::Align2::LEFT_TOP, name, FontId::proportional(11.5), TEXT);
            if !sub.is_empty() {
                p.text(pos2(card.min.x, card.max.y + 24.0), egui::Align2::LEFT_TOP, sub, FontId::proportional(9.5), DIM);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(k);
            }
        }
    }
    clicked
}
