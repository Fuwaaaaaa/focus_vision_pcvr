mod adb;
mod config;
mod demo;
mod driver;
mod export;
#[cfg(feature = "simulator")]
mod sim;
mod stats_history;
mod status_parser;
mod svg_export;
mod ui;

use status_parser::{parse_status_json, ConnectionStatus};

use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Parse the two launch flags. `--demo` and `--simulate` are mutually
/// exclusive (demo is a fake synthesizer that bypasses status.json; simulate
/// runs a real engine that writes it) — if both are present, demo wins.
fn parse_flags<I: IntoIterator<Item = String>>(args: I) -> (bool, bool) {
    let mut demo = false;
    let mut simulate = false;
    for a in args {
        match a.as_str() {
            "--demo" => demo = true,
            "--simulate" => simulate = true,
            _ => {}
        }
    }
    if demo && simulate {
        log::warn!("--demo and --simulate are mutually exclusive; --demo takes precedence");
        simulate = false;
    }
    (demo, simulate)
}

fn main() -> eframe::Result {
    env_logger::init();

    let (demo_mode, simulate) = parse_flags(std::env::args().skip(1));

    let title = if demo_mode {
        "Focus Vision PCVR — DEMO MODE"
    } else if simulate {
        "Focus Vision PCVR — SIMULATION"
    } else {
        "Focus Vision PCVR"
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 640.0])
            .with_min_inner_size([400.0, 500.0])
            .with_title(title),
        ..Default::default()
    };

    eframe::run_native(
        "Focus Vision PCVR",
        options,
        Box::new(move |cc| Ok(Box::new(CompanionApp::new(cc, demo_mode, simulate)))),
    )
}

pub(crate) struct CompanionApp {
    // Driver state
    pub(crate) steamvr_dir: Option<PathBuf>,
    pub(crate) driver_installed: bool,
    pub(crate) driver_status: String,

    // ADB state
    pub(crate) adb_path: Option<String>,
    pub(crate) devices: Vec<adb::AdbDevice>,
    pub(crate) apk_path: String,
    pub(crate) deploy_status: String,
    pub(crate) last_device_scan: Instant,

    // Streaming state
    pub(crate) pin_code: String,
    pub(crate) latency_ms: f32,
    pub(crate) fps: u32,
    pub(crate) bitrate_mbps: f32,

    // Audio settings
    pub(crate) audio_enabled: bool,
    pub(crate) audio_bitrate_kbps: u32,

    // Deploy async state
    pub(crate) deploy_in_progress: bool,
    pub(crate) deploy_result: Arc<Mutex<Option<String>>>,

    // Engine status (read from status.json)
    last_status_read: Instant,

    // UI state
    pub(crate) active_tab: Tab,
    pub(crate) status_log: Arc<Mutex<Vec<String>>>,

    // v1.1: Codec selection
    pub(crate) selected_codec: String,
    pub(crate) local_config: config::LocalConfig,

    // v1.1: Stats history for sparkline graphs
    pub(crate) stats_history: stats_history::StatsHistory,

    // v1.1: Log export
    pub(crate) export_in_progress: bool,
    pub(crate) export_result: Arc<Mutex<Option<String>>>,

    // v1.2: Subsystem status (read from status.json)
    pub(crate) sub_ft_active: bool,
    pub(crate) sub_sleep_active: bool,
    pub(crate) sub_audio_enabled: bool,
    pub(crate) sub_packet_loss: f32,

    // v1.2: Sleep mode + face tracking settings
    pub(crate) sleep_enabled: bool,
    pub(crate) sleep_timeout: u32,
    pub(crate) ft_enabled: bool,
    pub(crate) ft_smoothing: f32,

    // Session Recording settings
    pub(crate) recording_enabled: bool,
    pub(crate) recording_output_dir: String,

    // Engine liveness — `false` when status.json is missing or its mtime is
    // older than ENGINE_STALE_THRESHOLD. Used by the Home tab banner.
    // Defaults to false so a freshly-launched companion (before the first
    // status.json read) shows "engine stopped" instead of misleading
    // "everything's fine".
    pub(crate) engine_alive: bool,

    // Two-stage confirmation for "Reset to defaults". `Some(deadline)` puts
    // the button into "click again to confirm" mode until the deadline; a
    // second click before then performs the reset, otherwise the prompt
    // reverts on the next paint. Keeps an irreversible action one tap away
    // from a misclick.
    pub(crate) reset_confirm_until: Option<Instant>,

    // Demo mode: when true, the companion synthesizes `ParsedStatus`
    // from elapsed wall-clock time instead of reading `status.json`. ADB
    // scans and driver installs are also disabled so a screen-recording
    // session doesn't accidentally poke the user's real environment.
    // Selected via `--demo` on the command line; defaults to false.
    pub(crate) demo_mode: bool,
    pub(crate) demo_start: Instant,

    // PIN expiry countdown — populated from `status.json` (or by the demo
    // synthesizer). `None` means the engine isn't emitting the field yet,
    // which is also the v2-era payload shape; the UI gates the display
    // on Some(_) so old engines don't show a stuck "Expires in: 0:00".
    //
    // We additionally track when we last observed a new value, so the
    // Home tab can render a live local countdown even when the engine
    // only emits the field once per PIN issuance (which is the case
    // today — the engine doesn't repaint status.json once a second).
    pub(crate) pin_expires_in_seconds: Option<u32>,
    pub(crate) pin_expires_observed_at: Option<Instant>,

    // Integrated simulation mode (feature = "simulator"). `sim` holds the
    // running engine+mock-client handle; `sim_error` is the sticky last error
    // shown in the UI; `sim_autostart` requests a one-shot start on the first
    // frame when launched with `--simulate`.
    #[cfg(feature = "simulator")]
    pub(crate) sim: Option<sim::SimHandle>,
    pub(crate) sim_error: Option<String>,
    pub(crate) sim_autostart: bool,
}

/// status.json is considered stale (engine probably died) once its mtime
/// is older than this. The engine rewrites the file on every meaningful
/// event (PIN issued, session started, frame stats updated). 5 s is
/// generous enough to avoid false positives on a busy host but short
/// enough that a real crash is surfaced quickly.
const ENGINE_STALE_THRESHOLD: Duration = Duration::from_secs(5);

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Tab {
    Home,
    Deploy,
    Settings,
}

impl CompanionApp {
    fn new(cc: &eframe::CreationContext, demo_mode: bool, simulate: bool) -> Self {
        // Load custom fonts from DESIGN.md: Instrument Serif (brand) + Geist (UI)
        let mut fonts = egui::FontDefinitions::default();

        // Instrument Serif for brand/display text
        if let Ok(data) = std::fs::read("fonts/InstrumentSerif-Regular.ttf") {
            fonts.font_data.insert(
                "InstrumentSerif".to_string(),
                egui::FontData::from_owned(data).into(),
            );
            fonts.families.entry(egui::FontFamily::Name("Brand".into()))
                .or_default()
                .insert(0, "InstrumentSerif".to_string());
        }

        // Geist for UI body text
        if let Ok(data) = std::fs::read("fonts/Geist-Regular.ttf") {
            fonts.font_data.insert(
                "Geist".to_string(),
                egui::FontData::from_owned(data).into(),
            );
            // Set as default proportional font
            fonts.families.entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "Geist".to_string());
        }

        // Geist Mono for stats/data
        if let Ok(data) = std::fs::read("fonts/GeistMono-Regular.ttf") {
            fonts.font_data.insert(
                "GeistMono".to_string(),
                egui::FontData::from_owned(data).into(),
            );
            fonts.families.entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "GeistMono".to_string());
        }

        cc.egui_ctx.set_fonts(fonts);
        let steamvr_dir = driver::find_steamvr_drivers_dir();
        let driver_installed = steamvr_dir.as_ref()
            .map(|d| driver::is_driver_installed(d))
            .unwrap_or(false);

        let adb_path = adb::find_adb();
        let driver_status = if steamvr_dir.is_none() {
            "SteamVR not found".to_string()
        } else if driver_installed {
            "Driver installed".to_string()
        } else {
            "Driver not installed".to_string()
        };

        // Load the persisted config once and hand its fields out to the
        // shadow UI state. Holding `local_config` last means the move into
        // the struct ends the load() lifetimes cleanly.
        let cfg = config::LocalConfig::load();

        Self {
            steamvr_dir,
            driver_installed,
            driver_status,
            adb_path,
            devices: Vec::new(),
            apk_path: cfg.deploy.apk_path.clone(),
            deploy_status: String::new(),
            last_device_scan: Instant::now() - Duration::from_secs(10),
            pin_code: "----".to_string(),
            latency_ms: 0.0,
            fps: 0,
            bitrate_mbps: 0.0,
            audio_enabled: cfg.audio.enabled,
            audio_bitrate_kbps: cfg.audio.bitrate_kbps,
            deploy_in_progress: false,
            deploy_result: Arc::new(Mutex::new(None)),
            last_status_read: Instant::now() - Duration::from_secs(10),
            active_tab: Tab::Home,
            status_log: Arc::new(Mutex::new(Vec::new())),
            selected_codec: cfg.video.codec.clone(),
            stats_history: stats_history::StatsHistory::new(),
            export_in_progress: false,
            export_result: Arc::new(Mutex::new(None)),
            sub_ft_active: false,
            sub_sleep_active: false,
            sub_audio_enabled: true,
            sub_packet_loss: 0.0,
            sleep_enabled: cfg.sleep_mode.enabled,
            sleep_timeout: cfg.sleep_mode.timeout_seconds,
            ft_enabled: cfg.face_tracking.enabled,
            ft_smoothing: cfg.face_tracking.smoothing,
            recording_enabled: cfg.recording.enabled,
            recording_output_dir: cfg.recording.output_dir.clone(),
            // Starts false: we haven't seen a fresh status.json yet, so we
            // assume the engine is down until proven otherwise. The first
            // read_engine_status() tick (within 1 s) will flip this to true
            // if SteamVR is running and the engine is healthy.
            engine_alive: false,
            reset_confirm_until: None,
            demo_mode,
            demo_start: Instant::now(),
            pin_expires_in_seconds: None,
            pin_expires_observed_at: None,
            #[cfg(feature = "simulator")]
            sim: None,
            sim_error: None,
            // Demo wins over simulate (already enforced in parse_flags), so a
            // demo launch never autostarts a real engine.
            sim_autostart: simulate && !demo_mode,
            local_config: cfg,
        }
    }

    /// Start the in-process simulation. No-op stub when the `simulator`
    /// feature is not compiled in.
    #[cfg(feature = "simulator")]
    pub(crate) fn start_sim(&mut self) {
        if self.sim.is_some() {
            return; // already running
        }
        // Refuse to start if a real engine appears alive — two writers racing
        // on the single shared status.json would confuse the UI.
        if self.engine_alive {
            self.sim_error = Some(
                "実エンジンが稼働中です（status.json が新しい）。SteamVR を停止してから Simulation を開始してください。".to_string(),
            );
            return;
        }
        match sim::start() {
            Ok(h) => {
                self.sim_error = None;
                self.sim = Some(h);
                self.log("Simulation started");
            }
            Err(e) => {
                self.sim_error = Some(e.clone());
                self.log(&format!("Simulation start failed: {e}"));
            }
        }
    }

    /// Stop the in-process simulation. No-op stub when the feature is off.
    #[cfg(feature = "simulator")]
    pub(crate) fn stop_sim(&mut self) {
        if let Some(h) = self.sim.take() {
            h.stop();
            self.log("Simulation stopped");
        }
    }

    /// Whether a simulation is currently running.
    #[cfg(feature = "simulator")]
    pub(crate) fn is_simulating(&self) -> bool {
        self.sim.is_some()
    }

    #[cfg(not(feature = "simulator"))]
    pub(crate) fn start_sim(&mut self) {
        self.sim_error =
            Some("このビルドは --features simulator なしでビルドされています。".to_string());
    }

    #[cfg(not(feature = "simulator"))]
    pub(crate) fn stop_sim(&mut self) {}

    #[cfg(not(feature = "simulator"))]
    pub(crate) fn is_simulating(&self) -> bool {
        false
    }

    pub(crate) fn log(&self, msg: &str) {
        if let Ok(mut log) = self.status_log.lock() {
            log.push(msg.to_string());
            if log.len() > 100 { log.remove(0); }
        }
    }

    pub(crate) fn scan_devices(&mut self) {
        if self.last_device_scan.elapsed() < Duration::from_secs(3) {
            return;
        }
        self.last_device_scan = Instant::now();

        // Demo mode never touches the user's real ADB devices — scanning
        // would surface real hardware in the picker, which is misleading
        // when the rest of the UI is synthetic.
        if self.demo_mode {
            self.devices.clear();
            return;
        }

        if let Some(ref adb) = self.adb_path {
            self.devices = adb::list_devices(adb);
        }
    }

    fn read_engine_status(&mut self) {
        if self.last_status_read.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_status_read = Instant::now();

        if self.demo_mode {
            // Bypass the filesystem: the synthesizer produces a fresh
            // ParsedStatus from elapsed time. We force engine_alive=true
            // so the red "engine stopped" banner stays hidden — the
            // yellow DEMO banner on top tells the user what's actually
            // happening.
            let parsed = demo::synthesize(self.demo_start.elapsed());
            self.engine_alive = true;
            self.apply_parsed_status(parsed);
            return;
        }

        let path = match dirs_next::data_dir() {
            Some(d) => d.join("FocusVisionPCVR").join("status.json"),
            None => return,
        };

        // Engine liveness from file mtime. If the file is missing or older
        // than ENGINE_STALE_THRESHOLD, the engine has either not started or
        // died. Read this BEFORE parsing so a stale-but-readable payload
        // doesn't accidentally show "connected".
        self.engine_alive = match std::fs::metadata(&path) {
            Ok(meta) => meta
                .modified()
                .ok()
                .and_then(|mtime| std::time::SystemTime::now().duration_since(mtime).ok())
                .map(|age| age < ENGINE_STALE_THRESHOLD)
                .unwrap_or(false),
            Err(_) => false,
        };

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let parsed = match parse_status_json(&contents) {
            Some(p) => p,
            None => return,
        };

        // Forward-compat: a schema bump only logs — the parser already
        // tolerates extra fields and missing optional ones.
        if let Some(observed) = parsed.schema_version {
            if observed != fvp_common::STATUS_SCHEMA_VERSION {
                log::debug!(
                    "status.json schema_version mismatch: companion expects {}, engine wrote {}",
                    fvp_common::STATUS_SCHEMA_VERSION,
                    observed
                );
            }
        }

        self.apply_parsed_status(parsed);
    }

    /// Apply a parsed status payload to the live UI state. Split out from
    /// `read_engine_status` so unit tests can drive the state machine
    /// without touching the filesystem.
    fn apply_parsed_status(&mut self, parsed: status_parser::ParsedStatus) {
        use status_parser::ConnectionStatus as CS;
        // Pin expiry: when the engine emits a fresh value, snapshot it
        // along with the wall-clock time we received it, so the UI can
        // count down locally even though status.json isn't rewritten
        // every second. A change of more than 1s OR moving from None
        // counts as "fresh" — without this we'd reset the countdown
        // every poll cycle.
        match (parsed.pin_expires_in_seconds, self.pin_expires_in_seconds) {
            (Some(new), Some(old)) if (new as i64 - old as i64).abs() <= 1 => {
                // No-op: engine re-emitted same value, keep our running countdown.
            }
            (Some(new), _) => {
                self.pin_expires_in_seconds = Some(new);
                self.pin_expires_observed_at = Some(Instant::now());
            }
            (None, _) => {
                self.pin_expires_in_seconds = None;
                self.pin_expires_observed_at = None;
            }
        }

        match parsed.connection {
            CS::Disconnected => {
                // Leave stats alone on transient disconnects so the sparkline
                // doesn't snap to zero between status writes.
            }
            CS::WaitingForPin => {
                self.pin_code = parsed.pin;
            }
            CS::Connected => {
                self.pin_code = parsed.pin;
                self.latency_ms = parsed.latency_ms;
                self.fps = parsed.fps;
                self.bitrate_mbps = parsed.bitrate_mbps;
                self.sub_ft_active = parsed.subsystems.ft_active.unwrap_or(false);
                self.sub_sleep_active = parsed.subsystems.sleep_active.unwrap_or(false);
                self.sub_audio_enabled = parsed.subsystems.audio_enabled.unwrap_or(true);
                self.sub_packet_loss = parsed.subsystems.packet_loss_pct.unwrap_or(0.0);
                self.stats_history.push(self.latency_ms, self.fps as f32, self.sub_packet_loss);
            }
        }
    }

    fn check_deploy_result(&mut self) {
        if let Ok(mut result) = self.deploy_result.lock() {
            if let Some(msg) = result.take() {
                self.deploy_status = msg.clone();
                self.deploy_in_progress = false;
                self.log(&msg);
            }
        }
    }

    fn check_export_result(&mut self) {
        if let Ok(mut result) = self.export_result.lock() {
            if let Some(msg) = result.take() {
                self.export_in_progress = false;
                self.log(&msg);
            }
        }
    }
}

impl eframe::App for CompanionApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Auto-refresh device list and engine status
        self.scan_devices();
        self.read_engine_status();
        self.check_deploy_result();
        self.check_export_result();

        // Simulation: one-shot autostart (when launched with --simulate) and
        // surfacing of any error raised by the worker threads.
        if self.sim_autostart {
            self.sim_autostart = false;
            self.start_sim();
        }
        #[cfg(feature = "simulator")]
        {
            let err = self.sim.as_ref().and_then(|h| h.error());
            if let Some(e) = err {
                self.sim_error = Some(e);
            }
        }

        // Request repaint every second for live stats
        ctx.request_repaint_after(Duration::from_secs(1));

        // Color scheme matching DESIGN.md
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = egui::Color32::from_rgb(10, 10, 12);
        style.visuals.window_fill = egui::Color32::from_rgb(17, 17, 20);
        style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(26, 26, 31);
        ctx.set_style(style);

        let accent = egui::Color32::from_rgb(52, 211, 153); // #34D399
        let text_muted = egui::Color32::from_rgb(152, 152, 164);

        if self.demo_mode {
            // Warning yellow banner pinned above the tab bar. Uses #FBBF24
            // on a darker fill so it reads as "informational" rather than
            // "error" — the red engine-stopped banner uses #F87171 instead.
            egui::TopBottomPanel::top("demo_banner")
                .frame(
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(38, 30, 8))
                        .inner_margin(egui::Margin::symmetric(12, 6)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("●")
                                .color(egui::Color32::from_rgb(251, 191, 36)),
                        );
                        ui.label(
                            egui::RichText::new(
                                "DEMO MODE — シミュレーション中（実エンジンは起動していません）",
                            )
                            .color(egui::Color32::from_rgb(251, 191, 36))
                            .strong(),
                        );
                    });
                });
        }

        // Simulation banner — info blue (#60a5fa, DESIGN.md "情報通知"), kept
        // visually distinct from the demo banner (warning yellow) and the
        // engine-stopped banner (error red). Shown only while a real
        // in-process engine is running.
        #[cfg(feature = "simulator")]
        if self.is_simulating() {
            egui::TopBottomPanel::top("sim_banner")
                .frame(
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(8, 22, 38))
                        .inner_margin(egui::Margin::symmetric(12, 6)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("●")
                                .color(egui::Color32::from_rgb(96, 165, 250)),
                        );
                        ui.label(
                            egui::RichText::new(
                                "SIMULATION — ローカルエンジン稼働中（ハードウェア不要）",
                            )
                            .color(egui::Color32::from_rgb(96, 165, 250))
                            .strong(),
                        );
                    });
                });
        }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Home, "Home");
                ui.selectable_value(&mut self.active_tab, Tab::Deploy, "Deploy to HMD");
                ui.selectable_value(&mut self.active_tab, Tab::Settings, "Settings");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                Tab::Home => self.render_home(ui, accent, text_muted),
                Tab::Deploy => self.render_deploy(ui, accent, text_muted),
                Tab::Settings => self.render_settings(ui, accent, text_muted),
            }
        });
    }
}

/// Simple file picker fallback (no rfd crate — uses Windows common dialog via command).
/// `pub(crate)` so the Deploy tab can invoke it from `ui::deploy`.
pub(crate) fn rfd_pick_file() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("powershell")
            .args(["-Command", r#"
                Add-Type -AssemblyName System.Windows.Forms
                $dialog = New-Object System.Windows.Forms.OpenFileDialog
                $dialog.Filter = 'APK files (*.apk)|*.apk|All files (*.*)|*.*'
                $dialog.Title = 'Select APK to install'
                if ($dialog.ShowDialog() -eq 'OK') { $dialog.FileName }
            "#])
            .output()
            .ok()?;

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() { None } else { Some(path) }
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// File-save dialog companion to `rfd_pick_file()`. Returns `None` if the
/// user cancelled, no dialog backend is available, or the call failed.
/// `default_stem` becomes the suggested filename (no extension); `ext`
/// is the file extension without a dot. Used by the SVG export button
/// in the Settings tab.
pub(crate) fn pick_save_path(default_stem: &str, ext: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // Inject the values via env vars rather than concatenating into the
        // script body — that way a user-supplied stem with quotes or
        // backticks can't break the PowerShell quoting.
        let output = Command::new("powershell")
            .env("FVP_SAVE_STEM", default_stem)
            .env("FVP_SAVE_EXT", ext)
            .args([
                "-Command",
                r#"
                Add-Type -AssemblyName System.Windows.Forms
                $stem = $env:FVP_SAVE_STEM
                $ext = $env:FVP_SAVE_EXT
                $dialog = New-Object System.Windows.Forms.SaveFileDialog
                $dialog.Filter = "$ext files (*.$ext)|*.$ext|All files (*.*)|*.*"
                $dialog.Title = 'Save'
                $dialog.FileName = "$stem.$ext"
                $dialog.DefaultExt = $ext
                $dialog.AddExtension = $true
                if ($dialog.ShowDialog() -eq 'OK') { $dialog.FileName }
                "#,
            ])
            .output()
            .ok()?;

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // No native dialog on non-Windows targets — drop the SVG next to
        // the user's data dir so they can still find it from a script.
        let _ = (default_stem, ext);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::parse_flags;

    fn flags(args: &[&str]) -> (bool, bool) {
        parse_flags(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parse_flags_demo_only() {
        assert_eq!(flags(&["--demo"]), (true, false));
    }

    #[test]
    fn parse_flags_simulate_only() {
        assert_eq!(flags(&["--simulate"]), (false, true));
    }

    #[test]
    fn parse_flags_both_demo_wins() {
        // Mutually exclusive: demo takes precedence, simulate suppressed.
        assert_eq!(flags(&["--simulate", "--demo"]), (true, false));
        assert_eq!(flags(&["--demo", "--simulate"]), (true, false));
    }

    #[test]
    fn parse_flags_neither() {
        assert_eq!(flags(&[]), (false, false));
        assert_eq!(flags(&["--other", "foo"]), (false, false));
    }

    #[test]
    fn parse_flags_order_independent() {
        assert_eq!(flags(&["foo", "--simulate", "bar"]), (false, true));
    }
}
