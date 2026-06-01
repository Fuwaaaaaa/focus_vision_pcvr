//! Home tab — engine banner, driver status, PIN display, live stats.

use std::path::PathBuf;

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

use crate::driver;
use crate::stats_history;
use crate::status_parser::ConnectionStatus;
use crate::CompanionApp;

impl CompanionApp {
    pub(crate) fn render_home(&mut self, ui: &mut egui::Ui, accent: egui::Color32, text_muted: egui::Color32) {
        ui.add_space(16.0);

        // Brand
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Focus").size(32.0));
            ui.label(egui::RichText::new("Vision").size(32.0).color(accent).italics());
        });
        ui.label(egui::RichText::new("PCVR Streaming").size(14.0).color(text_muted));

        ui.add_space(16.0);

        // Integrated simulation control. Only compiled into the `simulator`
        // build, and hidden in demo mode (demo is a separate fake path).
        // Lets the user run the full pipeline locally with no VR hardware.
        #[cfg(feature = "simulator")]
        if !self.demo_mode {
            let blue = egui::Color32::from_rgb(96, 165, 250);
            ui.group(|ui| {
                if self.is_simulating() {
                    if ui
                        .button(egui::RichText::new("■ Stop Simulation").color(blue).strong())
                        .clicked()
                    {
                        self.stop_sim();
                    }
                    ui.label(
                        egui::RichText::new("ローカルエンジン稼働中 — 実機なしでフルパイプライン実行中")
                            .size(12.0)
                            .color(text_muted),
                    );
                } else {
                    if ui
                        .button(egui::RichText::new("▶ Start Simulation").color(blue).strong())
                        .clicked()
                    {
                        self.start_sim();
                    }
                    ui.label(
                        egui::RichText::new("実機なしでパイプライン全体をローカル実行（ヘッドセット不要）")
                            .size(12.0)
                            .color(text_muted),
                    );
                }
                if let Some(err) = &self.sim_error {
                    ui.label(
                        egui::RichText::new(err)
                            .size(12.0)
                            .color(egui::Color32::from_rgb(248, 113, 113)),
                    );
                }
            });
            ui.add_space(8.0);
        }

        // Engine-died banner. Surfaced when status.json is missing or its
        // mtime is older than ENGINE_STALE_THRESHOLD — the engine has
        // either not started, was killed, or SteamVR crashed.
        //
        // Note: the companion app CANNOT restart the engine itself because
        // the engine lives inside vrserver.exe (loaded as a SteamVR driver
        // staticlib). Only SteamVR's process-lifecycle restart works, so
        // the banner directs the user there instead of offering a button.
        // Suppressed while an in-process simulation is running (the sim writes
        // a real status.json that lights the UI up within ~1 s of start).
        if !self.engine_alive && !self.is_simulating() {
            let red = egui::Color32::from_rgb(248, 113, 113);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("●").color(red).size(16.0));
                    ui.label(
                        egui::RichText::new("ストリーミングエンジンが停止しています")
                            .color(red)
                            .strong(),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "SteamVR を起動 (または再起動) するとエンジンが立ち上がります。\
                         SteamVR が動作中でもこの表示が消えない場合は、Settings タブの\
                         Export Logs から診断情報を収集してください。",
                    )
                    .size(12.0)
                    .color(text_muted),
                );
            });
            ui.add_space(8.0);
        }

        ui.add_space(8.0);

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

                // Live countdown — locally derived so we don't depend on
                // the engine rewriting status.json every second. Only
                // renders when the engine actually emitted an expiry
                // value (None = old engine = no countdown).
                if let (Some(baseline), Some(observed)) =
                    (self.pin_expires_in_seconds, self.pin_expires_observed_at)
                {
                    let elapsed = observed.elapsed().as_secs() as u32;
                    let remaining = baseline.saturating_sub(elapsed);
                    let mins = remaining / 60;
                    let secs = remaining % 60;
                    let countdown_color = if remaining < 30 {
                        egui::Color32::from_rgb(248, 113, 113) // urgency red
                    } else if remaining < 60 {
                        egui::Color32::from_rgb(251, 191, 36) // warning yellow
                    } else {
                        text_muted
                    };
                    ui.label(
                        egui::RichText::new(format!("Expires in {}:{:02}", mins, secs))
                            .size(12.0)
                            .color(countdown_color)
                            .monospace(),
                    );
                }
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

            // Sparkline graphs (30s history)
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Latency (30s)").size(11.0).color(text_muted));
                let points = stats_history::StatsHistory::as_plot_points(&self.stats_history.latency_ms);
                Plot::new("latency_spark")
                    .height(50.0)
                    .show_axes(false)
                    .show_grid(false)
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .show(ui, |plot_ui| {
                        plot_ui.line(Line::new(PlotPoints::new(points)).color(accent));
                    });

                ui.label(egui::RichText::new("FPS (30s)").size(11.0).color(text_muted));
                let fps_points = stats_history::StatsHistory::as_plot_points(&self.stats_history.fps);
                Plot::new("fps_spark")
                    .height(50.0)
                    .show_axes(false)
                    .show_grid(false)
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .show(ui, |plot_ui| {
                        plot_ui.line(Line::new(PlotPoints::new(fps_points)).color(accent));
                    });
            });

            // Subsystem status indicators
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Subsystems").size(13.0).color(text_muted));
                ui.add_space(4.0);

                let active_color = accent;
                let idle_color = text_muted;

                ui.horizontal(|ui| {
                    let (ft_color, ft_label) = if self.sub_ft_active {
                        (active_color, "FT Active")
                    } else {
                        (idle_color, "FT Idle")
                    };
                    ui.label(egui::RichText::new("●").size(11.0).color(ft_color));
                    ui.label(egui::RichText::new(ft_label).size(11.0).color(ft_color));

                    ui.add_space(16.0);

                    let (sleep_color, sleep_label) = if self.sub_sleep_active {
                        (idle_color, "Sleep")
                    } else {
                        (active_color, "Awake")
                    };
                    ui.label(egui::RichText::new("●").size(11.0).color(sleep_color));
                    ui.label(egui::RichText::new(sleep_label).size(11.0).color(sleep_color));

                    ui.add_space(16.0);

                    let (audio_color, audio_label) = if self.sub_audio_enabled {
                        (active_color, "Audio OK")
                    } else {
                        (idle_color, "Audio Off")
                    };
                    ui.label(egui::RichText::new("●").size(11.0).color(audio_color));
                    ui.label(egui::RichText::new(audio_label).size(11.0).color(audio_color));

                    ui.add_space(16.0);

                    let loss_color = if self.sub_packet_loss > 5.0 {
                        egui::Color32::from_rgb(248, 113, 113) // error red
                    } else if self.sub_packet_loss > 2.0 {
                        egui::Color32::from_rgb(251, 191, 36) // warning yellow
                    } else {
                        text_muted
                    };
                    ui.label(egui::RichText::new(format!("Loss {:.1}%", self.sub_packet_loss))
                        .size(11.0).color(loss_color));
                });
            });
        }

        // Recent activity log tail. Collapsed by default so the Home tab
        // stays calm — when something needs explaining, the user can pop
        // it open. Last 10 lines fits one screen on the default 480x640
        // window with the log's tendency to one-line entries.
        ui.add_space(12.0);
        egui::CollapsingHeader::new(
            egui::RichText::new("Recent activity").size(12.0).color(text_muted),
        )
        .default_open(false)
        .show(ui, |ui| {
            let entries: Vec<String> = self
                .status_log
                .lock()
                .map(|log| log.iter().rev().take(10).rev().cloned().collect())
                .unwrap_or_default();
            if entries.is_empty() {
                ui.label(
                    egui::RichText::new("(no events yet)")
                        .size(11.0)
                        .color(text_muted)
                        .italics(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for entry in entries.iter() {
                            ui.label(egui::RichText::new(entry).size(11.0).monospace());
                        }
                    });
            }
        });
    }
}
