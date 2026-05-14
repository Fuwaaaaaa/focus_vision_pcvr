//! Settings tab — Driver / Audio / Sleep / FT / Recording / Codec / Diagnostics / Maintenance.
//!
//! Also owns the recording-dir validation helper, since it's used only from
//! this tab. Keeping the pure validator co-located with its caller makes
//! the tab independently scannable.

use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::{config, driver, export, CompanionApp};

/// Severity tier for the inline recording-dir diagnostic. Two levels keeps
/// the visual language simple — yellow for "you typed something the engine
/// will tolerate but might not be what you meant", red for "this can't work".
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum DirSeverity {
    Warning,
    Error,
}

/// Result of validating a user-typed recording output dir. None means the
/// input is fine (either blank → use default, or an existing directory).
#[derive(Debug)]
pub(crate) struct DirDiagnostic {
    pub severity: DirSeverity,
    pub message: &'static str,
}

/// Validate the recording output dir string the user typed into Settings.
///
/// Returns `None` when the input is acceptable:
/// - blank/whitespace: engine falls back to `%APPDATA%/FocusVisionPCVR/recordings`
/// - an existing directory
///
/// Returns `Some(...)` with a user-facing hint when:
/// - the path exists but is not a directory (Error)
/// - the path doesn't exist (Warning — engine creates it on startup)
pub(crate) fn recording_dir_diagnostic(path_str: &str) -> Option<DirDiagnostic> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => None,
        Ok(_) => Some(DirDiagnostic {
            severity: DirSeverity::Error,
            message: "Path exists but is not a directory",
        }),
        Err(_) => Some(DirDiagnostic {
            severity: DirSeverity::Warning,
            message: "Directory does not exist yet (engine will create it on startup)",
        }),
    }
}

impl CompanionApp {
    pub(crate) fn render_settings(&mut self, ui: &mut egui::Ui, _accent: egui::Color32, text_muted: egui::Color32) {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Settings").size(20.0));

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Driver").size(13.0).color(text_muted));

            if self.driver_installed
                && ui.button("Uninstall Driver").clicked() {
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

            if let Some(ref dir) = self.steamvr_dir {
                ui.label(egui::RichText::new(format!("SteamVR: {}", dir.display())).size(11.0).color(text_muted));
            }
        });

        ui.add_space(16.0);

        // Audio settings
        ui.group(|ui| {
            ui.label(egui::RichText::new("Audio").size(13.0).color(text_muted));

            let prev_audio_enabled = self.audio_enabled;
            let prev_audio_bitrate = self.audio_bitrate_kbps;
            ui.checkbox(&mut self.audio_enabled, "Audio streaming enabled");

            if self.audio_enabled {
                ui.horizontal(|ui| {
                    ui.label("Bitrate:");
                    ui.add(egui::Slider::new(&mut self.audio_bitrate_kbps, 64..=256).suffix(" kbps"));
                });
                ui.label(egui::RichText::new("WASAPI loopback — no virtual device needed").size(11.0).color(text_muted));
            }

            // Persist on change. Engine reads bitrate at startup, so a manual
            // engine restart is needed to pick up new values — the hint below
            // makes that explicit instead of leaving the user guessing why
            // the slider movement seems to do nothing in real time.
            if self.audio_enabled != prev_audio_enabled || self.audio_bitrate_kbps != prev_audio_bitrate {
                self.local_config.audio.enabled = self.audio_enabled;
                self.local_config.audio.bitrate_kbps = self.audio_bitrate_kbps;
                match self.local_config.save() {
                    Ok(()) => self.log(&format!("Audio: {} ({} kbps) — restart engine to apply",
                        if self.audio_enabled { "enabled" } else { "disabled" }, self.audio_bitrate_kbps)),
                    Err(e) => self.log(&format!("Failed to save audio config: {e}")),
                }
            }
        });

        ui.add_space(16.0);

        // Sleep Mode settings (v1.2)
        ui.group(|ui| {
            ui.label(egui::RichText::new("Sleep Mode").size(13.0).color(text_muted));

            let prev_enabled = self.sleep_enabled;
            let prev_timeout = self.sleep_timeout;
            ui.checkbox(&mut self.sleep_enabled, "Auto-sleep on inactivity");

            if self.sleep_enabled {
                ui.horizontal(|ui| {
                    ui.label("Timeout:");
                    ui.add(egui::Slider::new(&mut self.sleep_timeout, 30..=900).suffix("s"));
                });
                ui.label(egui::RichText::new(format!("{}m {}s — bitrate drops to 8 Mbps during sleep",
                    self.sleep_timeout / 60, self.sleep_timeout % 60)).size(11.0).color(text_muted));
            }

            if self.sleep_enabled != prev_enabled || self.sleep_timeout != prev_timeout {
                self.local_config.sleep_mode.enabled = self.sleep_enabled;
                self.local_config.sleep_mode.timeout_seconds = self.sleep_timeout;
                match self.local_config.save() {
                    Ok(()) => self.log(&format!("Sleep mode: {} (timeout {}s)",
                        if self.sleep_enabled { "enabled" } else { "disabled" }, self.sleep_timeout)),
                    Err(e) => self.log(&format!("Failed to save config: {e}")),
                }
            }
        });

        ui.add_space(16.0);

        // Face Tracking settings (v1.2)
        ui.group(|ui| {
            ui.label(egui::RichText::new("Face Tracking").size(13.0).color(text_muted));

            let prev_enabled = self.ft_enabled;
            let prev_smoothing = self.ft_smoothing;
            ui.checkbox(&mut self.ft_enabled, "Face tracking → VRChat OSC");

            if self.ft_enabled {
                ui.horizontal(|ui| {
                    ui.label("Smoothing:");
                    ui.add(egui::Slider::new(&mut self.ft_smoothing, 0.0..=0.95).fixed_decimals(2));
                });
                ui.label(egui::RichText::new("0.0 = raw, higher = smoother (reduces jitter)").size(11.0).color(text_muted));
            }

            if self.ft_enabled != prev_enabled || (self.ft_smoothing - prev_smoothing).abs() > 0.001 {
                self.local_config.face_tracking.enabled = self.ft_enabled;
                self.local_config.face_tracking.smoothing = self.ft_smoothing;
                match self.local_config.save() {
                    Ok(()) => self.log(&format!("Face tracking: {} (smoothing {:.2})",
                        if self.ft_enabled { "enabled" } else { "disabled" }, self.ft_smoothing)),
                    Err(e) => self.log(&format!("Failed to save config: {e}")),
                }
            }
        });

        ui.add_space(16.0);

        // Session Recording
        ui.group(|ui| {
            ui.label(egui::RichText::new("Session Recording").size(13.0).color(text_muted));

            let prev_rec_enabled = self.recording_enabled;
            let prev_rec_dir = self.recording_output_dir.clone();
            ui.checkbox(&mut self.recording_enabled, "Record sessions to disk (video + audio)");

            if self.recording_enabled {
                ui.horizontal(|ui| {
                    ui.label("Output dir:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.recording_output_dir)
                            .hint_text("(blank = %APPDATA%/FocusVisionPCVR/recordings)")
                            .desired_width(320.0),
                    );
                });

                // Inline directory validation. Blank is OK (engine falls back
                // to the default path). A typed path that doesn't exist yet
                // gets a yellow warning rather than a hard error, because
                // the engine creates missing dirs at startup — we just want
                // the user to catch typos before they record.
                if let Some(diag) = recording_dir_diagnostic(&self.recording_output_dir) {
                    let color = match diag.severity {
                        DirSeverity::Warning => egui::Color32::from_rgb(251, 191, 36),
                        DirSeverity::Error => egui::Color32::from_rgb(248, 113, 113),
                    };
                    ui.label(egui::RichText::new(diag.message).size(11.0).color(color));
                }

                ui.label(
                    egui::RichText::new("Video: raw Annex B (.h265/.h264) / Audio: 16-bit PCM (.wav). ffmpeg -i rec.h265 -i rec.wav -c:v copy rec.mp4")
                        .size(11.0).color(text_muted),
                );
                ui.label(
                    egui::RichText::new("Engine restart required for changes to take effect.")
                        .size(11.0).color(text_muted),
                );
            }

            if self.recording_enabled != prev_rec_enabled || self.recording_output_dir != prev_rec_dir {
                self.local_config.recording.enabled = self.recording_enabled;
                self.local_config.recording.output_dir = self.recording_output_dir.clone();
                match self.local_config.save() {
                    Ok(()) => self.log(&format!("Recording: {} (dir: {})",
                        if self.recording_enabled { "enabled" } else { "disabled" },
                        if self.recording_output_dir.is_empty() { "<default>" } else { &self.recording_output_dir })),
                    Err(e) => self.log(&format!("Failed to save config: {e}")),
                }
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

        // Codec selection (v1.1)
        ui.group(|ui| {
            ui.label(egui::RichText::new("Video Codec").size(13.0).color(text_muted));
            let prev = self.selected_codec.clone();
            ui.radio_value(&mut self.selected_codec, "h265".to_string(), "H.265 (HEVC) — higher quality");
            ui.radio_value(&mut self.selected_codec, "h264".to_string(), "H.264 — lower latency (2-5ms)");
            if self.selected_codec != prev {
                self.local_config.video.codec = self.selected_codec.clone();
                match self.local_config.save() {
                    Ok(()) => self.log(&format!("Codec set to {}. Restart engine to apply.", self.selected_codec)),
                    Err(e) => self.log(&format!("Failed to save config: {e}")),
                }
            }
        });

        ui.add_space(16.0);

        // Log export (v1.1)
        ui.group(|ui| {
            ui.label(egui::RichText::new("Diagnostics").size(13.0).color(text_muted));
            let export_enabled = !self.export_in_progress;
            ui.add_enabled_ui(export_enabled, |ui| {
                let label = if self.export_in_progress { "Exporting..." } else { "Export Logs (zip)" };
                if ui.button(label).clicked() {
                    self.export_in_progress = true;
                    let adb = self.adb_path.clone();
                    let serial = self.devices.first().map(|d| d.serial.clone());
                    let result = self.export_result.clone();
                    thread::Builder::new()
                        .name("fvp-export".into())
                        .spawn(move || {
                            let msg = match export::export_logs(
                                adb.as_deref(),
                                serial.as_deref(),
                            ) {
                                Ok(path) => format!("Logs exported: {}", path.display()),
                                Err(e) => format!("Export failed: {e}"),
                            };
                            if let Ok(mut guard) = result.lock() {
                                *guard = Some(msg);
                            }
                        })
                        .expect("spawn export thread");
                }
            });
            ui.label(egui::RichText::new("PC logs + HMD logcat + system info → zip").size(11.0).color(text_muted));
        });

        ui.add_space(16.0);

        // Reset to defaults — wipes every UI-side override (codec, audio,
        // sleep, FT, recording, deploy.apk_path) back to factory settings.
        // Two-stage confirm avoids misclicks: the button flips to "Confirm
        // reset" for 3 s, then reverts. Engine-side settings only re-apply
        // after the next engine restart, mirrored elsewhere in this tab.
        ui.group(|ui| {
            ui.label(egui::RichText::new("Maintenance").size(13.0).color(text_muted));
            let now = Instant::now();
            let confirming = self
                .reset_confirm_until
                .map(|t| t > now)
                .unwrap_or(false);

            let (label, btn_color) = if confirming {
                ("Confirm reset", egui::Color32::from_rgb(248, 113, 113))
            } else {
                ("Reset to defaults", egui::Color32::from_rgb(232, 232, 236))
            };

            if ui
                .button(egui::RichText::new(label).color(btn_color))
                .clicked()
            {
                if confirming {
                    self.reset_to_defaults();
                    self.reset_confirm_until = None;
                } else {
                    self.reset_confirm_until = Some(now + Duration::from_secs(3));
                }
            }

            ui.label(
                egui::RichText::new(
                    "All companion-side overrides (codec, audio, sleep, FT, recording, APK path) revert to defaults. Engine restart required for engine-side changes.",
                )
                .size(11.0)
                .color(text_muted),
            );
        });

        ui.add_space(16.0);

        ui.label(egui::RichText::new(format!("Focus Vision PCVR v{}", env!("CARGO_PKG_VERSION"))).size(11.0).color(text_muted));
    }

    /// Wipe LocalConfig back to defaults, persist, and re-sync every UI
    /// shadow field so the on-screen sliders/toggles match what was saved.
    /// Failure to save is non-fatal: we log it but still apply the in-memory
    /// reset, so the user at least gets immediate visual feedback.
    pub(crate) fn reset_to_defaults(&mut self) {
        self.local_config = config::LocalConfig::default();

        self.selected_codec = self.local_config.video.codec.clone();
        self.sleep_enabled = self.local_config.sleep_mode.enabled;
        self.sleep_timeout = self.local_config.sleep_mode.timeout_seconds;
        self.ft_enabled = self.local_config.face_tracking.enabled;
        self.ft_smoothing = self.local_config.face_tracking.smoothing;
        self.recording_enabled = self.local_config.recording.enabled;
        self.recording_output_dir = self.local_config.recording.output_dir.clone();
        self.audio_enabled = self.local_config.audio.enabled;
        self.audio_bitrate_kbps = self.local_config.audio.bitrate_kbps;
        self.apk_path = self.local_config.deploy.apk_path.clone();

        match self.local_config.save() {
            Ok(()) => self.log("Settings reset to defaults"),
            Err(e) => self.log(&format!("Reset applied in-memory but save failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_recording_dir_is_accepted() {
        assert!(recording_dir_diagnostic("").is_none());
        assert!(recording_dir_diagnostic("   ").is_none());
        assert!(recording_dir_diagnostic("\t \n").is_none());
    }

    #[test]
    fn existing_directory_returns_no_diagnostic() {
        // The cargo target dir is guaranteed to exist during `cargo test`,
        // so we use the project root (CARGO_MANIFEST_DIR points at the
        // package, which is always a real dir).
        let dir = env!("CARGO_MANIFEST_DIR");
        assert!(recording_dir_diagnostic(dir).is_none());
    }

    #[test]
    fn nonexistent_path_yields_warning() {
        let diag = recording_dir_diagnostic(r"C:\definitely-not-a-real-path-12345\nope")
            .expect("should warn about missing dir");
        assert_eq!(diag.severity, DirSeverity::Warning);
        assert!(diag.message.contains("does not exist"));
    }

    #[test]
    fn file_instead_of_dir_yields_error() {
        // The companion-app's own Cargo.toml is a file, not a dir. If a user
        // points the recording dir at a file (e.g. by mistake selecting an
        // existing config file), we surface an error.
        let cargo_toml = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let cargo_toml_str = cargo_toml.to_string_lossy();
        let diag = recording_dir_diagnostic(&cargo_toml_str)
            .expect("should error on file-as-dir");
        assert_eq!(diag.severity, DirSeverity::Error);
        assert!(diag.message.contains("not a directory"));
    }
}
