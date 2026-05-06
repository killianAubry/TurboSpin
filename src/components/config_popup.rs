// Config popup overlay (Cmd+,).
//
// Modal popup containing all simulator and noise configuration in one place.
// Replaces the status-bar combo boxes with a clean, card-based layout.
// Sections: primary simulator, compare simulator, TurboSpin settings,
// Qiskit backend config, noise quick toggles.

use egui::{Align2, CornerRadius, Key, RichText};

use crate::state::{AppState, SimulatorKind, TurboSpinCompression, TurboSpinMode};
use crate::theme::{color, space};

const POPUP_WIDTH: f32 = 480.0;

pub fn show(ctx: &egui::Context, state: &mut AppState) -> bool {
    if !state.ui.config_popup_open {
        return false;
    }

    let screen = ctx.content_rect();

    egui::Area::new(egui::Id::new("config_popup"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, screen.height() * 0.08))
        .show(ctx, |ui| {
            let frame = egui::Frame::NONE
                .fill(color::BG_PANEL)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::same(space::LG as i8));

            frame.show(ui, |ui| {
                ui.set_width(POPUP_WIDTH);

                // ── Header ──
                ui.horizontal(|ui| {
                    ui.label(RichText::new("configuration").color(color::TEXT_MUTED).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("esc to close").color(color::TEXT_DIM).monospace().size(11.0));
                    });
                });
                ui.add_space(space::MD);

                // ── Primary simulator ──
                section_card(ui, "primary simulator", |ui| {
                    sim_selector(ui, &mut state.simulator, "primary_sim");
                    if state.simulator == SimulatorKind::TurboSpin {
                        ui.add_space(space::SM);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("mode").color(color::TEXT_MUTED).monospace().size(11.0));
                            ui.add_space(space::SM);
                            egui::ComboBox::new("cfg_ts_mode", "")
                                .selected_text(RichText::new(state.turbospin_mode.label()).monospace().color(color::TEXT_PRIMARY).text().to_string())
                                .show_ui(ui, |ui| {
                                    for &m in TurboSpinMode::ALL {
                                        ui.selectable_value(&mut state.turbospin_mode, m, m.label());
                                    }
                                });
                        });
                    }
                    if state.simulator == SimulatorKind::TurboSpin {
                        ui.add_space(space::SM);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("compress").color(color::TEXT_MUTED).monospace().size(11.0));
                            ui.add_space(space::SM);
                            egui::ComboBox::new("cfg_ts_comp", "")
                                .selected_text(RichText::new(state.turbospin_compression.label()).monospace().color(color::TEXT_PRIMARY).text().to_string())
                                .show_ui(ui, |ui| {
                                    for &c in TurboSpinCompression::ALL {
                                        ui.selectable_value(&mut state.turbospin_compression, c, c.label());
                                    }
                                });
                        });
                    }
                });
                ui.add_space(space::MD);

                // ── Compare simulator ──
                section_card(ui, "compare against", |ui| {
                    let mut current = state.compare_simulator;
                    ui.horizontal(|ui| {
                        if ui.selectable_label(current.is_none(), "off").clicked() {
                            state.compare_simulator = None;
                            state.compare_simulation = None;
                        }
                        for &kind in SimulatorKind::ALL {
                            if kind == state.simulator {
                                continue;
                            }
                            if ui.selectable_label(current == Some(kind), kind.label()).clicked() {
                                current = Some(kind);
                            }
                        }
                    });
                    if current != state.compare_simulator {
                        state.compare_simulator = current;
                        state.compare_simulation = None;
                    }
                    if let Some(cmp) = state.compare_simulator {
                        if cmp == SimulatorKind::TurboSpin {
                            ui.add_space(space::SM);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("mode").color(color::TEXT_MUTED).monospace().size(11.0));
                                ui.add_space(space::SM);
                                egui::ComboBox::new("cfg_cmp_ts_mode", "")
                                    .selected_text(RichText::new(state.compare_turbospin_mode.label()).monospace().color(color::ACCENT_GREEN).text().to_string())
                                    .show_ui(ui, |ui| {
                                        for &m in TurboSpinMode::ALL {
                                            ui.selectable_value(&mut state.compare_turbospin_mode, m, m.label());
                                        }
                                    });
                            });
                        }
                        if cmp == SimulatorKind::TurboSpin {
                            ui.add_space(space::SM);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("compress").color(color::TEXT_MUTED).monospace().size(11.0));
                                ui.add_space(space::SM);
                                egui::ComboBox::new("cfg_cmp_ts_comp", "")
                                    .selected_text(RichText::new(state.compare_compression.label()).monospace().color(color::ACCENT_GREEN).text().to_string())
                                    .show_ui(ui, |ui| {
                                        for &c in TurboSpinCompression::ALL {
                                            ui.selectable_value(&mut state.compare_compression, c, c.label());
                                        }
                                    });
                            });
                        }
                    }
                });
                ui.add_space(space::MD);

                // ── Qiskit backend config ──
                section_card(ui, "qiskit backend", |ui| {
                    ui.label(
                        RichText::new("Qiskit processes OpenQASM circuits via python3.")
                            .color(color::TEXT_DIM)
                            .monospace()
                            .size(11.0),
                    );
                    ui.add_space(space::XS);
                    ui.label(
                        RichText::new("Set QISKIT_PYTHON env var to change python path.")
                            .color(color::TEXT_DIM)
                            .monospace()
                            .size(10.0),
                    );
                    let python = std::env::var("QISKIT_PYTHON").unwrap_or_else(|_| "python3".to_string());
                    ui.add_space(space::XS);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("python:").color(color::TEXT_MUTED).monospace().size(11.0));
                        ui.add_space(space::SM);
                        ui.label(RichText::new(&python).color(color::ACCENT_GREEN).monospace().size(11.0));
                    });
                });
                ui.add_space(space::MD);

                // ── Noise quick settings ──
                section_card(ui, "noise quick settings", |ui| {
                    let cfg = &mut state.noise_config;
                    ui.horizontal(|ui| {
                        let resp = ui.selectable_label(cfg.noise_enabled, "");
                        if resp.clicked() {
                            cfg.noise_enabled = !cfg.noise_enabled;
                        }
                        ui.add_space(space::SM);
                        let label = if cfg.noise_enabled {
                            RichText::new("noise enabled").color(color::ACCENT_GREEN).monospace()
                        } else {
                            RichText::new("noise disabled").color(color::TEXT_DIM).monospace()
                        };
                        ui.label(label);
                    });
                    if cfg.noise_enabled {
                        ui.add_space(space::SM);
                        ui.horizontal(|ui| {
                            if ui.selectable_label(cfg.depolarizing_enabled, "").clicked() {
                                cfg.depolarizing_enabled = !cfg.depolarizing_enabled;
                            }
                            ui.label(RichText::new("depol").color(if cfg.depolarizing_enabled { color::ACCENT_YELLOW } else { color::TEXT_DIM }).monospace().size(11.0));
                            ui.add_space(space::MD);
                            if ui.selectable_label(cfg.amplitude_damping_enabled, "").clicked() {
                                cfg.amplitude_damping_enabled = !cfg.amplitude_damping_enabled;
                            }
                            ui.label(RichText::new("T1").color(if cfg.amplitude_damping_enabled { color::ACCENT_YELLOW } else { color::TEXT_DIM }).monospace().size(11.0));
                            ui.add_space(space::MD);
                            if ui.selectable_label(cfg.phase_damping_enabled, "").clicked() {
                                cfg.phase_damping_enabled = !cfg.phase_damping_enabled;
                            }
                            ui.label(RichText::new("T2").color(if cfg.phase_damping_enabled { color::ACCENT_YELLOW } else { color::TEXT_DIM }).monospace().size(11.0));
                        });
                    }
                    ui.add_space(space::XS);
                    ui.label(
                        RichText::new("open noise panel for detailed control")
                            .color(color::TEXT_DIM)
                            .monospace()
                            .size(10.0),
                    );
                });
            });
        });

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        state.ui.config_popup_open = false;
    }

    true
}

fn sim_selector(ui: &mut egui::Ui, sim: &mut SimulatorKind, _id: &str) {
    ui.horizontal(|ui| {
        for &kind in SimulatorKind::ALL {
            let sel = ui.selectable_label(*sim == kind, kind.label());
            if sel.clicked() {
                let prev = *sim;
                *sim = kind;
                if prev != kind {
                    // Template swap handled by caller
                }
            }
        }
    });
}

fn section_card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(color::BG_ELEVATED)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(space::MD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).color(color::TEXT_MUTED).monospace().size(11.0));
            ui.add_space(space::SM);
            add_contents(ui);
        });
}
