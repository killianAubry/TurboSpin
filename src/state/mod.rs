// Composition root for application state.
//
//     editor_text  ──[parse]──▶  circuit          (live, on every edit)
//     editor_text  ──[run]────▶  simulation       (on ⌘R / :run)
//
// Three simulators are wired up:
//
//   * `SimulatorKind::OpenQasm`   buffer is OpenQASM 2 → Qiskit
//   * `SimulatorKind::Qiskit`     buffer is Python      → Qiskit (exec)
//   * `SimulatorKind::TurboSpin`  buffer is OpenQASM 2 → Spinoza CLI (`--comp-bit`)
//
// The visualizer-only `Circuit` is parsed in pure Rust here so the gate grid
// updates as you type. The actual statevector comes from the chosen runner;
// `SimulationState::from_statevector` derives probabilities and Bloch vectors
// from it, so every panel besides the editor and circuit visualizer reads off
// the same `statevector` source of truth.

pub mod circuit;
pub mod noise;
pub mod simulation;
pub mod simulator;
pub mod ui_state;
pub mod metrics;

use std::path::PathBuf;

use crate::dsl::{parse_python, parse_qasm, Diagnostic};
use crate::tiling::{Tile, TileId, ViewKind};

pub use circuit::Circuit;
pub use simulation::SimulationState;
pub use simulator::{SimulatorKind, SourceKind, TurboSpinCompression, TurboSpinMode};
pub use ui_state::{StatusKind, UiState};
pub use noise::NoiseConfig;
pub use metrics::{CompressionInfo, MetricsTracker};

pub struct AppState {
    pub editor_text: String,
    last_synced_text: String,
    /// When this differs from `simulator`, `ensure_synced` re-derives the
    /// derived state (qubit count, gates, diagnostics).
    last_sync_simulator: SimulatorKind,

    pub circuit: Circuit,
    pub circuit_last_clean: Circuit,
    pub diagnostics: Vec<Diagnostic>,
    pub simulator: SimulatorKind,
    pub turbospin_compression: TurboSpinCompression,
    pub turbospin_mode: TurboSpinMode,
    pub simulation: SimulationState,

    /// Compare mode: when set, `rerun()` also executes this simulator and
    /// stores its result in `compare_simulation`. Panels overlay the two
    /// results in different colours.
    pub compare_simulator: Option<SimulatorKind>,
    pub compare_simulation: Option<SimulationState>,
    pub compare_compression: TurboSpinCompression,
    pub compare_turbospin_mode: TurboSpinMode,

    pub workspace_dir: PathBuf,
    pub ui: UiState,
    pub tiles: Tile,
    pub focused_tile: TileId,
    pub metrics_tracker: MetricsTracker,
    pub noise_config: NoiseConfig,
}

impl AppState {
    pub fn new() -> Self {
        let simulator = SimulatorKind::Qiskit;
        let editor_text = String::new();

        let (circuit, diagnostics) = parse_for(simulator, &editor_text);
        let circuit_last_clean = circuit.clone();
        let simulation = SimulationState::ground_state(circuit.num_qubits);

        let tiles = Tile::leaf(ViewKind::Editor);
        let focused_tile = tiles.first_leaf().id;

        let workspace_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Self {
            last_synced_text: editor_text.clone(),
            last_sync_simulator: simulator,
            editor_text,
            circuit,
            circuit_last_clean,
            diagnostics,
            simulation,
            simulator,
            turbospin_compression: TurboSpinCompression::default(),
            turbospin_mode: TurboSpinMode::default(),
            compare_simulator: None,
            compare_simulation: None,
            compare_compression: TurboSpinCompression::default(),
            compare_turbospin_mode: TurboSpinMode::default(),
            workspace_dir,
            ui: UiState::default(),
            tiles,
            focused_tile,
            metrics_tracker: MetricsTracker::new(),
            noise_config: NoiseConfig::default(),
        }
    }

    #[inline]
    pub fn circuit_file_path(&self) -> PathBuf {
        self.workspace_dir
            .join(format!("circuit.{}", self.simulator.circuit_extension()))
    }

    pub fn save_circuit_file(&mut self) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = self;
            return Err("save: use a desktop build".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self.circuit_file_path();
            std::fs::write(&path, self.editor_text.as_bytes())
                .map_err(|e| format!("save {}: {e}", path.display()))?;
            Ok(())
        }
    }

    pub fn load_circuit_file(&mut self) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = self;
            return Err("load: use a desktop build".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self.circuit_file_path();
            let s = std::fs::read_to_string(&path)
                .map_err(|e| format!("load {}: {e}", path.display()))?;
            self.load_editor_text_from_path(path, s)
        }
    }

    /// Replace the editor with file contents, set `workspace_dir` to the parent, sync, and rerun.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_editor_text_from_path(&mut self, path: PathBuf, text: String) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            self.workspace_dir = parent.to_path_buf();
        }
        self.editor_text = text;
        self.ensure_synced();
        self.rerun()
    }

    /// Cheap, runs every frame. Re-parses the buffer into `circuit` whenever
    /// the editor text or simulator selection has changed, so the visualizer
    /// stays in lockstep with the editor. Does **not** touch `simulation` —
    /// that is reserved for `rerun`.
    pub fn ensure_synced(&mut self) {
        let simulator_changed = self.simulator != self.last_sync_simulator;
        let text_changed = self.editor_text != self.last_synced_text;
        if !simulator_changed && !text_changed {
            return;
        }
        self.last_sync_simulator = self.simulator;
        self.last_synced_text = self.editor_text.clone();

        let (parsed, diagnostics) = parse_for(self.simulator, &self.editor_text);
        apply_circuit_parse(self.simulator, parsed, diagnostics, &mut self.circuit, &mut self.circuit_last_clean, &mut self.diagnostics);
    }

    /// Run the editor source through the selected simulator. On success
    /// replaces `self.simulation` so every statevector-derived panel
    /// (probability, state vector, Bloch) refreshes on the next frame.
    ///
    /// When `compare_simulator` is set, also runs the compare simulator and
    /// stores its result in `compare_simulation`.
    pub fn rerun(&mut self) -> Result<(), String> {
        self.metrics_tracker.start_run();

        self.last_sync_simulator = self.simulator;
        self.last_synced_text = self.editor_text.clone();

        let (parsed, diagnostics) = parse_for(self.simulator, &self.editor_text);
        apply_circuit_parse(
            self.simulator,
            parsed,
            diagnostics,
            &mut self.circuit,
            &mut self.circuit_last_clean,
            &mut self.diagnostics,
        );

        let (mut sim, compression) = run_simulator(
            self.simulator,
            &self.editor_text,
            self.turbospin_compression,
            self.turbospin_mode,
        )?;

        // Apply noise models to the statevector before deriving panels.
        crate::state::noise::apply_noise(
            &mut sim.statevector,
            sim.num_qubits,
            &self.noise_config,
            &self.circuit,
        );
        // Re-derive probabilities & Bloch from the (now noisy) statevector.
        sim = crate::state::simulation::SimulationState::from_statevector(
            sim.num_qubits,
            sim.statevector,
        );

        let actual_sv_bytes = sim.statevector.capacity() * std::mem::size_of::<crate::state::simulation::Complex>()
            + sim.probabilities.capacity() * std::mem::size_of::<f32>();

        let metrics = self.metrics_tracker.finalize_run(sim.num_qubits, self.circuit.num_steps, compression, actual_sv_bytes);
        sim.metrics = metrics;
        self.simulation = sim;

        // --- Compare simulator run ---
        if let Some(cmp_kind) = self.compare_simulator {
            let cmp_mode = if cmp_kind == SimulatorKind::TurboSpin {
                self.compare_turbospin_mode
            } else {
                self.turbospin_mode
            };
            match run_simulator(cmp_kind, &self.editor_text, self.compare_compression, cmp_mode) {
                Ok((mut cmp_sim, _cmp_compression)) => {
                    crate::state::noise::apply_noise(
                        &mut cmp_sim.statevector,
                        cmp_sim.num_qubits,
                        &self.noise_config,
                        &self.circuit,
                    );
                    cmp_sim = SimulationState::from_statevector(
                        cmp_sim.num_qubits,
                        cmp_sim.statevector,
                    );
                    self.compare_simulation = Some(cmp_sim);
                }
                Err(e) => {
                    self.compare_simulation = None;
                    self.ui.flash(format!("compare run failed: {e}"), StatusKind::Err);
                }
            }
        }

        Ok(())
    }

    /// Replace the editor with the current simulator's starter template.
    pub fn load_default_template(&mut self) {
        self.editor_text = self.simulator.default_template().to_string();
        self.ensure_synced();
    }
}

/// Apply parse output to `circuit` / `circuit_last_clean`.
///
/// OpenQASM (openqasm + turbospin): the grid advances only when there are no
/// diagnostics — otherwise we keep showing the last error-free layout.
/// Python: always follow the live parse (best-effort).
fn apply_circuit_parse(
    simulator: SimulatorKind,
    parsed: Circuit,
    diagnostics: Vec<Diagnostic>,
    circuit: &mut Circuit,
    last_clean: &mut Circuit,
    diags_out: &mut Vec<Diagnostic>,
) {
    *diags_out = diagnostics;
    if simulator.source_kind() == SourceKind::OpenQasm {
        if diags_out.is_empty() {
            *last_clean = parsed.clone();
            *circuit = parsed;
        } else {
            *circuit = last_clean.clone();
        }
    } else {
        *last_clean = parsed.clone();
        *circuit = parsed;
    }
}

/// Pure-Rust source → `Circuit` for the visualizer. Picks the parser based
/// on the simulator's source family.
fn parse_for(kind: SimulatorKind, src: &str) -> (Circuit, Vec<Diagnostic>) {
    match kind.source_kind() {
        SourceKind::OpenQasm => parse_qasm(src),
        SourceKind::Python => parse_python(src),
    }
}

fn run_simulator(
    kind: SimulatorKind,
    src: &str,
    turbospin_compression: TurboSpinCompression,
    turbospin_mode: TurboSpinMode,
) -> Result<(SimulationState, Option<CompressionInfo>), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (kind, src, turbospin_compression, turbospin_mode);
        return Err("run: desktop builds only".into());
    }

    #[cfg(not(target_arch = "wasm32"))]
    match kind {
        SimulatorKind::Qiskit => {
            crate::qiskit::run_circuit_source(kind, src).map(|sim| (sim, None))
        }
        SimulatorKind::TurboSpin => {
            let result = crate::turbospin::run_qasm_source(src, turbospin_compression, turbospin_mode)?;
            Ok((result.simulation, result.compression))
        }
    }
}
