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
use std::process::{Child, Command};
use std::time::Duration;

use chrono::Local;
use eframe::egui;
use webbrowser;

/* ============================================================================
   Constants & help text
   ============================================================================ */

const HELP_TEXT: &str =
"Dynamic launcher (Rust/egui, OpenBSD) 1.0.0 — PNSL licensed.\n\
\n\
PNSL (Permissive Non-Sale License)\n\
Copyright © 2025 Christopher Forster\n\
\n\
1. Permission\n\
Permission to use, copy, modify, and distribute this software for any purpose, without charging a fee\n\
specifically for the software itself, is hereby granted, provided that the above copyright notice and\n\
this permission notice appear in all copies. You may not introduce more restrictions.\n\
\n\
2. No warranty\n\
THIS SOFTWARE IS PROVIDED 'AS IS' AND WITHOUT ANY WARRANTIES. IN NO EVENT SHALL THE COPYRIGHT OWNER OR\n\
CONTRIBUTORS BE LIABLE FOR ANY DIRECT OR INDIRECT LOSSES OR DAMAGES ARISING FROM THE USE OF THIS SOFTWARE.\n\
\n\
3. Derivative works\n\
Any binary or combined work that directly incorporates PNSL-licensed code is considered a derivative work\n\
and must comply with this license. However, merely calling, linking to, or interacting with unmodified,\n\
separately distributed PNSL-licensed code does not, by itself, constitute a derivative work.\n\
\n\
OpenBSD notes:\n\
  • Halt: asks for confirmation, then tries \"doas /sbin/shutdown -p now\", and if that fails, \"/sbin/shutdown -p now\".\n\
    Any error messages from doas/shutdown are shown in a system message box.\n\
  • CMD: prefers Sakura (if installed). If Sakura is not found, falls back to your default terminal\n\
    ($TERMINAL) or common terminals like xterm, konsole, etc.\n\
  • top: opens a terminal running \"top\" (same terminal preference as CMD).\n\
  • WWW: opens DuckDuckGo using the system web browser.\n\
  • Clock: click to toggle between time and date.\n\
\n\
Keyboard:\n\
  • Left / Right / Tab: move focus between tiles.\n\
  • Enter / Space: activate focused tile.\n\
  • Esc: closes dialogs/help/system messages.\n\
  • F1: show/hide this help.\n\
";

const TILE_COUNT: usize = 5;
const TILE_MIN_W: f32 = 160.0;
const TILE_MIN_H: f32 = 72.0;

#[derive(Copy, Clone, PartialEq, Eq)]
enum ClockMode {
    Time,
    Date,
}

/* ============================================================================
   Fonts (keep defaults, but hook exists if you want custom fonts)
   ============================================================================ */

fn configure_default_font(ctx: &egui::Context) {
    let fonts = egui::FontDefinitions::default();
    ctx.set_fonts(fonts);
}

/* ============================================================================
   OpenBSD helpers
   ============================================================================ */

/// Attempt to shut down the system.
///
/// Strategy:
///   1. Try `doas /sbin/shutdown -p now` (typical non-root usage).
///   2. If that fails, try `/sbin/shutdown -p now` directly (for root).
///   3. If both fail, return a combined error string so the GUI can show it.
///
/// This way we do not *assume* whether we are root or not; we simply
/// attempt both and report whatever went wrong.
fn shutdown_pc_now() -> Result<(), String> {
    let mut errors = Vec::new();

    // 1) Try via doas
    match Command::new("doas")
        .args(["/sbin/shutdown", "-p", "now"])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                return Ok(());
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    errors.push("doas failed to run /sbin/shutdown -p now".to_string());
                } else {
                    errors.push(format!("doas error: {stderr}"));
                }
            }
        }
        Err(e) => {
            errors.push(format!("Failed to run doas: {e}"));
        }
    }

    // 2) Try direct /sbin/shutdown (works only if we actually have privileges)
    match Command::new("/sbin/shutdown")
        .args(["-p", "now"])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                return Ok(());
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    errors.push("Direct /sbin/shutdown -p now failed".to_string());
                } else {
                    errors.push(format!("shutdown error: {stderr}"));
                }
            }
        }
        Err(e) => {
            errors.push(format!("Failed to run /sbin/shutdown: {e}"));
        }
    }

    if errors.is_empty() {
        Err("Shutdown failed for unknown reasons.".to_string())
    } else {
        Err(errors.join("\n"))
    }
}

/// Spawn a terminal emulator with `-e <command...>`.
fn spawn_term(term: &str, cmd: &[&str]) -> std::io::Result<Child> {
    let mut c = Command::new(term);
    c.arg("-e");
    for &part in cmd {
        c.arg(part);
    }
    c.spawn()
}

/// Try to run `cmd` inside a terminal emulator.
///
/// Preference order:
///   1. Sakura (if installed).
///   2. $TERMINAL (if set and not sakura).
///   3. Common terminals: xterm, konsole, xfce4-terminal, gnome-terminal.
///   4. As a last resort, run the command directly (assuming we're already in a terminal).
fn spawn_in_terminal(cmd: &[&str]) {
    let term_env = env::var("TERMINAL").ok();

    // 1) Prefer sakura explicitly.
    if spawn_term("sakura", cmd).is_ok() {
        return;
    }

    // 2) Then $TERMINAL, if set and not sakura (to avoid double-trying).
    if let Some(ref term) = term_env {
        if term != "sakura" {
            if spawn_term(term, cmd).is_ok() {
                return;
            }
        }
    }

    // 3) Fallback list of common terminals, skipping one we already tried via $TERMINAL.
    for term in ["xterm", "konsole", "xfce4-terminal", "gnome-terminal"] {
        if term_env.as_deref() == Some(term) {
            continue;
        }
        if spawn_term(term, cmd).is_ok() {
            return;
        }
    }

    // 4) Fallback: run cmd directly (e.g. if we're already running inside a terminal).
    if let Some((first, rest)) = cmd.split_first() {
        let _ = Command::new(first).args(rest).spawn();
    }
}

/// Open the default command shell in a terminal.
fn open_cmd_terminal() {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/ksh".to_string());
    let cmd = [shell.as_str()];
    spawn_in_terminal(&cmd);
}

/// Open `top` in a terminal.
fn open_top_terminal() {
    let cmd = ["top"];
    spawn_in_terminal(&cmd);
}

/* ============================================================================
   Clock + window title helpers
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
   App state
   ============================================================================ */

struct DynamicLauncherApp {
    clock_mode: ClockMode,
    clock_tile_text: String,
    window_title: String,

    focus_index: usize,
    show_help: bool,
    show_shutdown_confirm: bool,
    help_text: String,

    // A simple system message box for errors from doas/shutdown etc.
    system_message: Option<String>,
}

impl DynamicLauncherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        configure_default_font(&cc.egui_ctx);

        let help_text = HELP_TEXT.to_string();

        let (clock_text, title) = get_time_strings(ClockMode::Time);
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));

        Self {
            clock_mode: ClockMode::Time,
            clock_tile_text: clock_text,
            window_title: title,
            focus_index: 0,
            show_help: false,
            show_shutdown_confirm: false,
            help_text,
            system_message: None,
        }
    }

    fn tile_label(&self, idx: usize) -> String {
        match idx {
            0 => "Halt".to_string(),
            1 => "🖥 CMD".to_string(),
            2 => "📊 top".to_string(),
            3 => "🌐 WWW".to_string(),
            4 => self.clock_tile_text.clone(),
            _ => String::new(),
        }
    }

    fn handle_action(&mut self, idx: usize) {
        match idx {
            // Halt: ask for confirmation
            0 => {
                self.show_shutdown_confirm = true;
            }
            // CMD: open terminal with $SHELL (preferring Sakura)
            1 => {
                open_cmd_terminal();
            }
            // top: open terminal running top (preferring Sakura)
            2 => {
                open_top_terminal();
            }
            // WWW: open DuckDuckGo
            3 => {
                let _ = webbrowser::open("https://duckduckgo.com/");
            }
            // Clock: toggle Time/Date
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

    // Measure text via egui's font system using a small binary search
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
   Tile widget
   ============================================================================ */

fn tile_button(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    label: &str,
    is_active: bool,
    is_focused: bool,
) -> egui::Response {
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

    // Fully dynamic font size: recomputed every frame based on current tile size.
    let font_size = compute_tile_font_size(ui, label, size);
    let text = egui::RichText::new(label).size(font_size);

    let button = egui::Button::new(text)
        .min_size(size)
        .fill(bg)
        .stroke(stroke);

    ui.add(button)
}

/* ============================================================================
   eframe::App implementation
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

        // Keyboard navigation and shortcuts
        let tab_or_right = ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::Tab)
        });
        let left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
        let enter_or_space =
            ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
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
            if self.show_shutdown_confirm {
                if let Err(err) = shutdown_pc_now() {
                    // Show message box to the user
                    self.system_message = Some(err);
                    self.show_shutdown_confirm = false;
                }
            } else {
                let idx = self.focus_index;
                self.handle_action(idx);
            }
        }

        if esc {
            if self.system_message.is_some() {
                // Close system message first
                self.system_message = None;
            } else if self.show_shutdown_confirm {
                self.show_shutdown_confirm = false;
            } else if self.show_help {
                self.show_help = false;
            }
        }

        if f1 {
            self.show_help = !self.show_help;
        }

        // Smooth clock updates
        ctx.request_repaint_after(Duration::from_millis(200));

        // Global dynamic font sizes for dialogs & help, based on current window height
        let (dialog_font_size, help_font_size) = {
            let rect = ctx.input(|i| i.content_rect());
            let h = rect.height().max(200.0);
            let dialog = (h * 0.035).clamp(16.0, 30.0);
            let help = (h * 0.020).clamp(12.0, 20.0);
            (dialog, help)
        };

        // Main tile strip
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(24, 24, 24)))
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

                        let tile_width = ((available.x - total_spacing) / TILE_COUNT as f32)
                            .max(TILE_MIN_W * 0.5);
                        let tile_height = available.y.max(TILE_MIN_H);
                        let tile_size = egui::vec2(tile_width, tile_height);

                        for idx in 0..TILE_COUNT {
                            let label = self.tile_label(idx);
                            let is_focused = self.focus_index == idx;

                            // We don't track a running state for CMD/top, so is_active = false
                            let resp = tile_button(ui, tile_size, &label, false, is_focused);

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

        // F1 help window (monospace, dynamic font)
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

        // Shutdown confirmation window: compact one-line UI
        if self.show_shutdown_confirm {
            let mut do_shutdown = false;
            let mut close_dialog = false;

            egui::Window::new("Shutdown")
                .title_bar(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size(egui::vec2(420.0, 90.0))
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
                if let Err(err) = shutdown_pc_now() {
                    self.system_message = Some(err);
                    self.show_shutdown_confirm = false;
                }
            }
            if close_dialog {
                self.show_shutdown_confirm = false;
            }
        }

        // System message box: shows errors from doas/shutdown attempts, etc.
        if let Some(ref msg) = self.system_message {
            let mut close_msg = false;

            egui::Window::new("System message")
                .title_bar(true)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size(egui::vec2(520.0, 140.0))
                .show(ctx, |ui| {
                    let font_size = dialog_font_size;

                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(msg)
                                .size(font_size)
                                .color(egui::Color32::WHITE),
                        );
                        ui.add_space(12.0);
                        if ui
                            .add_sized(
                                egui::vec2(80.0, 30.0),
                                egui::Button::new(
                                    egui::RichText::new("OK").size(font_size),
                                ),
                            )
                            .clicked()
                        {
                            close_msg = true;
                        }
                        ui.add_space(6.0);
                    });
                });

            if close_msg {
                self.system_message = None;
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
