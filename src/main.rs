mod tagger;

use eframe::egui::{self, Color32, RichText};
use rfd::FileDialog;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use tagger::{Options, Outcome, PhotoInfo};

const ROW_HEIGHT: f32 = 24.0;

fn main() -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../icons/512x512.png"))
        .expect("Failed to load icon")
        .to_rgba8();
    let (width, height) = icon.dimensions();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 740.0])
            .with_min_inner_size([600.0, 560.0])
            .with_title("CantiniAutoGPS")
            .with_drag_and_drop(true)
            .with_icon(egui::IconData {
                rgba: icon.into_raw(),
                width,
                height,
            }),
        ..Default::default()
    };

    eframe::run_native(
        "CantiniAutoGPS",
        options,
        Box::new(|cc| Ok(Box::new(GeoTagApp::new(cc)))),
    )
}

// ------------------------------------------------------------------ styling

fn accent(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(10, 132, 255) // systemBlue (dark)
    } else {
        Color32::from_rgb(0, 122, 255) // systemBlue (light)
    }
}

fn setup_style(ctx: &egui::Context) {
    // Use the system font (San Francisco) when available; egui's defaults remain as fallback.
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts
            .font_data
            .insert("sf".into(), egui::FontData::from_owned(bytes));
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "sf".into());
    }
    ctx.set_fonts(fonts);

    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        ctx.style_mut_of(theme, |style| {
            use egui::{FontFamily::Proportional, FontId, TextStyle};
            style.text_styles = [
                (TextStyle::Heading, FontId::new(20.0, Proportional)),
                (TextStyle::Body, FontId::new(13.0, Proportional)),
                (TextStyle::Button, FontId::new(13.0, Proportional)),
                (TextStyle::Small, FontId::new(11.0, Proportional)),
                (TextStyle::Monospace, FontId::new(12.0, egui::FontFamily::Monospace)),
            ]
            .into();
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(12.0, 5.0);
            style.spacing.interact_size.y = 24.0;

            let v = &mut style.visuals;
            v.panel_fill = if dark {
                Color32::from_rgb(30, 30, 30)
            } else {
                Color32::from_rgb(246, 246, 246)
            };
            v.selection.bg_fill = accent(dark);
            v.hyperlink_color = accent(dark);
            let r = egui::Rounding::same(6.0);
            v.widgets.noninteractive.rounding = r;
            v.widgets.inactive.rounding = r;
            v.widgets.hovered.rounding = r;
            v.widgets.active.rounding = r;
            v.widgets.open.rounding = r;
            v.window_rounding = egui::Rounding::same(10.0);
        });
    }
}

fn secondary(text: impl Into<String>) -> RichText {
    RichText::new(text).small().weak()
}

/// A grouped section: heading above a rounded container, as in macOS settings panes.
fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.label(RichText::new(title).strong());
    let dark = ui.visuals().dark_mode;
    egui::Frame::none()
        .fill(if dark { Color32::from_rgb(44, 44, 46) } else { Color32::WHITE })
        .stroke(egui::Stroke::new(0.5_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .rounding(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

// ---------------------------------------------------------------------- app

enum Msg {
    Scanned(Vec<PhotoInfo>),
    Tagged(PathBuf, Outcome),
    Finished { cancelled: bool },
}

struct PhotoRow {
    info: PhotoInfo,
    outcome: Option<Outcome>,
}

struct GpxEntry {
    path: PathBuf,
    range: Option<(i64, i64)>,
}

struct GeoTagApp {
    exiftool: Option<PathBuf>,
    photos: Vec<PhotoRow>,
    tracks: Vec<GpxEntry>,
    timezones: Vec<String>,
    tz: String,
    tz_auto: bool,
    tz_detected: bool,
    drift_secs: i64,
    keep_backup: bool,
    replace_gps: bool,
    last_dir: Option<PathBuf>,

    log: String,
    scanning: usize,
    running: bool,
    progress: (usize, usize),
    summary: Option<String>,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl GeoTagApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let get = |k: &str| cc.storage.and_then(|s| s.get_string(k));
        let mut timezones: Vec<String> = (-12..=12).map(|h| format!("{h:+03}:00")).collect();
        timezones[12] = "+00:00".into();
        let tz = get("tz")
            .filter(|t| tagger::parse_offset(t).is_some())
            .unwrap_or_else(|| "+00:00".into());
        if !timezones.contains(&tz) {
            timezones.push(tz.clone());
        }
        let (tx, rx) = channel();
        let exiftool = tagger::find_exiftool();
        Self {
            log: match &exiftool {
                Some(p) => format!("Using ExifTool at {}\n", p.display()),
                None => "ExifTool was not found.\n".into(),
            },
            exiftool,
            photos: Vec::new(),
            tracks: Vec::new(),
            timezones,
            tz,
            tz_auto: true,
            tz_detected: false,
            drift_secs: 0,
            keep_backup: get("keep_backup").map(|v| v != "false").unwrap_or(true),
            replace_gps: get("replace_gps").map(|v| v == "true").unwrap_or(false),
            last_dir: get("last_dir").map(PathBuf::from).filter(|p| p.is_dir()),
            scanning: 0,
            running: false,
            progress: (0, 0),
            summary: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }

    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let paths = tagger::expand_dirs(paths);
        let (gpx, rest): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| tagger::is_gpx(p));
        for g in gpx {
            if !self.tracks.iter().any(|t| t.path == g) {
                let range = tagger::gpx_range(&g);
                if range.is_none() {
                    self.log.push_str(&format!("No timestamps found in {}\n", g.display()));
                }
                self.tracks.push(GpxEntry { path: g, range });
            }
        }
        let photos: Vec<PathBuf> = rest
            .into_iter()
            .filter(|p| tagger::is_photo(p))
            .filter(|p| !self.photos.iter().any(|r| &r.info.path == p))
            .collect();
        if photos.is_empty() {
            return;
        }
        let Some(exiftool) = self.exiftool.clone() else { return };
        self.scanning += 1;
        self.summary = None;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(Msg::Scanned(tagger::scan(&exiftool, &photos)));
            ctx.request_repaint();
        });
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Scanned(infos) => {
                    self.scanning = self.scanning.saturating_sub(1);
                    self.log.push_str(&format!("Read {} photo(s).\n", infos.len()));
                    if self.tz_auto {
                        if let Some(off) = infos.iter().find_map(|i| i.offset.clone()) {
                            if !self.timezones.contains(&off) {
                                self.timezones.push(off.clone());
                                self.timezones
                                    .sort_by_key(|t| tagger::parse_offset(t).unwrap_or(0));
                            }
                            self.tz = off;
                            self.tz_detected = true;
                        }
                    }
                    self.photos
                        .extend(infos.into_iter().map(|info| PhotoRow { info, outcome: None }));
                    self.photos.sort_by(|a, b| a.info.path.cmp(&b.info.path));
                }
                Msg::Tagged(path, outcome) => {
                    self.progress.0 += 1;
                    if let Outcome::Failed(m) | Outcome::NoMatch(m) = &outcome {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.log.push_str(&format!("{name}: {m}\n"));
                    }
                    if let Some(row) = self.photos.iter_mut().find(|r| r.info.path == path) {
                        row.outcome = Some(outcome);
                    }
                }
                Msg::Finished { cancelled } => {
                    self.running = false;
                    let count = |f: fn(&Outcome) -> bool| {
                        self.photos.iter().filter(|r| r.outcome.as_ref().is_some_and(f)).count()
                    };
                    let s = format!(
                        "{}Tagged {} · Already had GPS {} · No matching track point {} · Failed {}",
                        if cancelled { "Cancelled. " } else { "" },
                        count(|o| *o == Outcome::Tagged),
                        count(|o| *o == Outcome::AlreadyHasGps),
                        count(|o| matches!(o, Outcome::NoMatch(_))),
                        count(|o| matches!(o, Outcome::Failed(_))),
                    );
                    self.log.push_str(&format!("{s}\n"));
                    self.summary = Some(s);
                }
            }
        }
    }

    fn options(&self) -> Options {
        Options {
            tz: self.tz.clone(),
            drift_secs: self.drift_secs,
            keep_backup: self.keep_backup,
            replace_existing_gps: self.replace_gps,
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(exiftool) = self.exiftool.clone() else { return };
        let opts = self.options();
        let gpx: Vec<PathBuf> = self.tracks.iter().map(|t| t.path.clone()).collect();
        let photos: Vec<PathBuf> = self.photos.iter().map(|r| r.info.path.clone()).collect();
        for r in &mut self.photos {
            r.outcome = None;
        }
        self.running = true;
        self.summary = None;
        self.progress = (0, photos.len());
        self.cancel = Arc::new(AtomicBool::new(false));
        self.log.push_str(&format!("\nTagging {} photo(s)…\n", photos.len()));

        let (tx, ctx, cancel) = (self.tx.clone(), ctx.clone(), self.cancel.clone());
        thread::spawn(move || {
            let mut cancelled = false;
            for photo in photos {
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                let outcome = tagger::tag_one(&exiftool, &gpx, &opts, &photo);
                let _ = tx.send(Msg::Tagged(photo, outcome));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::Finished { cancelled });
            ctx.request_repaint();
        });
    }

    fn pick(&mut self, ctx: &egui::Context, photos: bool) {
        let mut dlg = FileDialog::new();
        if let Some(d) = &self.last_dir {
            dlg = dlg.set_directory(d);
        }
        dlg = if photos {
            let exts: Vec<String> = tagger::PHOTO_EXTS
                .iter()
                .flat_map(|e| [e.to_string(), e.to_uppercase()])
                .collect();
            dlg.add_filter("Photos", &exts)
        } else {
            dlg.add_filter("GPX tracks", &["gpx", "GPX"])
        };
        if let Some(files) = dlg.pick_files() {
            self.last_dir = files.first().and_then(|f| f.parent()).map(PathBuf::from);
            self.add_paths(ctx, files);
        }
    }

    /// How many photos (after time zone and clock correction) fall inside any loaded track.
    fn coverage(&self) -> Option<Coverage> {
        let offset = tagger::parse_offset(&self.tz)?;
        let ranges: Vec<(i64, i64)> = self.tracks.iter().filter_map(|t| t.range).collect();
        let times: Vec<i64> = self.photos.iter().filter_map(|r| r.info.local_time).collect();
        if times.is_empty() || ranges.is_empty() {
            return None;
        }
        let inside = times
            .iter()
            .filter(|&&t| {
                let utc = t - offset + self.drift_secs;
                ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&utc))
            })
            .count();
        Some(Coverage {
            inside,
            with_time: times.len(),
            photo_span: (*times.iter().min()?, *times.iter().max()?),
            track_span: (ranges.iter().map(|r| r.0).min()?, ranges.iter().map(|r| r.1).max()?),
        })
    }
}

struct Coverage {
    inside: usize,
    with_time: usize,
    photo_span: (i64, i64),
    track_span: (i64, i64),
}

impl eframe::App for GeoTagApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("tz", self.tz.clone());
        storage.set_string("keep_backup", self.keep_backup.to_string());
        storage.set_string("replace_gps", self.replace_gps.to_string());
        if let Some(d) = &self.last_dir {
            storage.set_string("last_dir", d.to_string_lossy().into_owned());
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages();

        // Drag and drop
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && !self.running {
            self.add_paths(ctx, dropped);
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let dark = ctx.style().visuals.dark_mode;
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            let rect = ctx.screen_rect().shrink(6.0);
            painter.rect(
                rect,
                12.0,
                accent(dark).gamma_multiply(0.12),
                egui::Stroke::new(2.0_f32, accent(dark)),
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop photos, folders or GPX files",
                egui::FontId::proportional(17.0),
                accent(dark),
            );
        }

        let ready = self.exiftool.is_some()
            && !self.photos.is_empty()
            && !self.tracks.is_empty()
            && self.scanning == 0
            && !self.running;

        // Bottom bar: progress / result on the left, primary action on the right
        egui::TopBottomPanel::bottom("actions")
            .frame(
                egui::Frame::none()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(20.0, 14.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dark = ui.visuals().dark_mode;
                        if self.running {
                            if ui
                                .add_sized([110.0, 30.0], egui::Button::new("Cancel"))
                                .clicked()
                            {
                                self.cancel.store(true, Ordering::Relaxed);
                            }
                        } else {
                            let btn = egui::Button::new(
                                RichText::new("Tag Photos").strong().color(Color32::WHITE),
                            )
                            .fill(accent(dark));
                            if ui.add_enabled(ready, btn.min_size(egui::vec2(120.0, 30.0))).clicked() {
                                self.start(ctx);
                            }
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if self.running {
                                let (done, total) = self.progress;
                                ui.add(
                                    egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                                        .desired_width(220.0),
                                );
                                ui.label(format!("{done} of {total}"));
                            } else if self.scanning > 0 {
                                ui.spinner();
                                ui.label("Reading photos…");
                            } else if let Some(s) = &self.summary {
                                ui.label(s);
                            }
                        });
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.panel_fill).inner_margin(20.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    self.content(ui, ctx);
                });
            });
    }
}

impl GeoTagApp {
    fn content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Geotag photos from a GPS track");
        ui.label(secondary("Matches each photo's capture time to your GPX track and writes the location."));
        ui.add_space(4.0);

        if self.exiftool.is_none() {
            let dark = ui.visuals().dark_mode;
            egui::Frame::none()
                .fill(Color32::from_rgb(255, 159, 10).gamma_multiply(if dark { 0.25 } else { 0.18 }))
                .rounding(10.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("⚠ ExifTool is required").strong());
                    ui.label("Install it with Homebrew, then reopen the app:");
                    ui.monospace("brew install exiftool");
                });
        }

        let busy = self.running;
        ui.add_enabled_ui(!busy, |ui| {
            // ---- Photos
            let mut remove: Option<usize> = None;
            section(ui, "Photos", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add Photos…").clicked() {
                        self.pick(ctx, true);
                    }
                    if ui.add_enabled(!self.photos.is_empty(), egui::Button::new("Clear")).clicked() {
                        self.photos.clear();
                        self.summary = None;
                    }
                    ui.label(secondary(match self.photos.len() {
                        0 => String::new(),
                        1 => "1 photo".into(),
                        n => format!("{n} photos"),
                    }));
                });
                if self.photos.is_empty() {
                    ui.add_space(4.0);
                    ui.vertical_centered(|ui| {
                        ui.label(secondary("Drop photos or folders here, or choose Add Photos…"));
                        ui.label(secondary("DNG, JPEG, HEIC, TIFF and most RAW formats"));
                    });
                    ui.add_space(4.0);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("photos")
                        .max_height(ROW_HEIGHT * 6.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, ROW_HEIGHT, self.photos.len(), |ui, range| {
                            for i in range {
                                let row = &self.photos[i];
                                ui.horizontal(|ui| {
                                    ui.set_height(ROW_HEIGHT - 8.0);
                                    ui.label(row.info.path.file_name().unwrap_or_default().to_string_lossy());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .small_button("✕")
                                                .on_hover_text("Remove from list")
                                                .clicked()
                                            {
                                                remove = Some(i);
                                            }
                                            status_label(ui, row);
                                        },
                                    );
                                });
                            }
                        });
                }
            });
            if let Some(i) = remove {
                self.photos.remove(i);
                self.summary = None;
            }

            ui.add_space(4.0);

            // ---- Tracks
            let mut remove_track: Option<usize> = None;
            section(ui, "GPS tracks", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add GPX…").clicked() {
                        self.pick(ctx, false);
                    }
                    ui.label(secondary("Add several for multi-day trips"));
                });
                if self.tracks.is_empty() {
                    ui.label(secondary("No track loaded. Drop a .gpx file here or choose Add GPX…"));
                }
                for (i, t) in self.tracks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(t.path.file_name().unwrap_or_default().to_string_lossy());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Remove from list").clicked() {
                                remove_track = Some(i);
                            }
                            ui.label(secondary(match t.range {
                                Some((lo, hi)) => format!(
                                    "{} – {} UTC",
                                    tagger::format_epoch(lo),
                                    tagger::format_epoch(hi)
                                ),
                                None => "No timestamps".into(),
                            }));
                        });
                    });
                }
            });
            if let Some(i) = remove_track {
                self.tracks.remove(i);
            }

            ui.add_space(4.0);

            // ---- Options
            section(ui, "Time and options", |ui| {
                egui::Grid::new("options").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                    ui.label("Camera time zone");
                    ui.vertical(|ui| {
                        egui::ComboBox::from_id_salt("tz")
                            .selected_text(format!("UTC{}", self.tz.replace('-', "−")))
                            .show_ui(ui, |ui| {
                                for tz in self.timezones.clone() {
                                    let label = format!("UTC{}", tz.replace('-', "−"));
                                    if ui.selectable_value(&mut self.tz, tz, label).clicked() {
                                        self.tz_auto = false;
                                        self.tz_detected = false;
                                    }
                                }
                            });
                        ui.label(secondary(if self.tz_detected {
                            "Detected from your photos."
                        } else {
                            "The time zone your camera clock was set to when shooting."
                        }));
                    });
                    ui.end_row();

                    ui.label("Clock correction");
                    ui.vertical(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.drift_secs)
                                .suffix(" s")
                                .speed(1.0)
                                .range(-86_400..=86_400),
                        );
                        ui.label(secondary(
                            "Seconds added to photo times. Use a positive value if the camera clock ran behind.",
                        ));
                    });
                    ui.end_row();

                    ui.label("When tagging");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut self.keep_backup, "Keep a backup copy of each photo");
                        ui.label(secondary("Saved next to the photo as “name.ext_original”."));
                        ui.checkbox(&mut self.replace_gps, "Replace existing GPS location");
                        ui.label(secondary("Off: photos that already have a location are left alone."));
                    });
                    ui.end_row();
                });

                if let Some(c) = self.coverage() {
                    ui.separator();
                    let all = c.inside == c.with_time;
                    let dark = ui.visuals().dark_mode;
                    let (icon, color) = if all {
                        ("✓", if dark { Color32::from_rgb(48, 209, 88) } else { Color32::from_rgb(36, 138, 61) })
                    } else if c.inside == 0 {
                        ("⚠", if dark { Color32::from_rgb(255, 69, 58) } else { Color32::from_rgb(215, 0, 21) })
                    } else {
                        ("⚠", if dark { Color32::from_rgb(255, 159, 10) } else { Color32::from_rgb(201, 52, 0) })
                    };
                    ui.label(
                        RichText::new(format!(
                            "{icon} {} of {} photos fall within the track",
                            c.inside, c.with_time
                        ))
                        .color(color)
                        .strong(),
                    );
                    ui.label(secondary(format!(
                        "Photos taken {} – {} (camera time). Track covers {} – {} UTC.",
                        tagger::format_epoch(c.photo_span.0),
                        tagger::format_epoch(c.photo_span.1),
                        tagger::format_epoch(c.track_span.0),
                        tagger::format_epoch(c.track_span.1),
                    )));
                    if !all {
                        ui.label(secondary(
                            "If this looks wrong, check the camera time zone and clock correction.",
                        ));
                    }
                }
            });
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new("Details").default_open(false).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("log").max_height(140.0).stick_to_bottom(true).show(
                ui,
                |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                },
            );
        });
    }
}

fn status_label(ui: &mut egui::Ui, row: &PhotoRow) {
    let dark = ui.visuals().dark_mode;
    let green = if dark { Color32::from_rgb(48, 209, 88) } else { Color32::from_rgb(36, 138, 61) };
    let orange = if dark { Color32::from_rgb(255, 159, 10) } else { Color32::from_rgb(201, 52, 0) };
    let red = if dark { Color32::from_rgb(255, 69, 58) } else { Color32::from_rgb(215, 0, 21) };
    let (text, color, tip): (&str, Option<Color32>, String) = match &row.outcome {
        Some(Outcome::Tagged) => ("✓ Tagged", Some(green), String::new()),
        Some(Outcome::AlreadyHasGps) => ("Already has GPS", None, "Left unchanged".into()),
        Some(Outcome::NoMatch(m)) => ("⚠ No matching track point", Some(orange), m.clone()),
        Some(Outcome::Failed(m)) => ("⚠ Failed", Some(red), m.clone()),
        None if row.info.local_time.is_none() => ("No capture time", Some(orange), String::new()),
        None if row.info.has_gps => ("Has GPS", None, String::new()),
        None => ("", None, String::new()),
    };
    if text.is_empty() {
        return;
    }
    let mut label = secondary(text);
    if let Some(c) = color {
        label = label.color(c);
    }
    let r = ui.label(label);
    if !tip.is_empty() {
        r.on_hover_text(tip);
    }
}
