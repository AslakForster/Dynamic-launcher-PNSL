/*
PNSL (Permissive Non-Sale License)
Copyright © 2025 Christopher Forster

Permission to use, copy, modify, and/or distribute this software for any purpose
without charging a fee specifically for the software itself is hereby granted,
provided that the above copyright notice and this permission notice appear in all copies.
You may not introduce more restrictions.

THIS SOFTWARE IS PROVIDED ‘AS IS’ AND WITHOUT ANY WARRANTIES. IN NO EVENT SHALL THE
COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT OR INDIRECT LOSSES OR DAMAGES
ARISING FROM THE USE OF THIS SOFTWARE.

Any binary or combined work that directly incorporates PNSL-licensed code is considered
a derivative work and must comply with this license. However, merely calling, linking to,
or interacting with unmodified, separately distributed PNSL-licensed code does not, by itself,
constitute a derivative work.
*/

use std::env;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use eframe::egui;
use webbrowser;

/* ============================================================================
   Constants & license text
   ============================================================================ */

const HELP_TEXT: &str =
"Dynamic launcher (Rust/egui, Linux) 1.0.0 — PNSL licensed.\n\
\n\
PNSL (Permissive Non-Sale License)\n\
Copyright © 2025 Christopher Forster\n\
\n\
1. Permission\n\
Permission to use, copy, modify, and distribute this software for any purpose, without charging a fee specifically for the software itself, is hereby granted, provided that the above copyright notice and this permission notice appear in all copies. You may not introduce more restrictions.\n\
\n\
2. No warranty\n\
THIS SOFTWARE IS PROVIDED 'AS IS' AND WITHOUT ANY WARRANTIES. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT OR INDIRECT LOSSES OR DAMAGES ARISING FROM THE USE OF THIS SOFTWARE.\n\
\n\
3. Derivative works\n\
Any binary or combined work that directly incorporates PNSL-licensed code is considered a derivative work and must comply with this license. However, merely calling, linking to, or interacting with unmodified, separately distributed PNSL-licensed code does not, by itself, constitute a derivative work.\n\
\n\
Linux notes:\n\
On Fedora GNOME, the Full Zoom and Lens Zoom tiles control the built-in\n\
Accessibility \"Zoom\" feature (GNOME screen magnifier) via GSettings.\n\
This means your normal GNOME Zoom keyboard shortcuts and Settings panel\n\
stay in sync with Dynamic Launcher.\n\
";

const TILE_COUNT: usize = 5;

// Layout-ish equivalents of the original WinAPI version
const TILE_MIN_W: f32 = 160.0;
const TILE_MIN_H: f32 = 72.0;

// Poll interval for magnifier state (ms)
const MAG_CHECK_MS: u64 = 1500;

// GNOME schemas
const APP_SCHEMA: &str = "org.gnome.desktop.a11y.applications";
const MAG_SCHEMA: &str = "org.gnome.desktop.a11y.magnifier";

#[derive(Copy, Clone, PartialEq, Eq)]
enum ClockMode {
    Time,
    Date,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum MagnifierBackend {
    Gnome,
    None,
}

/* ============================================================================
   Font config (Linux: keep egui defaults, but hook exists if you want Segoe later)
   ============================================================================ */

fn configure_default_font(ctx: &egui::Context) {
    use egui::FontDefinitions;

    let fonts = FontDefinitions::default();
    ctx.set_fonts(fonts);
}

/* ============================================================================
   Linux helpers
   ============================================================================ */

/// Try to shut down the system on a systemd-based Linux (like Fedora).
/// This uses `systemctl poweroff` as a simple approximation of the
/// original Windows shutdown behavior.
fn shutdown_pc_now() {
    let _ = Command::new("systemctl")
        .args(["poweroff"])
        .spawn();
}

/* -------- GNOME Accessibility Zoom helpers (Magnifier equivalent) --------- */

fn is_gsettings_available() -> bool {
    // Just try to run "gsettings --version"; if it works, we assume it's present.
    Command::new("gsettings")
        .arg("--version")
        .output()
        .is_ok()
}

/// Determine if the current desktop is GNOME-ish based on env vars.
fn detect_magnifier_backend() -> MagnifierBackend {
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| env::var("DESKTOP_SESSION"))
        .unwrap_or_default()
        .to_lowercase();

    // Most GNOME sessions include "gnome" somewhere, e.g. "ubuntu:GNOME".
    let is_gnome_like = desktop.contains("gnome");

    if is_gnome_like && is_gsettings_available() {
        MagnifierBackend::Gnome
    } else {
        MagnifierBackend::None
    }
}

/// Run `gsettings get` and return stdout as trimmed String on success.
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run `gsettings set` and return whether it succeeded.
/// Logs to stderr on failure for easier debugging.
fn gsettings_set(schema: &str, key: &str, value: &str) -> bool {
    match Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
    {
        Ok(status) => {
            if !status.success() {
                eprintln!(
                    "gsettings set {} {} {} failed with status: {:?}",
                    schema,
                    key,
                    value,
                    status.code()
                );
            }
            status.success()
        }
        Err(e) => {
            eprintln!(
                "gsettings set {} {} {} failed to spawn: {}",
                schema, key, value, e
            );
            false
        }
    }
}

/// Check if GNOME Accessibility Zoom is enabled.
fn gnome_magnifier_enabled() -> bool {
    if !is_gsettings_available() {
        return false;
    }
    matches!(
        gsettings_get(APP_SCHEMA, "screen-magnifier-enabled").as_deref(),
        Some("true")
    )
}

/// Returns (fullscreen_active, lens_active) based on GNOME’s Zoom settings.
fn gnome_magnifier_mode() -> (bool, bool) {
    if !gnome_magnifier_enabled() {
        return (false, false);
    }

    let pos = gsettings_get(MAG_SCHEMA, "screen-position").unwrap_or_default();
    let lens = gsettings_get(MAG_SCHEMA, "lens-mode").unwrap_or_default();

    let lens_active = lens.trim() == "true";
    let fullscreen_active = !lens_active && pos.contains("full-screen");

    (fullscreen_active, lens_active)
}

/// General-purpose desktop zoom: full-screen, moderate zoom,
/// calm tracking. Good for general reading/accessibility.
fn gnome_launch_magnifier_fullscreen_175() {
    if !is_gsettings_available() {
        return;
    }

    // Moderate zoom level
    gsettings_set(MAG_SCHEMA, "mag-factor", "1.75");

    // Full-screen zoom
    gsettings_set(MAG_SCHEMA, "screen-position", "'full-screen'");

    // Normal (non-lens) mode
    gsettings_set(MAG_SCHEMA, "lens-mode", "false");

    // Tracking behaviour: fairly calm and predictable
    gsettings_set(MAG_SCHEMA, "mouse-tracking", "'proportional'");
    gsettings_set(MAG_SCHEMA, "focus-tracking", "'proportional'");
    gsettings_set(MAG_SCHEMA, "caret-tracking", "'centered'");

    // No crosshair, normal colours
    gsettings_set(MAG_SCHEMA, "show-cross-hairs", "false");
    gsettings_set(MAG_SCHEMA, "invert-lightness", "false");

    // Allow scroll-at-edges for smoother navigation
    gsettings_set(MAG_SCHEMA, "scroll-at-edges", "true");

    // This actually turns Accessibility → Zoom on
    gsettings_set(APP_SCHEMA, "screen-magnifier-enabled", "true");
}

/// Reading / inspection mode:
/// - Only top half of the screen is magnified
/// - Stronger zoom
/// - Lens behaviour that follows the mouse
/// - Clean look (no crosshair)
fn gnome_launch_magnifier_lens_175() {
    if !is_gsettings_available() {
        return;
    }

    // Stronger zoom for inspection
    gsettings_set(MAG_SCHEMA, "mag-factor", "2.5");

    // Top half of the screen acts as the magnified area
    gsettings_set(MAG_SCHEMA, "screen-position", "'top-half'");

    // Lens mode: magnified view follows the mouse like a magnifying glass
    gsettings_set(MAG_SCHEMA, "lens-mode", "true");

    // Aggressive tracking: keep mouse / focus centred in the lens area
    gsettings_set(MAG_SCHEMA, "mouse-tracking", "'centered'");
    gsettings_set(MAG_SCHEMA, "focus-tracking", "'centered'");
    gsettings_set(MAG_SCHEMA, "caret-tracking", "'centered'");

    // No crosshair (this removes the red lines)
    gsettings_set(MAG_SCHEMA, "show-cross-hairs", "false");

    // In this mode, let the lens move rather than scrolling the whole view at edges
    gsettings_set(MAG_SCHEMA, "scroll-at-edges", "false");

    // Keep colours normal (you can flip this to "true" for a high-contrast mode)
    gsettings_set(MAG_SCHEMA, "invert-lightness", "false");

    // Turn Zoom on
    gsettings_set(APP_SCHEMA, "screen-magnifier-enabled", "true");
}

/// Best-effort attempt to disable GNOME Accessibility Zoom asynchronously.
fn gnome_close_magnifier_async() {
    if !is_gsettings_available() {
        return;
    }

    thread::spawn(|| {
        gsettings_set(APP_SCHEMA, "screen-magnifier-enabled", "false");
    });
}

/* ============================================================================
   Clock + title helpers (cross-platform via chrono)
   ============================================================================ */

fn get_time_strings(mode: ClockMode) -> (String, String) {
    let now = Local::now();

    match mode {
        ClockMode::Time => {
            let clock = now.format("%H:%M:%S").to_string();
            let title = format!(
                "Dynamic Launcher [{}] — PNSL licensed (press F1 for more information)",
                now.format("%Y-%m-%d")
            );
            (clock, title)
        }
        ClockMode::Date => {
            let clock = now.format("%Y-%m-%d").to_string();
            let title = format!(
                "Dynamic Launcher [{}] — PNSL licensed (press F1 for more information)",
                now.format("%H:%M:%S")
            );
            (clock, title)
        }
    }
}

/* ============================================================================
   Egui app state
   ============================================================================ */

struct DynamicLauncherApp {
    clock_mode: ClockMode,
    clock_tile_text: String,
    window_title: String,

    magnifier_backend: MagnifierBackend,
    zoom_full_active: bool,
    zoom_lens_active: bool,

    focus_index: usize,
    show_help: bool,
    show_shutdown_confirm: bool,
    help_text: String,

    last_mag_check: Instant,
}

impl DynamicLauncherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        configure_default_font(&cc.egui_ctx);

        let help_text = HELP_TEXT.to_string();

        let (clock_text, title) = get_time_strings(ClockMode::Time);
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));

        let backend = detect_magnifier_backend();
        let (zf, zl) = match backend {
            MagnifierBackend::Gnome => gnome_magnifier_mode(),
            MagnifierBackend::None => (false, false),
        };

        Self {
            clock_mode: ClockMode::Time,
            clock_tile_text: clock_text,
            window_title: title,
            magnifier_backend: backend,
            zoom_full_active: zf,
            zoom_lens_active: zl,
            focus_index: 0,
            show_help: false,
            show_shutdown_confirm: false,
            help_text,
            last_mag_check: Instant::now(),
        }
    }

    fn tile_label(&self, idx: usize) -> String {
        match idx {
            0 => "Halt".to_string(),
            1 => "🖥Full Zoom".to_string(),
            2 => "🗖Lens Zoom".to_string(),
            3 => "🌐WWW".to_string(),
            4 => self.clock_tile_text.clone(),
            _ => String::new(),
        }
    }

    fn toggle_full_zoom(&mut self) {
        match self.magnifier_backend {
            MagnifierBackend::Gnome => {
                if self.zoom_full_active {
                    gnome_close_magnifier_async();
                    self.zoom_full_active = false;
                    self.zoom_lens_active = false;
                } else {
                    gnome_close_magnifier_async();
                    gnome_launch_magnifier_fullscreen_175();
                    self.zoom_full_active = true;
                    self.zoom_lens_active = false;
                }
            }
            MagnifierBackend::None => {
                eprintln!("Magnifier: no supported backend detected.");
                self.zoom_full_active = false;
                self.zoom_lens_active = false;
            }
        }
    }

    fn toggle_lens_zoom(&mut self) {
        match self.magnifier_backend {
            MagnifierBackend::Gnome => {
                if self.zoom_lens_active {
                    gnome_close_magnifier_async();
                    self.zoom_lens_active = false;
                    self.zoom_full_active = false;
                } else {
                    gnome_close_magnifier_async();
                    gnome_launch_magnifier_lens_175();
                    self.zoom_lens_active = true;
                    self.zoom_full_active = false;
                }
            }
            MagnifierBackend::None => {
                eprintln!("Magnifier: no supported backend detected.");
                self.zoom_lens_active = false;
                self.zoom_full_active = false;
            }
        }
    }

    fn close_magnifier(&self) {
        match self.magnifier_backend {
            MagnifierBackend::Gnome => gnome_close_magnifier_async(),
            MagnifierBackend::None => {}
        }
    }

    fn handle_action(&mut self, idx: usize) {
        match idx {
            0 => {
                // Halt tile -> show one-line shutdown confirmation
                self.show_shutdown_confirm = true;
            }
            1 => self.toggle_full_zoom(),
            2 => self.toggle_lens_zoom(),
            3 => {
                let _ = webbrowser::open("https://duckduckgo.com/");
            }
            4 => {
                self.clock_mode = match self.clock_mode {
                    ClockMode::Time => ClockMode::Date,
                    ClockMode::Date => ClockMode::Time,
                };
            }
            _ => {}
        }
    }
}

/* ============================================================================
   Dynamic tile text sizing ("fit to control", recomputed every frame)
   ============================================================================ */

fn compute_tile_font_size(ui: &egui::Ui, label: &str, size: egui::Vec2) -> f32 {
    let min_size = 14.0_f32;
    let max_size = (size.y * 0.70).max(min_size); // fairly big for tall tiles
    let mut best = min_size;
    let mut lo = min_size;
    let mut hi = max_size;

    let color = ui.visuals().widgets.active.text_color();

    // Use fonts_mut so we can call layout_no_wrap (which needs &mut self)
    ui.ctx().fonts_mut(|fonts| {
        for _ in 0..10 {
            let mid = 0.5 * (lo + hi);
            let font_id = egui::FontId::proportional(mid);
            let galley = fonts.layout_no_wrap(label.to_owned(), font_id, color);
            let text_size = galley.size();

            let fits_width = text_size.x <= size.x * 0.90;
            let fits_height = text_size.y <= size.y * 0.80;
            let fits = fits_width && fits_height;

            if fits {
                best = mid;
                lo = mid;
            } else {
                hi = mid;
            }

            if (hi - lo) <= 0.5 {
                break;
            }
        }
    });

    best
}

/* ============================================================================
   Widgets
   ============================================================================ */

fn tile_button(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    label: &str,
    is_active: bool,
    is_focused: bool,
) -> egui::Response {
    // Slightly darker background
    let bg = if is_active {
        egui::Color32::from_rgb(22, 80, 40) // darker green
    } else {
        egui::Color32::from_rgb(35, 35, 38) // darker gray
    };

    let stroke = egui::Stroke::new(
        if is_focused { 2.0 } else { 1.0 },
        if is_active {
            egui::Color32::from_rgb(120, 200, 120)
        } else {
            egui::Color32::from_rgb(80, 80, 80)
        },
    );

    // Fully dynamic font size: measure text and choose the largest size
    // that fits this tile *in the current frame*. Resizing the window
    // changes `size`, so this recomputes and scales the text.
    let font_size = compute_tile_font_size(ui, label, size);
    let text = egui::RichText::new(label).size(font_size);

    let button = egui::Button::new(text)
        .min_size(size)
        .fill(bg)
        .stroke(stroke);

    ui.add(button)
}

/* ============================================================================
   egui::App impl
   ============================================================================ */

impl eframe::App for DynamicLauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Update clock + window title
        let (clock_text, title) = get_time_strings(self.clock_mode);
        self.clock_tile_text = clock_text;
        if self.window_title != title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Poll magnifier state periodically (GNOME Accessibility Zoom backend).
        if self.last_mag_check.elapsed() >= Duration::from_millis(MAG_CHECK_MS) {
            match self.magnifier_backend {
                MagnifierBackend::Gnome => {
                    let (full, lens) = gnome_magnifier_mode();
                    self.zoom_full_active = full;
                    self.zoom_lens_active = lens;
                }
                MagnifierBackend::None => {}
            }
            self.last_mag_check = Instant::now();
        }

        // Keyboard input (navigation + shortcuts)
        let tab_or_right = ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::Tab)
        });
        let left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
        let enter_or_space = ctx.input(|i| {
            i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
        });
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let f1 = ctx.input(|i| i.key_pressed(egui::Key::F1));

        if tab_or_right {
            self.focus_index = (self.focus_index + 1) % TILE_COUNT;
        }
        if left {
            if self.focus_index == 0 {
                self.focus_index = TILE_COUNT - 1;
            } else {
                self.focus_index -= 1;
            }
        }

        if enter_or_space {
            // If shutdown dialog is open, Enter acts as Yes (like a default button)
            if self.show_shutdown_confirm {
                shutdown_pc_now();
            } else {
                let idx = self.focus_index;
                self.handle_action(idx);
            }
        }

        if esc {
            if self.show_shutdown_confirm {
                self.show_shutdown_confirm = false;
            } else if self.show_help {
                self.show_help = false;
            } else {
                // ESC behaves like the WinAPI version: try to close Magnifier.
                self.close_magnifier();
            }
        }

        if f1 {
            self.show_help = !self.show_help;
        }

        // Keep repainting often enough to update the clock smoothly.
        ctx.request_repaint_after(Duration::from_millis(200));

        // Compute global dynamic font sizes (for dialogs & help) from window height.
        let (dialog_font_size, help_font_size) = {
            let rect = ctx.input(|i| i.content_rect());
            let h = rect.height().max(200.0);
            let dialog = (h * 0.035).clamp(16.0, 30.0);
            let help = (h * 0.020).clamp(12.0, 20.0);
            (dialog, help)
        };

        // Main tile strip
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default().fill(egui::Color32::from_rgb(24, 24, 24)),
            )
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(
                    TILE_COUNT as f32 * TILE_MIN_W * 0.9,
                    TILE_MIN_H,
                ));
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let available = ui.available_size();
                        let spacing = ui.spacing().item_spacing.x;
                        let total_spacing = spacing * ((TILE_COUNT - 1) as f32);

                        let tile_width = ((available.x - total_spacing)
                            / TILE_COUNT as f32)
                            .max(TILE_MIN_W * 0.5);
                        let tile_height = available.y.max(TILE_MIN_H);
                        let tile_size = egui::vec2(tile_width, tile_height);

                        for idx in 0..TILE_COUNT {
                            let label = self.tile_label(idx);
                            let is_active = match idx {
                                1 => self.zoom_full_active,
                                2 => self.zoom_lens_active,
                                _ => false,
                            };
                            let is_focused = self.focus_index == idx;

                            let resp =
                                tile_button(ui, tile_size, &label, is_active, is_focused);

                            if resp.hovered() {
                                self.focus_index = idx;
                            }
                            if resp.clicked() {
                                self.handle_action(idx);
                            }
                        }
                    });
                });
            });

        // F1 help window: white text on black background, dynamic font size
        if self.show_help {
            egui::Window::new("About & License")
                .open(&mut self.show_help)
                .resizable(true)
                .vscroll(false)
                .hscroll(false)
                .frame(egui::Frame::default().fill(egui::Color32::BLACK))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&self.help_text)
                                    .monospace()
                                    .size(help_font_size)
                                    .color(egui::Color32::WHITE),
                            );
                        });
                });
        }

        // Shutdown confirmation window: compact, one-line UI "Shutdown computer [ Yes ] [ No ]"
        if self.show_shutdown_confirm {
            let mut do_shutdown = false;
            let mut close_dialog = false;

            egui::Window::new("Shutdown")
                .title_bar(false) // hide the extra "Shutdown" text bar
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size(egui::vec2(420.0, 90.0)) // compact dialog size
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    let font_size = dialog_font_size;

                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.horizontal_centered(|ui| {
                            ui.label(
                                egui::RichText::new("Shutdown computer")
                                    .size(font_size)
                                    .color(egui::Color32::WHITE),
                            );

                            ui.add_space(16.0);

                            let yes = ui.add_sized(
                                egui::vec2(90.0, 32.0),
                                egui::Button::new(
                                    egui::RichText::new("Yes").size(font_size),
                                ),
                            );

                            ui.add_space(8.0);

                            let no = ui.add_sized(
                                egui::vec2(90.0, 32.0),
                                egui::Button::new(
                                    egui::RichText::new("No").size(font_size),
                                ),
                            );

                            if yes.clicked() {
                                do_shutdown = true;
                            }
                            if no.clicked() {
                                close_dialog = true;
                            }
                        });
                        ui.add_space(6.0);
                    });
                });

            if do_shutdown {
                shutdown_pc_now();
            }
            if close_dialog {
                self.show_shutdown_confirm = false;
            }
        }
    }
}

/* ============================================================================
   main()
   ============================================================================ */

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Wide strip for 5 tiles, compact vertically
            .with_inner_size([900.0, 79.0])
            .with_min_inner_size([600.0, TILE_MIN_H])
            .with_resizable(true)
            .with_title("Dynamic Launcher"),
        ..Default::default()
    };

    eframe::run_native(
        "Dynamic Launcher",
        native_options,
        Box::new(|cc| Ok(Box::new(DynamicLauncherApp::new(cc)))),
    )
}

