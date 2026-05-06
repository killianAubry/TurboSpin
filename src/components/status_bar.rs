// Status bar — minimal chrome only.
//
// Top: workspace folder (opens native picker) + simulator label +
// config button (Cmd+,) that opens the full config popup.
// Bottom: transient `status_message` only.

use egui::{CornerRadius, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::state::{AppState, SimulatorKind, StatusKind};
use crate::theme::{color, space};
use crate::workspace;

pub fn show_top(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.add_space(space::SM);

        let resp = folder_glyph(ui)
            .on_hover_text(workspace_tooltip(state));

        if resp.clicked() {
            workspace::pick_workspace_folder(state);
        }

        ui.add_space(space::MD);

        // Active simulator indicator
        let sim_label = state.simulator.label();
        ui.label(
            RichText::new(format!("● {}", sim_label))
                .color(color::ACCENT_YELLOW)
                .monospace()
                .size(12.0),
        );

        // Compare indicator
        if let Some(cmp) = state.compare_simulator {
            ui.add_space(space::SM);
            ui.label(RichText::new("vs").color(color::TEXT_DIM).monospace().size(11.0));
            ui.add_space(space::XS);
            ui.label(
                RichText::new(cmp.label())
                    .color(color::ACCENT_GREEN)
                    .monospace()
                    .size(12.0),
            );
        }

        // TurboSpin badges
        if state.simulator == SimulatorKind::TurboSpin {
            ui.add_space(space::SM);
            ui.label(
                RichText::new(format!("[{}]", state.turbospin_mode.label()))
                    .color(color::TEXT_MUTED)
                    .monospace()
                    .size(11.0),
            );
        }
        if state.simulator == SimulatorKind::TurboSpin {
            ui.add_space(space::XS);
            ui.label(
                RichText::new(format!("[{}]", state.turbospin_compression.label()))
                    .color(color::TEXT_DIM)
                    .monospace()
                    .size(11.0),
            );
        }

        // Noise status
        if state.noise_config.noise_enabled {
            ui.add_space(space::SM);
            ui.label(
                RichText::new("[noise]")
                    .color(color::ACCENT_GREEN)
                    .monospace()
                    .size(11.0),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Config button
            let btn_resp = ui.add(
                egui::Button::new(
                    RichText::new("config ⌘,")
                        .monospace()
                        .color(color::TEXT_MUTED)
                        .size(11.0),
                )
                .fill(color::BG_ELEVATED)
                .min_size(Vec2::new(90.0, 20.0)),
            );
            if btn_resp.clicked() {
                state.ui.config_popup_open = !state.ui.config_popup_open;
            }
        });
    });
}

fn workspace_tooltip(state: &AppState) -> String {
    format!(
        "{}\n{}\n⌘S save · ⌘O open .qasm… · ⌘, config",
        state.workspace_dir.display(),
        state.circuit_file_path().display(),
    )
}

/// Tab + body outline — reads as a folder at 14×11 px, stroke-only; click
/// opens the workspace directory picker.
fn folder_glyph(ui: &mut egui::Ui) -> egui::Response {
    let size = Vec2::new(14.0, 11.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter_at(rect);

    let stroke_color = if resp.hovered() {
        color::ACCENT_YELLOW
    } else {
        color::TEXT_MUTED
    };
    let stroke = Stroke::new(1.0, stroke_color);
    let skind = StrokeKind::Inside;

    let tab = egui::Rect::from_min_max(rect.min, rect.min + Vec2::new(7.0, 4.0));
    p.rect_stroke(tab, CornerRadius::same(1), stroke, skind);

    let body = egui::Rect::from_min_max(rect.min + Vec2::new(0.0, 3.0), rect.max);
    p.rect_stroke(body, CornerRadius::same(2), stroke, skind);

    resp
}

pub fn show_bottom(ui: &mut egui::Ui, state: &mut AppState) {
    if let Some((msg, kind)) = state.ui.status_message.as_ref() {
        let c = match kind {
            StatusKind::Info => color::TEXT_MUTED,
            StatusKind::Ok => color::ACCENT_YELLOW,
            StatusKind::Err => color::ACCENT_RED,
        };
        ui.horizontal(|ui| {
            ui.add_space(space::SM);
            ui.label(RichText::new(msg.as_str()).color(c).monospace().size(11.0));
        });
    } else {
        ui.allocate_space(Vec2::new(ui.available_width(), 1.0));
    }
}

/// When switching simulators, seed from the new mode's template only if the
/// buffer still matches the *previous* mode's starter template.
pub fn on_simulator_changed(state: &mut AppState, prev: SimulatorKind) {
    let was_template = state.editor_text == prev.default_template();
    if was_template {
        state.editor_text = state.simulator.default_template().to_string();
    }

    if state.compare_simulator == Some(state.simulator) {
        state.compare_simulator = None;
        state.compare_simulation = None;
    }

    state.ensure_synced();

    let s = state.simulator;
    let suffix = if was_template {
        " (template loaded)"
    } else {
        ""
    };
    state.ui.flash(
        format!(
            "{} → .qasm files{}{}  ⌘R to run",
            s.label(),
            suffix,
            match s {
                SimulatorKind::TurboSpin => {
                    " · use `:compress …` and `:tsmode …`"
                }
                _ => "",
            }
        ),
        StatusKind::Info,
    );
}
