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


#![windows_subsystem = "windows"]

use std::{
    ffi::OsStr,
    fs,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    ptr::null_mut,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use webbrowser;

use winapi::{
    ctypes::c_int,
    shared::minwindef::{HKEY, UINT},
    um::{
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        minwinbase::SYSTEMTIME,
        processthreadsapi::{OpenProcess, TerminateProcess},
        shellapi::ShellExecuteW,
        synchapi::{Sleep, WaitForSingleObject},
        sysinfoapi::GetLocalTime,
        tlhelp32::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        winbase::lstrcmpiW,
        winnt::{KEY_READ, KEY_SET_VALUE, PROCESS_TERMINATE, REG_DWORD, SYNCHRONIZE},
        winreg::{
            HKEY_CURRENT_USER, RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW,
            RegSetValueExW,
        },
        winuser::{
            INPUT, SendInput, INPUT_KEYBOARD, KEYEVENTF_KEYUP, SW_SHOWNORMAL, VK_ESCAPE, VK_LWIN,
        },
    },
};

/* ============================================================================
   Constants & license text
   ============================================================================ */

const HELP_TEXT: &str =
"Dynamic launcher (Rust/egui) 1.0.0 — PNSL licensed.\r\n\
\r\n\
PNSL (Permissive Non-Sale License)\r\n\
Copyright © 2025 Christopher Forster\r\n\
\r\n\
1. Permission\r\n\
Permission to use, copy, modify, and distribute this software for any purpose, without charging a fee specifically for the software itself, is hereby granted, provided that the above copyright notice and this permission notice appear in all copies. You may not introduce more restrictions.\r\n\
\r\n\
2. No warranty\r\n\
THIS SOFTWARE IS PROVIDED 'AS IS' AND WITHOUT ANY WARRANTIES. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT OR INDIRECT LOSSES OR DAMAGES ARISING FROM THE USE OF THIS SOFTWARE.\r\n\
\r\n\
3. Derivative works\r\n\
Any binary or combined work that directly incorporates PNSL-licensed code is considered a derivative work and must comply with this license. However, merely calling, linking to, or interacting with unmodified, separately distributed PNSL-licensed code does not, by itself, constitute a derivative work.\r\n\
\r\n";

const TILE_COUNT: usize = 5;

const MAG_SAFEWAIT_MS: u32 = 800;
const MAG_FORCEWAIT_MS: u32 = 2000;
const MAG_CHECK_MS: u64 = 1500;
const WAIT_TIMEOUT: u32 = 0x0000_0102; // Win32 WAIT_TIMEOUT

// Layout-ish equivalents of the original WinAPI version
const TILE_MIN_W: f32 = 160.0;
const TILE_MIN_H: f32 = 72.0;

#[derive(Copy, Clone, PartialEq, Eq)]
enum ClockMode {
    Time,
    Date,
}

static G_MAG_CLOSE_BUSY: AtomicBool = AtomicBool::new(false);

/* ============================================================================
   Helpers
   ============================================================================ */

fn wstr(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn shutdown_pc_now() {
    unsafe {
        let verb = wstr("open");
        let exe = wstr("shutdown");
        let args = wstr("-s -t 0");
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            args.as_ptr(),
            null_mut(),
            SW_SHOWNORMAL,
        );
    }
}

fn configure_segoe_font(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};

    // Start from defaults so we keep egui's fallbacks (emoji, etc.).
    let mut fonts = FontDefinitions::default();

    // Try to load Segoe UI from the standard Windows path.
    if let Ok(data) = fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts.font_data.insert(
            "segoe-ui".to_owned(),
            FontData::from_owned(data).into(), // Arc<FontData>
        );

        // Make Segoe UI the first choice for proportional text.
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe-ui".to_owned());
    }

    ctx.set_fonts(fonts);
}

/* ============================================================================
   Windows Magnifier helpers (WinAPI, kept close to original)
   ============================================================================ */

fn is_magnifier_running() -> bool {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut pe: PROCESSENTRY32W = zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;

        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                let exe = &pe.szExeFile;
                let target = wstr("magnify.exe");
                if lstrcmpiW(exe.as_ptr(), target.as_ptr()) == 0 {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut pe) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snap);
        found
    }
}

fn get_magnifier_mode_from_registry() -> u32 {
    unsafe {
        let mut mode: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let mut h_key: HKEY = null_mut();

        let key = wstr("Software\\Microsoft\\ScreenMagnifier");
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            KEY_READ,
            &mut h_key,
        ) == 0
        {
            let name = wstr("MagnificationMode");
            RegQueryValueExW(
                h_key,
                name.as_ptr(),
                null_mut(),
                null_mut(),
                &mut mode as *mut _ as *mut u8,
                &mut size,
            );
            RegCloseKey(h_key);
        }
        mode
    }
}

fn get_magnifier_state() -> (bool, bool) {
    if !is_magnifier_running() {
        return (false, false);
    }
    let mode = get_magnifier_mode_from_registry();
    match mode {
        1 => (true, false), // fullscreen
        2 => (false, true), // lens
        _ => (false, false),
    }
}

fn set_reg_dword(subkey: &str, name: &str, value: u32) -> bool {
    unsafe {
        let mut h: HKEY = null_mut();
        let sub = wstr(subkey);
        let ec = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            null_mut(),
            0,
            KEY_SET_VALUE,
            null_mut(),
            &mut h,
            null_mut(),
        );
        if ec != 0 {
            return false;
        }
        let name_w = wstr(name);
        let ec2 = RegSetValueExW(
            h,
            name_w.as_ptr(),
            0,
            REG_DWORD,
            &value as *const _ as *const u8,
            size_of::<u32>() as u32,
        );
        RegCloseKey(h);
        ec2 == 0
    }
}

fn launch_magnifier_fullscreen_175() {
    let _ = set_reg_dword("Software\\Microsoft\\ScreenMagnifier", "MagnificationMode", 1);
    let _ = set_reg_dword("Software\\Microsoft\\ScreenMagnifier", "Magnification", 175);

    unsafe {
        let file = wstr("magnify.exe");
        let args = wstr("/fullscreen");
        ShellExecuteW(
            null_mut(),
            wstr("open").as_ptr(),
            file.as_ptr(),
            args.as_ptr(),
            null_mut(),
            SW_SHOWNORMAL,
        );
    }
}

fn launch_magnifier_lens_175() {
    let _ = set_reg_dword("Software\\Microsoft\\ScreenMagnifier", "MagnificationMode", 2);
    let _ = set_reg_dword("Software\\Microsoft\\ScreenMagnifier", "Magnification", 175);

    unsafe {
        let file = wstr("magnify.exe");
        let args = wstr("/lens");
        ShellExecuteW(
            null_mut(),
            wstr("open").as_ptr(),
            file.as_ptr(),
            args.as_ptr(),
            null_mut(),
            SW_SHOWNORMAL,
        );
    }
}

fn send_win_chord(vk: u16) {
    unsafe {
        let mut inputs: [INPUT; 4] = [zeroed(); 4];
        for inp in &mut inputs {
            (*inp).type_ = INPUT_KEYBOARD;
        }

        {
            let ki = inputs[0].u.ki_mut();
            (*ki).wVk = VK_LWIN as u16;
            (*ki).wScan = 0;
            (*ki).dwFlags = 0;
            (*ki).time = 0;
            (*ki).dwExtraInfo = 0;
        }
        {
            let ki = inputs[1].u.ki_mut();
            (*ki).wVk = vk;
            (*ki).wScan = 0;
            (*ki).dwFlags = 0;
            (*ki).time = 0;
            (*ki).dwExtraInfo = 0;
        }
        {
            let ki = inputs[2].u.ki_mut();
            (*ki).wVk = vk;
            (*ki).wScan = 0;
            (*ki).dwFlags = KEYEVENTF_KEYUP;
            (*ki).time = 0;
            (*ki).dwExtraInfo = 0;
        }
        {
            let ki = inputs[3].u.ki_mut();
            (*ki).wVk = VK_LWIN as u16;
            (*ki).wScan = 0;
            (*ki).dwFlags = KEYEVENTF_KEYUP;
            (*ki).time = 0;
            (*ki).dwExtraInfo = 0;
        }

        SendInput(
            inputs.len() as UINT,
            inputs.as_mut_ptr(),
            size_of::<INPUT>() as c_int,
        );
    }
}

fn ensure_magnifier_closed(timeout_ms: u32) {
    unsafe {
        // Try the polite way first: Win+Esc
        send_win_chord(VK_ESCAPE as u16);
        Sleep(120);

        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return;
        }

        let mut pe: PROCESSENTRY32W = zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut pe) == 0 {
            CloseHandle(snap);
            return;
        }

        loop {
            let exe = &pe.szExeFile;
            let target = wstr("magnify.exe");
            if lstrcmpiW(exe.as_ptr(), target.as_ptr()) == 0 {
                let h = OpenProcess(SYNCHRONIZE | PROCESS_TERMINATE, 0, pe.th32ProcessID);
                if !h.is_null() {
                    if WaitForSingleObject(h, timeout_ms) == WAIT_TIMEOUT {
                        TerminateProcess(h, 0);
                        WaitForSingleObject(h, MAG_FORCEWAIT_MS);
                    }
                    CloseHandle(h);
                }
            }
            if Process32NextW(snap, &mut pe) == 0 {
                break;
            }
        }

        CloseHandle(snap);
    }
}

fn close_magnifier_async(timeout_ms: u32) {
    if !is_magnifier_running() {
        return;
    }
    if G_MAG_CLOSE_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    thread::spawn(move || {
        ensure_magnifier_closed(timeout_ms);
        G_MAG_CLOSE_BUSY.store(false, Ordering::SeqCst);
    });
}

/* ============================================================================
   Clock + title helpers
   ============================================================================ */

fn get_time_strings(mode: ClockMode) -> (String, String) {
    unsafe {
        let mut st: SYSTEMTIME = zeroed();
        GetLocalTime(&mut st);

        match mode {
            ClockMode::Time => {
                let clock = format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond);
                let title = format!(
                    "Dynamic Launcher [{:04}-{:02}-{:02}] — PNSL licensed (press F1 for more information)",
                    st.wYear, st.wMonth, st.wDay
                );
                (clock, title)
            }
            ClockMode::Date => {
                let clock = format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay);
                let title = format!(
                    "Dynamic Launcher [{:02}:{:02}:{:02}] — PNSL licensed (press F1 for more information)",
                    st.wHour, st.wMinute, st.wSecond
                );
                (clock, title)
            }
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
    zoom_full_active: bool,
    zoom_lens_active: bool,
    focus_index: usize,
    show_help: bool,
    show_shutdown_confirm: bool,
    last_mag_check: Instant,
    help_text: String,
}

impl DynamicLauncherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        configure_segoe_font(&cc.egui_ctx);

        let help_text = HELP_TEXT.replace("\r\n", "\n");

        let (clock_text, title) = get_time_strings(ClockMode::Time);
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));

        let (zf, zl) = get_magnifier_state();

        Self {
            clock_mode: ClockMode::Time,
            clock_tile_text: clock_text,
            window_title: title,
            zoom_full_active: zf,
            zoom_lens_active: zl,
            focus_index: 0,
            show_help: false,
            show_shutdown_confirm: false,
            last_mag_check: Instant::now(),
            help_text,
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
        if self.zoom_full_active {
            close_magnifier_async(MAG_SAFEWAIT_MS);
            self.zoom_full_active = false;
        } else {
            close_magnifier_async(MAG_SAFEWAIT_MS);
            launch_magnifier_fullscreen_175();
            self.zoom_full_active = true;
            self.zoom_lens_active = false;
        }
    }

    fn toggle_lens_zoom(&mut self) {
        if self.zoom_lens_active {
            close_magnifier_async(MAG_SAFEWAIT_MS);
            self.zoom_lens_active = false;
        } else {
            close_magnifier_async(MAG_SAFEWAIT_MS);
            launch_magnifier_lens_175();
            self.zoom_lens_active = true;
            self.zoom_full_active = false;
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
   Dynamic tile text sizing (WinAPI-like "fit to control")
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
    // Slightly darker than previous values (conservative tweak)
    let bg = if is_active {
        egui::Color32::from_rgb(22, 80, 40)   // darker green
    } else {
        egui::Color32::from_rgb(35, 35, 38)   // darker gray
    };

    let stroke = egui::Stroke::new(
        if is_focused { 2.0 } else { 1.0 },
        if is_active {
            egui::Color32::from_rgb(120, 200, 120)
        } else {
            egui::Color32::from_rgb(80, 80, 80)
        },
    );

    // Fully dynamic font size: measure text and choose the largest size that fits the tile
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

        // Poll magnifier state every ~1.5s
        if self.last_mag_check.elapsed() >= Duration::from_millis(MAG_CHECK_MS) {
            let (full, lens) = get_magnifier_state();
            self.zoom_full_active = full;
            self.zoom_lens_active = lens;
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
                close_magnifier_async(MAG_SAFEWAIT_MS);
            }
        }

        if f1 {
            self.show_help = !self.show_help;
        }

        // Keep repainting often enough to update the clock smoothly.
        ctx.request_repaint_after(Duration::from_millis(200));

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

        // F1 help window: white text on black background for good contrast
        if self.show_help {
            egui::Window::new("About & License")
                .open(&mut self.show_help)
                .resizable(true)
                .vscroll(false)
                .hscroll(false)
                .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&self.help_text)
                                    .monospace()
                                    .color(egui::Color32::WHITE),
                            );
                        });
                });
        }

        // Shutdown confirmation window: compact, one-line UI "Shutdown computer [ Yes ] [ No ]"
        if self.show_shutdown_confirm {
            egui::Window::new("Shutdown")
                .title_bar(false) // hide the extra "Shutdown" text bar
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size(egui::vec2(420.0, 90.0)) // compact dialog size
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    let font_size = 22.0;

                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0); // slightly less padding -> moves text up a tiny bit
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
                                shutdown_pc_now();
                            }
                            if no.clicked() {
                                self.show_shutdown_confirm = false;
                            }
                        });
                        ui.add_space(6.0);
                    });
                });
        }
    }
}

/* ============================================================================
   main()
   ============================================================================ */

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Wide strip for 5 tiles, compact vertically (matches your “After Dyn”)
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
