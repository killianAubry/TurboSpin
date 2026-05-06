// Command palette overlay (Cmd+E).
//
// A minimal modal: title, monospace input, hint line. Submitting executes a
// single-line command against `AppState`. The command surface is a thin shim
// over the same primitives the keybinds use, so users can do things that
// would otherwise need a button.

use egui::{Align2, CornerRadius, Key, RichText, Stroke, StrokeKind};

use crate::state::{AppState, SimulatorKind, StatusKind, TurboSpinCompression, TurboSpinMode};
use crate::theme::{color, space};
use crate::tiling::{auto_split_dir, ViewKind};

const HINTS: &[&str] = &[
    ":sim <name>          qiskit | turbospin",
    ":open <view>         circuit | prob | sv | bloch | editor | noise | fid | ent | dm",
    ":compare <name|off>  compare against another simulator (or off)",
    ":save                write circuit.<ext> to workspace folder",
    ":load                read circuit.<ext> from workspace folder",
    ":close               close the focused tile",
    ":run                 run through selected sim(s) — also ⌘R",
    ":compress <exact|1..8> primary turbospin bit depth",
    ":tsmode <bacqs|rpdq> turbospin compression mode",
    ":compare_compress <exact|1..8>  compare turbospin bit depth",
    ":compare_tsmode <bacqs|rpdq> compare turbospin mode",
    ":reset               replace the buffer with the current mode's template",
    ":clear               clear the editor",
    ":config              open config popup — also ⌘,",
    ":help                show this list in the status line",
];

/// Returns `true` if the palette is open (so callers can suppress global
/// keybinds while a command is being typed).
pub fn show(ctx: &egui::Context, state: &mut AppState, focused_tile_rect: egui::Rect) -> bool {
    if !state.ui.cmd_palette_open {
        return false;
    }

    let screen = ctx.content_rect();
    let width = (screen.width() * 0.55).clamp(420.0, 720.0);

    egui::Area::new(egui::Id::new("cmd_palette"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, screen.height() * 0.18))
        .show(ctx, |ui| {
            let frame = egui::Frame::NONE
                .fill(color::BG_PANEL)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::same(space::LG as i8));

            frame.show(ui, |ui| {
                ui.set_width(width);

                ui.horizontal(|ui| {
                    ui.label(RichText::new(":").color(color::ACCENT_PURPLE).monospace());
                    let edit = egui::TextEdit::singleline(&mut state.ui.cmd_palette_input)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("type a command, then Enter…");
                    let resp = ui.add(edit);
                    resp.request_focus();

                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        let cmd = std::mem::take(&mut state.ui.cmd_palette_input);
                        execute(state, cmd.trim(), focused_tile_rect);
                        state.ui.cmd_palette_open = false;
                    }
                });

                ui.add_space(space::SM);
                draw_hairline(ui);
                ui.add_space(space::SM);

                for hint in HINTS {
                    ui.label(RichText::new(*hint).monospace().color(color::TEXT_MUTED));
                }
                ui.add_space(space::XS);
                ui.label(
                    RichText::new("esc to close · ⌘E to toggle")
                        .monospace()
                        .color(color::TEXT_DIM),
                );
            });
        });

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        state.ui.cmd_palette_open = false;
        state.ui.cmd_palette_input.clear();
    }

    true
}

fn draw_hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 1.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(0),
        Stroke::new(1.0, color::GRID_LINE),
        StrokeKind::Inside,
    );
}

fn execute(state: &mut AppState, cmd: &str, focused_rect: egui::Rect) {
    let cmd = cmd.strip_prefix(':').unwrap_or(cmd).trim();
    if cmd.is_empty() {
        return;
    }
    let mut parts = cmd.split_whitespace();
    let head = parts.next().unwrap_or("").to_lowercase();

    match head.as_str() {
        "help" | "?" => {
            state.ui.flash("see the palette hint list", StatusKind::Info);
        }
        "run" => match state.rerun() {
            Ok(()) => {
                let l = state.simulator.label();
                if let Some(cmp) = state.compare_simulator {
                    state.ui.flash(format!("run ({l} vs {}) ok", cmp.label()), StatusKind::Ok);
                } else {
                    state.ui.flash(format!("run ({l}) ok"), StatusKind::Ok);
                }
            }
            Err(e) => state.ui.flash(e, StatusKind::Err),
        },
        "compress" | "tsbits" | "tscomp" => match parts.next().and_then(TurboSpinCompression::from_str) {
            Some(choice) => {
                state.turbospin_compression = choice;
                let suffix =
                    if state.simulator == SimulatorKind::TurboSpin {
                        "  ⌘R to rerun"
                    } else {
                        "  (applied when primary is turbospin)"
                    };
                state.ui.flash(
                    format!("primary turbospin bit depth: {}{suffix}", choice.label()),
                    StatusKind::Ok,
                );
            }
            None => state
                .ui
                .flash("usage: :compress <exact|1..8>", StatusKind::Err),
        },
        "compare_compress" | "vcompress" | "vstsbits" => {
            match parts.next().and_then(TurboSpinCompression::from_str) {
                Some(choice) => {
                    state.compare_compression = choice;
                    let ok_cmp = matches!(
                        state.compare_simulator,
                        Some(SimulatorKind::TurboSpin)
                    );
                    let suffix = if ok_cmp {
                        "  ⌘R to rerun"
                    } else {
                        "  (applied when vs is turbospin)"
                    };
                    state.ui.flash(
                        format!("compare turbospin bit depth: {}{suffix}", choice.label()),
                        StatusKind::Ok,
                    );
                }
                None => state.ui.flash(
                    "usage: :compare_compress <exact|1..8>",
                    StatusKind::Err,
                ),
            }
        },
        "tsmode" | "mode" => match parts.next().and_then(TurboSpinMode::from_str) {
            Some(mode) => {
                state.turbospin_mode = mode;
                let suffix = if state.simulator == SimulatorKind::TurboSpin {
                    "  ⌘R to rerun"
                } else {
                    "  (applied when primary is turbospin)"
                };
                state.ui.flash(
                    format!("turbospin mode: {}{suffix}", mode.label()),
                    StatusKind::Ok,
                );
            }
            None => state
                .ui
                .flash("usage: :tsmode <bacqs|rpdq>", StatusKind::Err),
        },
        "compare_tsmode" | "vsmode" => match parts.next().and_then(TurboSpinMode::from_str) {
            Some(mode) => {
                state.compare_turbospin_mode = mode;
                let suffix = if state.compare_simulator == Some(SimulatorKind::TurboSpin) {
                    "  ⌘R to rerun"
                } else {
                    "  (applied when vs is turbospin)"
                };
                state.ui.flash(
                    format!("compare turbospin mode: {}{suffix}", mode.label()),
                    StatusKind::Ok,
                );
            }
            None => state
                .ui
                .flash("usage: :compare_tsmode <bacqs|rpdq>", StatusKind::Err),
        },
        "config" | "cfg" => {
            state.ui.config_popup_open = !state.ui.config_popup_open;
            let status = if state.ui.config_popup_open { "opened" } else { "closed" };
            state.ui.flash(format!("config {status}"), StatusKind::Info);
        }
        "clear" => {
            state.editor_text.clear();
            state.ensure_synced();
            state.ui.flash("editor cleared", StatusKind::Ok);
        }
        "reset" => {
            state.load_default_template();
            state.ui.flash(
                format!("reset to {} template", state.simulator.label()),
                StatusKind::Ok,
            );
        }
        "sim" => match parts.next().and_then(SimulatorKind::from_str) {
            Some(s) if s == state.simulator => {
                state
                    .ui
                    .flash(format!("already on {}", s.label()), StatusKind::Info);
            }
            Some(s) => {
                let prev = state.simulator;
                let was_template = state.editor_text == prev.default_template();
                state.simulator = s;
                if was_template {
                    state.editor_text = s.default_template().to_string();
                }
                state.ensure_synced();
                state.ui.flash(
                    format!(
                        "simulator: {} (.{})",
                        s.label(),
                        s.circuit_extension()
                    ),
                    StatusKind::Ok,
                );
            }
            None => state
                .ui
                .flash("usage: :sim <qiskit|turbospin>", StatusKind::Err),
        },
        "open" => match parts.next().and_then(parse_view_kind) {
            Some(v) => {
                let dir = auto_split_dir(focused_rect.size());
                let new_id = state.tiles.split_focused(state.focused_tile, v, dir);
                if let Some(id) = new_id {
                    state.focused_tile = id;
                    state.ui.flash(format!("opened {} tile", v.label()), StatusKind::Ok);
                } else {
                    state.ui.flash("could not open tile", StatusKind::Err);
                }
            }
            None => state
                .ui
                .flash("usage: :open <circuit|prob|sv|bloch|editor>", StatusKind::Err),
        },
        "save" => match state.save_circuit_file() {
            Ok(()) => {
                let p = state.circuit_file_path();
                state
                    .ui
                    .flash(format!("saved {}", p.display()), StatusKind::Ok);
            }
            Err(e) => state.ui.flash(e, StatusKind::Err),
        },
        "load" => match state.load_circuit_file() {
            Ok(()) => {
                let p = state.circuit_file_path();
                state
                    .ui
                    .flash(format!("loaded {}", p.display()), StatusKind::Ok);
            }
            Err(e) => state.ui.flash(e, StatusKind::Err),
        },
        "compare" | "cmp" => {
            let arg = parts.next().unwrap_or("");
            if arg.is_empty() || arg == "off" || arg == "none" {
                state.compare_simulator = None;
                state.compare_simulation = None;
                state.ui.flash("compare off", StatusKind::Ok);
            } else if let Some(s) = SimulatorKind::from_str(arg) {
                if s == state.simulator {
                    state.ui.flash(
                        format!("cannot compare {} against itself", s.label()),
                        StatusKind::Err,
                    );
                } else {
                    state.compare_simulator = Some(s);
                    state.compare_simulation = None;
                    state.ui.flash(
                        format!("compare: {} vs {}  ⌘R to rerun", state.simulator.label(), s.label()),
                        StatusKind::Ok,
                    );
                }
            } else {
                state.ui.flash(
                    "usage: :compare <qiskit|turbospin|off>",
                    StatusKind::Err,
                );
            }
        }
        "close" => match state.tiles.close_focused(state.focused_tile) {
            crate::tiling::CloseResult::Closed(id) => {
                state.focused_tile = id;
                state.ui.flash("tile closed", StatusKind::Ok);
            }
            crate::tiling::CloseResult::WasOnlyLeaf => {
                state.ui.flash("can't close — last tile", StatusKind::Err);
            }
            crate::tiling::CloseResult::NotFound => {
                state.ui.flash("focused tile not found", StatusKind::Err);
            }
        },
        other => state
            .ui
            .flash(format!("unknown command: {other}"), StatusKind::Err),
    }
}

fn parse_view_kind(s: &str) -> Option<ViewKind> {
    match s.to_ascii_lowercase().as_str() {
        "circuit" | "c" => Some(ViewKind::Circuit),
        "prob" | "probabilities" | "p" => Some(ViewKind::Probability),
        "sv" | "state" | "statevector" => Some(ViewKind::StateVector),
        "bloch" | "b" => Some(ViewKind::Bloch),
        "editor" | "e" => Some(ViewKind::Editor),
        "noise" | "n" => Some(ViewKind::Noise),
        "fidelity" | "fid" | "f" => Some(ViewKind::Fidelity),
        "entanglement" | "ent" => Some(ViewKind::Entanglement),
        "density" | "dm" | "rho" => Some(ViewKind::DensityMatrix),
        _ => None,
    }
}
