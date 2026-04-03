mod adb;
mod driver;

use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 640.0])
            .with_min_inner_size([400.0, 500.0])
            .with_title("Focus Vision PCVR"),
        ..Default::default()
    };

    eframe::run_native(
        "Focus Vision PCVR",
        options,
        Box::new(|cc| Ok(Box::new(CompanionApp::new(cc)))),
    )
}

struct CompanionApp {
    // Driver state
    steamvr_dir: Option<PathBuf>,
    driver_installed: bool,
    driver_status: String,

    // ADB state
    adb_path: Option<String>,
    devices: Vec<adb::AdbDevice>,
    apk_path: String,
    deploy_status: String,
    last_device_scan: Instant,

    // Streaming state
    pin_code: String,
    connection_status: ConnectionStatus,
    latency_ms: f32,
    fps: u32,
    bitrate_mbps: f32,

    // Audio settings
    audio_enabled: bool,
    audio_bitrate_kbps: u32,

    // Deploy async state
    deploy_in_progress: bool,
    deploy_result: Arc<Mutex<Option<String>>>,

    // Engine status (read from status.json)
    last_status_read: Instant,

    // UI state
    active_tab: Tab,
    status_log: Arc<Mutex<Vec<String>>>,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Home,
    Deploy,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
enum ConnectionStatus {
    Disconnected,
    WaitingForPin,
    Connected,
}

impl CompanionApp {
    fn new(cc: &eframe::CreationContext) -> Self {
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

        Self {
            steamvr_dir,
            driver_installed,
            driver_status,
            adb_path,
            devices: Vec::new(),
            apk_path: String::new(),
            deploy_status: String::new(),
            last_device_scan: Instant::now() - Duration::from_secs(10),
            pin_code: "----".to_string(),
            connection_status: ConnectionStatus::Disconnected,
            latency_ms: 0.0,
            fps: 0,
            bitrate_mbps: 0.0,
            audio_enabled: true,
            audio_bitrate_kbps: 128,
            deploy_in_progress: false,
            deploy_result: Arc::new(Mutex::new(None)),
            last_status_read: Instant::now() - Duration::from_secs(10),
            active_tab: Tab::Home,
            status_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self, msg: &str) {
        if let Ok(mut log) = self.status_log.lock() {
            log.push(msg.to_string());
            if log.len() > 100 { log.remove(0); }
        }
    }

    fn scan_devices(&mut self) {
        if self.last_device_scan.elapsed() < Duration::from_secs(3) {
            return;
        }
        self.last_device_scan = Instant::now();

        if let Some(ref adb) = self.adb_path {
            self.devices = adb::list_devices(adb);
        }
    }

    fn read_engine_status(&mut self) {
        if self.last_status_read.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_status_read = Instant::now();

        let path = match dirs_next::data_dir() {
            Some(d) => d.join("FocusVisionPCVR").join("status.json"),
            None => return,
        };

        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&contents) {
                let status = val["status"].as_str().unwrap_or("unknown");
                match status {
                    "waiting" => {
                        if let Some(pin) = val["pin"].as_str() {
                            if pin != "----" {
                                self.pin_code = pin.to_string();
                                self.connection_status = ConnectionStatus::WaitingForPin;
                            } else {
                                self.connection_status = ConnectionStatus::Disconnected;
                            }
                        }
                    }
                    "streaming" => {
                        self.connection_status = ConnectionStatus::Connected;
                        if let Some(pin) = val["pin"].as_str() {
                            self.pin_code = pin.to_string();
                        }
                        self.latency_ms = val["latency_us"].as_u64().unwrap_or(0) as f32 / 1000.0;
                        self.fps = val["fps"].as_u64().unwrap_or(0) as u32;
                        self.bitrate_mbps = val["bitrate_mbps"].as_u64().unwrap_or(0) as f32;
                    }
                    _ => {
                        self.connection_status = ConnectionStatus::Disconnected;
                    }
                }
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
}

impl eframe::App for CompanionApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Auto-refresh device list and engine status
        self.scan_devices();
        self.read_engine_status();
        self.check_deploy_result();

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

impl CompanionApp {
    fn render_home(&mut self, ui: &mut egui::Ui, accent: egui::Color32, text_muted: egui::Color32) {
        ui.add_space(16.0);

        // Brand
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Focus").size(32.0));
            ui.label(egui::RichText::new("Vision").size(32.0).color(accent).italics());
        });
        ui.label(egui::RichText::new("PCVR Streaming").size(14.0).color(text_muted));

        ui.add_space(24.0);

        // Driver status
        ui.group(|ui| {
            ui.label(egui::RichText::new("SteamVR Driver").size(13.0).color(text_muted));
            ui.horizontal(|ui| {
                let (dot_color, status_text) = if self.driver_installed {
                    (accent, "Installed")
                } else if self.steamvr_dir.is_some() {
                    (egui::Color32::from_rgb(251, 191, 36), "Not installed")
                } else {
                    (egui::Color32::from_rgb(248, 113, 113), "SteamVR not found")
                };
                ui.label(egui::RichText::new("●").color(dot_color));
                ui.label(status_text);
            });

            if !self.driver_installed {
                if let Some(ref _dir) = self.steamvr_dir {
                    if ui.button("Install Driver").clicked() {
                        // Look for built driver in the project's build output
                        let driver_source = PathBuf::from("driver/build/focus_vision_pcvr");
                        match driver::install_driver(self.steamvr_dir.as_ref().unwrap(), &driver_source) {
                            Ok(()) => {
                                self.driver_installed = true;
                                self.driver_status = "Driver installed".to_string();
                                self.log("Driver installed successfully");
                            }
                            Err(e) => {
                                self.driver_status = format!("Install failed: {e}");
                                self.log(&format!("Driver install failed: {e}"));
                            }
                        }
                    }
                }
            }
        });

        ui.add_space(16.0);

        // Contextual setup hint — shows the next step based on current state
        let hint = if !self.driver_installed {
            Some("Next: Install the SteamVR driver above")
        } else if self.connection_status == ConnectionStatus::Disconnected {
            if self.devices.is_empty() {
                Some("Next: Connect Focus Vision via USB and deploy the APK (Deploy tab)")
            } else {
                Some("Next: Start SteamVR, then enter the PIN on your headset")
            }
        } else {
            None
        };
        if let Some(hint_text) = hint {
            ui.label(egui::RichText::new(hint_text).size(12.0).color(accent).italics());
            ui.add_space(8.0);
        }

        // PIN display
        ui.group(|ui| {
            ui.label(egui::RichText::new("Pairing PIN").size(13.0).color(text_muted));
            ui.add_space(8.0);

            let pin_text = if self.connection_status == ConnectionStatus::WaitingForPin {
                &self.pin_code
            } else if self.connection_status == ConnectionStatus::Connected {
                "Connected"
            } else {
                "----"
            };

            ui.label(
                egui::RichText::new(pin_text)
                    .size(48.0)
                    .monospace()
                    .color(if self.connection_status == ConnectionStatus::Connected {
                        accent
                    } else {
                        egui::Color32::from_rgb(232, 232, 236)
                    }),
            );

            if self.connection_status == ConnectionStatus::WaitingForPin {
                ui.label(egui::RichText::new("Enter this PIN on your headset").size(12.0).color(text_muted));
            }
        });

        ui.add_space(16.0);

        // Connection stats
        if self.connection_status == ConnectionStatus::Connected {
            ui.group(|ui| {
                ui.label(egui::RichText::new("Streaming").size(13.0).color(text_muted));
                ui.add_space(8.0);

                ui.columns(3, |cols| {
                    cols[0].vertical_centered(|ui| {
                        ui.label(egui::RichText::new(format!("{:.1}", self.latency_ms))
                            .size(24.0).monospace());
                        ui.label(egui::RichText::new("ms").size(11.0).color(text_muted));
                    });
                    cols[1].vertical_centered(|ui| {
                        ui.label(egui::RichText::new(format!("{}", self.fps))
                            .size(24.0).monospace());
                        ui.label(egui::RichText::new("fps").size(11.0).color(text_muted));
                    });
                    cols[2].vertical_centered(|ui| {
                        ui.label(egui::RichText::new(format!("{:.1}", self.bitrate_mbps))
                            .size(24.0).monospace());
                        ui.label(egui::RichText::new("Mbps").size(11.0).color(text_muted));
                    });
                });
            });
        }
    }

    fn render_deploy(&mut self, ui: &mut egui::Ui, accent: egui::Color32, text_muted: egui::Color32) {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Deploy to Headset").size(20.0));
        ui.label(egui::RichText::new("ADB経由でFocus VisionにAPKをインストール").size(13.0).color(text_muted));

        ui.add_space(16.0);

        // ADB status
        ui.group(|ui| {
            ui.label(egui::RichText::new("ADB").size(13.0).color(text_muted));
            match &self.adb_path {
                Some(path) => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("●").color(accent));
                        ui.label(format!("Found: {path}"));
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(248, 113, 113)));
                        ui.label("ADB not found. Install Android SDK Platform Tools.");
                    });
                }
            }
        });

        ui.add_space(8.0);

        // Connected devices
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Devices").size(13.0).color(text_muted));
                if ui.button("Refresh").clicked() {
                    self.last_device_scan = Instant::now() - Duration::from_secs(10);
                    self.scan_devices();
                }
            });

            if self.devices.is_empty() {
                ui.label(egui::RichText::new("No devices connected. Connect Focus Vision via USB and enable developer mode.").color(text_muted));
            } else {
                for device in &self.devices {
                    ui.horizontal(|ui| {
                        let color = if device.is_focus_vision { accent } else { text_muted };
                        ui.label(egui::RichText::new("●").color(color));
                        ui.label(format!("{} ({})", device.model, device.serial));
                        if device.is_focus_vision {
                            ui.label(egui::RichText::new("Focus Vision").color(accent).size(11.0));
                        }
                    });
                }
            }
        });

        ui.add_space(8.0);

        // APK path
        ui.group(|ui| {
            ui.label(egui::RichText::new("APK File").size(13.0).color(text_muted));
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.apk_path);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd_pick_file() {
                        self.apk_path = path;
                    }
                }
            });
        });

        ui.add_space(8.0);

        // Deploy button
        let can_deploy = self.adb_path.is_some()
            && !self.devices.is_empty()
            && !self.apk_path.is_empty()
            && std::path::Path::new(&self.apk_path).exists();

        let deploy_enabled = can_deploy && !self.deploy_in_progress;
        ui.add_enabled_ui(deploy_enabled, |ui| {
            let label = if self.deploy_in_progress {
                "Installing..."
            } else {
                "Install APK on All Devices"
            };
            if ui.button(
                egui::RichText::new(label).size(16.0)
            ).clicked() {
                self.deploy_in_progress = true;
                self.deploy_status = "Installing...".to_string();

                let adb = self.adb_path.clone().unwrap();
                let apk = self.apk_path.clone();
                let devices: Vec<_> = self.devices.iter().map(|d| d.serial.clone()).collect();
                let result = self.deploy_result.clone();

                thread::Builder::new()
                    .name("fvp-deploy".into())
                    .spawn(move || {
                        let mut outcomes = Vec::new();
                        for serial in &devices {
                            match adb::install_apk(&adb, serial, &apk) {
                                Ok(_) => {
                                    let _ = adb::launch_app(&adb, serial, "com.focusvision.pcvr");
                                    outcomes.push(format!("OK: {}", serial));
                                }
                                Err(e) => {
                                    outcomes.push(format!("FAIL {}: {}", serial, e));
                                }
                            }
                        }
                        let msg = outcomes.join(", ");
                        if let Ok(mut guard) = result.lock() {
                            *guard = Some(msg);
                        }
                    })
                    .expect("spawn deploy thread");
            }
        });

        if !self.deploy_status.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.deploy_status);
        }
    }

    fn render_settings(&mut self, ui: &mut egui::Ui, _accent: egui::Color32, text_muted: egui::Color32) {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Settings").size(20.0));

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Driver").size(13.0).color(text_muted));

            if self.driver_installed {
                if ui.button("Uninstall Driver").clicked() {
                    if let Some(ref dir) = self.steamvr_dir {
                        match driver::uninstall_driver(dir) {
                            Ok(()) => {
                                self.driver_installed = false;
                                self.log("Driver uninstalled");
                            }
                            Err(e) => {
                                self.log(&format!("Uninstall failed: {e}"));
                            }
                        }
                    }
                }
            }

            if let Some(ref dir) = self.steamvr_dir {
                ui.label(egui::RichText::new(format!("SteamVR: {}", dir.display())).size(11.0).color(text_muted));
            }
        });

        ui.add_space(16.0);

        // Audio settings
        ui.group(|ui| {
            ui.label(egui::RichText::new("Audio").size(13.0).color(text_muted));

            ui.checkbox(&mut self.audio_enabled, "Audio streaming enabled");

            if self.audio_enabled {
                ui.horizontal(|ui| {
                    ui.label("Bitrate:");
                    ui.add(egui::Slider::new(&mut self.audio_bitrate_kbps, 64..=256).suffix(" kbps"));
                });
                ui.label(egui::RichText::new("WASAPI loopback — no virtual device needed").size(11.0).color(text_muted));
            }
        });

        ui.add_space(16.0);

        // Log
        ui.group(|ui| {
            ui.label(egui::RichText::new("Log").size(13.0).color(text_muted));
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                if let Ok(log) = self.status_log.lock() {
                    for entry in log.iter().rev() {
                        ui.label(egui::RichText::new(entry).size(11.0).monospace().color(text_muted));
                    }
                }
            });
        });

        ui.add_space(16.0);

        ui.label(egui::RichText::new("Focus Vision PCVR v0.1.0").size(11.0).color(text_muted));
    }
}

/// Simple file picker fallback (no rfd crate — uses Windows common dialog via command)
fn rfd_pick_file() -> Option<String> {
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
