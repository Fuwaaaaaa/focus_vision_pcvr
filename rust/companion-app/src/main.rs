// A GUI app: no console window in the shipped (release) exe. Closing that
// window killed the process without `on_exit`, losing the last settings
// change. Debug builds keep the console for `cargo run` logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod adb;
mod config;
mod demo;
mod driver;
mod export;
mod file_dialog;
mod headset_link;
mod process;
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

    let args: Vec<String> = std::env::args().skip(1).collect();
    // The installer registers the driver through us: we find SteamVR in any
    // Steam library, which its script could not.
    if let Some(code) = driver::run_cli(&args) {
        std::process::exit(code);
    }
    let (demo_mode, simulate) = parse_flags(args);

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
    /// Set when the installer registered the driver with SteamVR
    /// (`vrpathreg`) instead of it being copied into SteamVR's folder.
    pub(crate) driver_registered_at: Option<PathBuf>,
    pub(crate) driver_status: String,

    // ADB state
    pub(crate) adb_path: Option<String>,
    pub(crate) devices: Vec<adb::AdbDevice>,
    pub(crate) apk_path: String,
    pub(crate) deploy_status: String,
    pub(crate) last_device_scan: Instant,

    // Streaming state
    pub(crate) pin_code: String,
    pub(crate) connection_status: ConnectionStatus,
    pub(crate) latency_ms: f32,
    pub(crate) fps: u32,
    pub(crate) bitrate_mbps: f32,

    // Audio settings
    pub(crate) audio_enabled: bool,
    pub(crate) audio_bitrate_kbps: u32,

    // Deploy async state
    pub(crate) deploy_in_progress: bool,
    pub(crate) deploy_result: Arc<Mutex<Option<String>>>,

    // "Send PIN to headset" async state (headset_link::send_to_headset)
    pub(crate) pairing_in_progress: bool,
    pub(crate) pairing_result: Arc<Mutex<Option<Result<String, String>>>>,
    pub(crate) pairing_status: String,
    // Where the headset connects: the engine's (TCP, UDP base) ports from
    // status.json, or the defaults for an engine that does not publish them.
    pub(crate) engine_ports: (u16, u16),

    // Engine status (read from status.json)
    last_status_read: Instant,

    // UI state
    pub(crate) active_tab: Tab,
    pub(crate) status_log: Arc<Mutex<Vec<String>>>,

    // v1.1: Codec selection
    pub(crate) selected_codec: String,
    pub(crate) local_config: config::LocalConfig,

    // Debounced persistence of `local_config`. UI handlers call
    // `mark_config_dirty()` instead of saving directly, so a slider drag or a
    // typed path writes the file once after the input settles rather than on
    // every frame. `pending_save_notes` holds one user-facing log line per
    // settings group (keyed by group), emitted when the save lands.
    config_dirty_since: Option<Instant>,
    pending_save_notes: Vec<(&'static str, String)>,

    // v1.1: Stats history for sparkline graphs
    pub(crate) stats_history: stats_history::StatsHistory,

    // v1.1: Log export
    pub(crate) export_in_progress: bool,
    pub(crate) export_result: Arc<Mutex<Option<String>>>,

    // Open file dialogs (each on its own thread): Deploy's APK picker, and
    // the stats SVG's save dialog with the SVG to write.
    pub(crate) apk_dialog: Option<file_dialog::DialogTask<String>>,
    pub(crate) svg_dialog: Option<(file_dialog::DialogTask<PathBuf>, String)>,

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
    // Home tab can render a smooth local countdown. The engine re-publishes
    // status.json every second as a liveness heartbeat but re-emits the same
    // expiry value for a given PIN, so the countdown itself is derived here.
    pub(crate) pin_expires_in_seconds: Option<u32>,
    pub(crate) pin_expires_observed_at: Option<Instant>,

    // Integrated simulation mode (feature = "simulator"). `sim` holds the
    // running engine+mock-client handle; `sim_error` is the sticky last error
    // shown in the UI; `sim_autostart` requests a one-shot start on the first
    // frame when launched with `--simulate`.
    #[cfg(feature = "simulator")]
    pub(crate) sim: Option<sim::SimHandle>,
    #[cfg(feature = "simulator")]
    pub(crate) sim_error: Option<String>,
    pub(crate) sim_autostart: bool,
}

/// status.json is considered stale (engine probably died) once its mtime
/// is older than this. The engine rewrites the file at least once a second
/// in every state (waiting for the HMD, reconnect backoff, streaming), so
/// 5 s is generous enough to avoid false positives on a busy host but short
/// enough that a real crash is surfaced quickly.
const ENGINE_STALE_THRESHOLD: Duration = Duration::from_secs(5);

/// How long settings must stay unchanged before `local.toml` is written.
const CONFIG_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Whether a debounced save is due: something changed and the last change is
/// at least `CONFIG_SAVE_DEBOUNCE` old.
fn config_save_due(dirty_since: Option<Instant>, now: Instant) -> bool {
    dirty_since.is_some_and(|t| now.saturating_duration_since(t) >= CONFIG_SAVE_DEBOUNCE)
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum Tab {
    Home,
    Deploy,
    Settings,
}

impl CompanionApp {
    fn new(cc: &eframe::CreationContext, demo_mode: bool, simulate: bool) -> Self {
        install_fonts(&cc.egui_ctx);

        let mut app = Self::with_config(demo_mode, simulate, config::LocalConfig::load());

        app.steamvr_dir = driver::find_steamvr_drivers_dir();
        app.driver_registered_at = driver::find_registered_driver();
        app.driver_installed = app.driver_registered_at.is_some()
            || app.steamvr_dir.as_ref()
                .map(|d| driver::is_driver_installed(d))
                .unwrap_or(false);
        app.driver_status = if app.driver_registered_at.is_some() {
            "Driver registered with SteamVR".to_string()
        } else if app.steamvr_dir.is_none() {
            "SteamVR not found".to_string()
        } else if app.driver_installed {
            "Driver installed".to_string()
        } else {
            "Driver not installed".to_string()
        };
        app.adb_path = adb::find_adb();
        app
    }

    /// Build the app state from a loaded config without touching the host
    /// environment (no SteamVR / ADB probing, no egui context). `new()`
    /// layers the environment detection on top; unit tests use this directly.
    fn with_config(demo_mode: bool, simulate: bool, cfg: config::LocalConfig) -> Self {
        Self {
            steamvr_dir: None,
            driver_installed: false,
            driver_registered_at: None,
            driver_status: "SteamVR not found".to_string(),
            adb_path: None,
            devices: Vec::new(),
            apk_path: cfg.deploy.apk_path.clone(),
            deploy_status: String::new(),
            last_device_scan: Instant::now() - Duration::from_secs(10),
            pin_code: "----".to_string(),
            connection_status: ConnectionStatus::Disconnected,
            latency_ms: 0.0,
            fps: 0,
            bitrate_mbps: 0.0,
            audio_enabled: cfg.audio.enabled,
            audio_bitrate_kbps: cfg.audio.bitrate_kbps,
            deploy_in_progress: false,
            deploy_result: Arc::new(Mutex::new(None)),
            pairing_in_progress: false,
            pairing_result: Arc::new(Mutex::new(None)),
            pairing_status: String::new(),
            engine_ports: headset_link::DEFAULT_PORTS,
            last_status_read: Instant::now() - Duration::from_secs(10),
            active_tab: Tab::Home,
            status_log: Arc::new(Mutex::new(Vec::new())),
            selected_codec: cfg.video.codec.clone(),
            config_dirty_since: None,
            pending_save_notes: Vec::new(),
            stats_history: stats_history::StatsHistory::new(),
            export_in_progress: false,
            export_result: Arc::new(Mutex::new(None)),
            apk_dialog: None,
            svg_dialog: None,
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
            #[cfg(feature = "simulator")]
            sim_error: None,
            // Demo wins over simulate (already enforced in parse_flags), so a
            // demo launch never autostarts a real engine.
            sim_autostart: simulate && !demo_mode,
            // Moved last so the clones above can still borrow `cfg`.
            local_config: cfg,
        }
    }

    /// Record that `local_config` changed. The file is written by
    /// `flush_config_if_due()` once the input has settled; `note` is logged
    /// then (the latest note per `group` wins, so a slider drag logs once).
    pub(crate) fn mark_config_dirty(&mut self, group: &'static str, note: String) {
        self.config_dirty_since = Some(Instant::now());
        self.pending_save_notes.retain(|(g, _)| *g != group);
        self.pending_save_notes.push((group, note));
    }

    fn flush_config_if_due(&mut self) {
        if config_save_due(self.config_dirty_since, Instant::now()) {
            self.flush_config();
        }
    }

    /// Write pending settings now (debounce elapsed, app exit, or an action
    /// that must persist immediately). No-op when nothing is pending.
    pub(crate) fn flush_config(&mut self) {
        if self.config_dirty_since.take().is_none() {
            return;
        }
        let notes = std::mem::take(&mut self.pending_save_notes);
        match self.local_config.save() {
            Ok(()) => {
                for (_, note) in notes {
                    self.log(&note);
                }
            }
            Err(e) => self.log(&format!("Failed to save config: {e}")),
        }
    }
}

/// Read `fonts/<file>` next to the exe (installer, release zip) or, failing
/// that, in the working directory (`cargo run` from the repo). A file egui
/// cannot parse is skipped: egui panics on one, and a CI download that saved
/// an HTML 404 page as `.ttf` once crashed every launch of the installed app.
fn read_font(file: &str) -> Option<Vec<u8>> {
    let exe_dir = std::env::current_exe().ok()
        .and_then(|exe| exe.parent().map(|d| d.to_path_buf()));
    let candidates = exe_dir.map(|d| d.join("fonts").join(file)).into_iter()
        .chain(std::iter::once(PathBuf::from("fonts").join(file)));
    for path in candidates {
        let Ok(data) = std::fs::read(&path) else { continue };
        if font_parses(&data) {
            return Some(data);
        }
        log::warn!("Ignoring {}: not a font egui can load", path.display());
    }
    None
}

/// The same parse egui runs on font data (ab_glyph), without the panic.
fn font_parses(data: &[u8]) -> bool {
    ab_glyph::FontRef::try_from_slice(data).is_ok()
}

/// Japanese fonts that ship with the OS, most preferred first. Geist and
/// egui's built-in fonts have no kana or kanji, so without one of these
/// every Japanese label is a row of boxes. Face 0 of each collection is a
/// regular Japanese face.
fn japanese_font_candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let fonts_dir = std::env::var_os("WINDIR")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        // Yu Gothic Medium rather than Regular: egui draws without hinting,
        // and Regular comes out thin and faint at 11-13 px.
        ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"]
            .iter()
            .map(|file| fonts_dir.join(file))
            .collect()
    }
    #[cfg(not(windows))]
    {
        [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// Whether `data` is a font egui can load that has kana and kanji.
fn covers_japanese(data: &[u8]) -> bool {
    use ab_glyph::Font;
    ab_glyph::FontRef::try_from_slice(data)
        .is_ok_and(|font| "あア漢（".chars().all(|c| font.glyph_id(c).0 != 0))
}

/// The first of `japanese_font_candidates()` that exists and covers Japanese.
fn read_japanese_font() -> Option<(PathBuf, Vec<u8>)> {
    japanese_font_candidates().into_iter().find_map(|path| {
        let data = std::fs::read(&path).ok()?;
        covers_japanese(&data).then_some((path, data))
    })
}

/// A font's (ascent, descent below the baseline, line gap) in em.
fn vertical_metrics(data: &[u8]) -> Option<(f32, f32, f32)> {
    use ab_glyph::Font;
    let font = ab_glyph::FontRef::try_from_slice(data).ok()?;
    let em = font.units_per_em()?;
    Some((font.ascent_unscaled() / em, -font.descent_unscaled() / em, font.line_gap_unscaled() / em))
}

/// `FontTweak::y_offset_factor` that puts `fallback`'s baseline on
/// `primary`'s. egui lines up two fonts in a row by centering their heights
/// (ascent + descent + line gap), not by their baselines, so a fallback with
/// other metrics sits too high or low — Yu Gothic's large line gap lifts it
/// about 0.25 em above Geist. egui applies the factor to the fallback's
/// size scaled by its (ascent + descent) em.
fn baseline_offset_factor(primary: &[u8], fallback: &[u8]) -> f32 {
    let (Some((ascent_p, descent_p, gap_p)), Some((ascent_f, descent_f, gap_f))) =
        (vertical_metrics(primary), vertical_metrics(fallback))
    else {
        return 0.0;
    };
    let height_p = ascent_p + descent_p + gap_p;
    let height_f = ascent_f + descent_f + gap_f;
    let shift_em = ascent_p - ascent_f - 0.5 * (height_p - height_f);
    shift_em / (ascent_f + descent_f)
}

/// Add `data` as the first fallback of every font family: right after the
/// family's own font, which keeps what it covers, and ahead of egui's
/// built-in fallbacks — whose full-width punctuation comes from an icon
/// font and looks out of place in Japanese text.
fn add_fallback_font(fonts: &mut egui::FontDefinitions, name: &str, data: Vec<u8>, tweak: egui::FontTweak) {
    fonts.font_data.insert(name.to_string(), egui::FontData::from_owned(data).tweak(tweak).into());
    for family in fonts.families.values_mut() {
        family.insert(family.len().min(1), name.to_string());
    }
}

fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

/// Custom fonts from DESIGN.md: Instrument Serif (brand), Geist (UI) and
/// Geist Mono (stats/data), plus an OS font for Japanese text. Missing or
/// unreadable font files fall back to egui defaults.
fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();

    // Instrument Serif for brand/display text
    if let Some(data) = read_font("InstrumentSerif-Regular.ttf") {
        fonts.font_data.insert(
            "InstrumentSerif".to_string(),
            egui::FontData::from_owned(data).into(),
        );
        fonts.families.entry(egui::FontFamily::Name("Brand".into()))
            .or_default()
            .insert(0, "InstrumentSerif".to_string());
    }

    // Geist for UI body text
    if let Some(data) = read_font("Geist-Regular.ttf") {
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
    if let Some(data) = read_font("GeistMono-Regular.ttf") {
        fonts.font_data.insert(
            "GeistMono".to_string(),
            egui::FontData::from_owned(data).into(),
        );
        fonts.families.entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "GeistMono".to_string());
    }

    match read_japanese_font() {
        Some((path, data)) => {
            // Line it up with the UI font. Geist Mono shares Geist's metrics,
            // so monospace text lines up too.
            let ui_font = fonts.families.get(&egui::FontFamily::Proportional)
                .and_then(|family| family.first())
                .and_then(|name| fonts.font_data.get(name));
            let tweak = egui::FontTweak {
                y_offset_factor: ui_font.map_or(0.0, |ui_font| baseline_offset_factor(&ui_font.font, &data)),
                ..Default::default()
            };
            log::info!("Japanese text font: {} ({tweak:?})", path.display());
            add_fallback_font(&mut fonts, "Japanese", data, tweak);
        }
        None => log::warn!("No Japanese font found; Japanese text will show as boxes"),
    }

    fonts
}

impl CompanionApp {
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
        log::warn!("--simulate ignored: built without --features simulator");
    }

    // API-parity stub: only the simulator build has a Stop control that calls
    // this, so in the default build it is intentionally never invoked.
    #[cfg(not(feature = "simulator"))]
    #[allow(dead_code)]
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
        // died. Checked BEFORE parsing: a stale-but-readable payload (e.g.
        // "streaming" left behind by a crashed engine) must not be shown as
        // a live connection with frozen stats and an old PIN.
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.engine_alive = status_parser::is_status_fresh(
            mtime,
            std::time::SystemTime::now(),
            ENGINE_STALE_THRESHOLD,
        );
        if !self.engine_alive {
            self.apply_stale_status();
            return;
        }

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
        self.connection_status = parsed.connection;
        self.engine_ports = parsed.engine_ports.unwrap_or(headset_link::DEFAULT_PORTS);
        // Pin expiry: when the engine emits a fresh value, snapshot it
        // along with the wall-clock time we received it, so the UI can
        // count down locally. The engine re-emits the same value on every
        // heartbeat write, so a change of more than 1s, moving from None, or
        // a different PIN counts as "fresh" — without this we'd reset the
        // countdown every poll cycle (or keep counting down a rotated PIN).
        let pin_changed = parsed.connection == CS::WaitingForPin && parsed.pin != self.pin_code;
        match (parsed.pin_expires_in_seconds, self.pin_expires_in_seconds) {
            (Some(new), Some(old)) if !pin_changed && (new as i64 - old as i64).abs() <= 1 => {
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

    /// status.json is missing or stale: the engine is not running, so
    /// nothing in the last payload can be trusted. Show Disconnected, drop
    /// the old PIN and its countdown instead of freezing the last state.
    fn apply_stale_status(&mut self) {
        self.connection_status = ConnectionStatus::Disconnected;
        self.pin_code = "----".to_string();
        self.pin_expires_in_seconds = None;
        self.pin_expires_observed_at = None;
    }

    /// Start the headset app over adb pointed at this PC with the current
    /// PIN (headset_link). Prefers a device adb reports as a VIVE headset.
    pub(crate) fn send_pin_to_headset(&mut self) {
        let (Some(adb), Some(device)) = (
            self.adb_path.clone(),
            self.devices.iter().find(|d| d.is_focus_vision).or(self.devices.first()).cloned(),
        ) else {
            return;
        };
        self.pairing_in_progress = true;
        self.pairing_status = format!("Sending the PIN to {}...", device.model);
        let pin = self.pin_code.clone();
        let ports = self.engine_ports;
        let result = self.pairing_result.clone();
        std::thread::Builder::new()
            .name("fvp-pairing".into())
            .spawn(move || {
                let outcome = headset_link::send_to_headset(&adb, &device.serial, &pin, ports);
                if let Ok(mut guard) = result.lock() {
                    *guard = Some(outcome);
                }
            })
            .expect("spawn pairing thread");
    }

    fn check_pairing_result(&mut self) {
        let outcome = match self.pairing_result.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(outcome) = outcome {
            self.pairing_in_progress = false;
            self.pairing_status = match outcome {
                Ok(msg) | Err(msg) => msg,
            };
            let note = self.pairing_status.clone();
            self.log(&note);
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
        self.check_pairing_result();
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

        // Debounced settings save. While a change is pending, wake up again
        // once the debounce window closes so the write doesn't wait for the
        // next 1 s stats repaint.
        self.flush_config_if_due();
        if self.config_dirty_since.is_some() {
            ctx.request_repaint_after(CONFIG_SAVE_DEBOUNCE);
        }

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

        // Every tab scrolls: at the default 480×640 (and the 400×500
        // minimum) Settings and a busy Home run past the bottom of the
        // window. One scroll offset per tab, so switching tabs doesn't carry
        // Settings' position over to Home.
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("tab_scroll", self.active_tab))
                .auto_shrink(false)
                .show(ui, |ui| match self.active_tab {
                    Tab::Home => self.render_home(ui, accent, text_muted),
                    Tab::Deploy => self.render_deploy(ui, accent, text_muted),
                    Tab::Settings => self.render_settings(ui, accent, text_muted),
                });
        });
    }

    /// Persist a change made within the last debounce window before the
    /// window closes, so closing the app right after moving a slider still
    /// saves it.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_config();
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

    #[test]
    fn font_check_accepts_real_fonts_and_rejects_an_html_page() {
        // REGRESSION: CI saved GitHub's 404 page as Geist-Regular.ttf and
        // egui panicked on it at every launch of the installed app.
        let defaults = eframe::egui::FontDefinitions::default();
        assert!(!defaults.font_data.is_empty());
        for data in defaults.font_data.values() {
            assert!(super::font_parses(&data.font));
        }
        assert!(!super::font_parses(b"<!DOCTYPE html><html><head><title>Page not found"));
        assert!(!super::font_parses(b""));
    }

    #[test]
    fn japanese_fallback_follows_each_familys_own_font() {
        let mut fonts = eframe::egui::FontDefinitions::default();
        let before = fonts.families.clone();
        super::add_fallback_font(&mut fonts, "Japanese", vec![0; 4], Default::default());
        assert!(fonts.font_data.contains_key("Japanese"));
        for (family, old) in fonts.families.values().zip(before.values()) {
            assert_eq!(family[0], old[0], "each family keeps its own font first");
            assert_eq!(family[1], "Japanese", "ahead of egui's emoji/icon fallbacks");
            assert_eq!(family[2..], old[1..]);
        }
    }

    /// REGRESSION: Japanese text sat ~3 px above the Latin baseline at 11-13
    /// px (Yu Gothic's large line gap; egui centers mixed fonts' heights).
    #[test]
    fn japanese_text_sits_on_the_latin_baseline() {
        use eframe::egui::{epaint::text::Glyph, Color32, Context, FontId, RawInput};
        if super::read_japanese_font().is_none() {
            // Windows always ships one (Yu Gothic); elsewhere it's optional.
            if cfg!(windows) {
                panic!("no Japanese font among {:?}", super::japanese_font_candidates());
            }
            eprintln!("no Japanese font on this machine; skipping the baseline check");
            return;
        }
        let ctx = Context::default();
        super::install_fonts(&ctx);
        let _ = ctx.run(RawInput::default(), |_| {});
        // Where a glyph is drawn: its baseline (`pos`) plus its bitmap's offset.
        let bottom = |g: &Glyph| g.pos.y + g.uv_rect.offset.y + g.uv_rect.size.y;
        for size in [11.0, 13.0, 15.0, 20.0, 32.0] {
            // Both H's stand on the baseline; the full-width one exists only
            // in the Japanese font.
            let galley = ctx.fonts(|f| f.layout_no_wrap("HＨ".into(), FontId::proportional(size), Color32::WHITE));
            let [latin, japanese] = [&galley.rows[0].glyphs[0], &galley.rows[0].glyphs[1]];
            assert_ne!(latin.font_impl_height, japanese.font_impl_height, "Ｈ must come from the Japanese font");
            let off = bottom(japanese) - bottom(latin);
            assert!(off.abs() <= 1.0, "at {size} px the Japanese text stands {off:+.2} px off the Latin baseline");
        }
    }

    /// REGRESSION: no loaded font had kana or kanji, so the engine-stopped
    /// banner and every other Japanese label rendered as boxes.
    #[test]
    fn japanese_ui_text_has_glyphs() {
        use eframe::egui::{Context, FontId, RawInput};
        let defaults = eframe::egui::FontDefinitions::default();
        assert!(defaults.font_data.values().all(|d| !super::covers_japanese(&d.font)),
            "egui's own fonts have no Japanese — the OS font is what draws it");
        if super::read_japanese_font().is_none() {
            // Windows always ships one (Yu Gothic); elsewhere it's optional.
            if cfg!(windows) {
                panic!("no Japanese font among {:?}", super::japanese_font_candidates());
            }
            eprintln!("no Japanese font on this machine; skipping the glyph check");
            return;
        }

        let ctx = Context::default();
        super::install_fonts(&ctx);
        let _ = ctx.run(RawInput::default(), |_| {}); // fonts load at frame start
        for text in [
            "DEMO MODE — シミュレーション中（実エンジンは起動していません）",
            "SIMULATION — ローカルエンジン稼働中（ハードウェア不要）",
            "ストリーミングエンジンが停止しています",
            "実機なしでパイプライン全体をローカル実行（ヘッドセット不要）",
            "ADB経由でFocus VisionにAPKをインストール",
            "実エンジンが稼働中です（status.json が新しい）。",
        ] {
            for font in [FontId::proportional(13.0), FontId::monospace(13.0)] {
                assert!(ctx.fonts(|f| f.has_glyphs(&font, text)), "{font:?} can't draw {text:?}");
            }
        }
    }

    // --- status state machine (no filesystem, no egui context) ---

    use super::{config, config_save_due, CompanionApp, CONFIG_SAVE_DEBOUNCE};
    use crate::status_parser::{parse_status_json, ConnectionStatus};
    use std::time::{Duration, Instant};

    fn app() -> CompanionApp {
        CompanionApp::with_config(false, false, config::LocalConfig::default())
    }

    fn waiting(pin: &str) -> crate::status_parser::ParsedStatus {
        parse_status_json(&format!(
            r#"{{"status":"waiting","pin":"{pin}","pin_expires_in_seconds":300}}"#
        ))
        .unwrap()
    }

    #[test]
    fn stale_status_drops_connected_state() {
        // Regression: a "streaming" payload left behind by a crashed engine
        // kept the UI on Connected with frozen stats.
        let mut a = app();
        a.apply_parsed_status(
            parse_status_json(r#"{"status":"streaming","pin":"123456","fps":90}"#).unwrap(),
        );
        assert_eq!(a.connection_status, ConnectionStatus::Connected);

        a.apply_stale_status();
        assert_eq!(a.connection_status, ConnectionStatus::Disconnected);
        assert_eq!(a.pin_code, "----");
    }

    #[test]
    fn stale_status_clears_pin_and_countdown() {
        let mut a = app();
        a.apply_parsed_status(waiting("048217"));
        assert_eq!(a.connection_status, ConnectionStatus::WaitingForPin);
        assert_eq!(a.pin_expires_in_seconds, Some(300));

        a.apply_stale_status();
        assert_eq!(a.connection_status, ConnectionStatus::Disconnected);
        assert_eq!(a.pin_code, "----");
        assert_eq!(a.pin_expires_in_seconds, None);
        assert!(a.pin_expires_observed_at.is_none());
    }

    #[test]
    fn heartbeat_rewrite_of_same_pin_keeps_countdown() {
        // The engine re-publishes status.json every second with the same
        // expiry value; that must not restart the countdown.
        let mut a = app();
        a.apply_parsed_status(waiting("048217"));
        let earlier = Instant::now() - Duration::from_secs(100);
        a.pin_expires_observed_at = Some(earlier);

        a.apply_parsed_status(waiting("048217"));
        assert_eq!(a.pin_expires_observed_at, Some(earlier));
    }

    #[test]
    fn rotated_pin_restarts_countdown() {
        let mut a = app();
        a.apply_parsed_status(waiting("048217"));
        let earlier = Instant::now() - Duration::from_secs(100);
        a.pin_expires_observed_at = Some(earlier);

        a.apply_parsed_status(waiting("731904"));
        assert_eq!(a.pin_code, "731904");
        assert!(a.pin_expires_observed_at.unwrap() > earlier, "new PIN must restart the countdown");
    }

    // --- debounced settings save ---

    #[test]
    fn config_save_due_waits_for_debounce_window() {
        let now = Instant::now();
        assert!(!config_save_due(None, now), "nothing pending");
        assert!(!config_save_due(Some(now), now), "change just happened");
        assert!(config_save_due(Some(now - CONFIG_SAVE_DEBOUNCE), now));
    }

    #[test]
    fn mark_config_dirty_keeps_latest_note_per_group() {
        // A slider drag marks the same group every frame; only one log line
        // per group may come out when the save lands.
        let mut a = app();
        a.mark_config_dirty("audio", "Audio: 64".to_string());
        a.mark_config_dirty("audio", "Audio: 96".to_string());
        a.mark_config_dirty("video", "Codec h264".to_string());
        assert!(a.config_dirty_since.is_some());
        let notes: Vec<&str> = a.pending_save_notes.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(notes, vec!["Audio: 96", "Codec h264"]);
    }

    #[test]
    fn flush_config_without_pending_change_is_noop() {
        let mut a = app();
        a.flush_config();
        assert!(a.config_dirty_since.is_none());
        assert!(a.status_log.lock().unwrap().is_empty());
    }
}
