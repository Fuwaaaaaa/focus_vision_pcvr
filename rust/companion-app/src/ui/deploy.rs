//! Deploy tab — ADB device list, APK picker, install button.

use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::{adb, file_dialog, headset_link, CompanionApp};

impl CompanionApp {
    pub(crate) fn render_deploy(&mut self, ui: &mut egui::Ui, accent: egui::Color32, text_muted: egui::Color32) {
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
                        } else {
                            ui.label(egui::RichText::new("not a VIVE headset — skipped").color(text_muted).size(11.0));
                        }
                    });
                }
            }
        });

        ui.add_space(8.0);

        // APK path
        ui.group(|ui| {
            ui.label(egui::RichText::new("APK File").size(13.0).color(text_muted));
            let prev_apk = self.apk_path.clone();
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.apk_path);
                let choosing = self.apk_dialog.is_some();
                let label = if choosing { "Choosing..." } else { "Browse..." };
                if ui.add_enabled(!choosing, egui::Button::new(label)).clicked() {
                    self.apk_dialog = Some(file_dialog::DialogTask::spawn(ui.ctx(), file_dialog::pick_apk));
                }
            });
            if let Some(answer) = self.apk_dialog.as_ref().and_then(|d| d.poll()) {
                self.apk_dialog = None;
                if let Some(path) = answer {
                    self.apk_path = path;
                }
            }

            // Debounced: typing a path saves once the input settles, not on
            // every keystroke.
            if self.apk_path != prev_apk {
                self.local_config.deploy.apk_path = self.apk_path.clone();
                self.mark_config_dirty("deploy", format!("APK path saved: {}", self.apk_path));
            }
        });

        ui.add_space(8.0);

        // Deploy button: the VIVE headsets only (adb::deploy_targets).
        let targets = adb::deploy_targets(&self.devices);
        let can_deploy = self.adb_path.is_some()
            && !targets.is_empty()
            && !self.apk_path.is_empty()
            && std::path::Path::new(&self.apk_path).exists();

        let deploy_enabled = can_deploy && !self.deploy_in_progress;
        ui.add_enabled_ui(deploy_enabled, |ui| {
            let label = if self.deploy_in_progress {
                "Installing...".to_string()
            } else if targets.len() > 1 {
                format!("Install APK on {} Headsets", targets.len())
            } else {
                "Install APK on the Headset".to_string()
            };
            if ui.button(
                egui::RichText::new(label).size(16.0)
            ).clicked() {
                self.deploy_in_progress = true;
                self.deploy_status = "Installing...".to_string();

                let adb = self.adb_path.clone().unwrap();
                let apk = self.apk_path.clone();
                let devices = targets.clone();
                let result = self.deploy_result.clone();
                // With the engine waiting for a PIN, start the app pointed at
                // this PC right away; otherwise just start it.
                let pin = self.pin_code.clone();
                let ports = self.engine_ports;

                thread::Builder::new()
                    .name("fvp-deploy".into())
                    .spawn(move || {
                        let mut outcomes = Vec::new();
                        for serial in &devices {
                            match adb::install_apk(&adb, serial, &apk) {
                                Ok(_) => {
                                    let launched = if headset_link::is_pin(&pin) {
                                        headset_link::send_to_headset(&adb, serial, &pin, ports)
                                    } else {
                                        adb::launch_app(&adb, serial, headset_link::CLIENT_PACKAGE)
                                    };
                                    outcomes.push(match launched {
                                        Ok(_) => format!("OK: {}", serial),
                                        Err(e) => format!("Installed on {}, but: {}", serial, e),
                                    });
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
}
