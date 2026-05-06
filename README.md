# qsim (desktop UI)

A Rust quantum-circuit playground built on
[`egui`](https://github.com/emilk/egui)/[`eframe`](https://github.com/emilk/egui/tree/master/crates/eframe).

The native **TurboSpin** simulator (Spinoza + BACQS/RPDQ CLI) lives in its own
repository at [github.com/killianAubry/TurboSpin](https://github.com/killianAubry/TurboSpin) and is **vendored here as a git submodule**
under `TurboSpin/`.

**Clone this desktop project with submodules:**

```bash
git clone -b qsim-ui --recurse-submodules https://github.com/killianAubry/TurboSpin.git qsim
cd qsim
```

GitHub default branch `main` on that repository points at the **simulator workspace only**.
This UI and the rest of the Rust app ship on branch **`qsim-ui`**.

---

The main view is a `micro`-style **text editor** for circuit source. Auxiliary
views (circuit diagram, probability distribution, state vector, 3D Bloch
spheres, noise and more) live in a **tiling window manager** — open them with
`⌘T`, move focus with `⌘H/J/K/L` (also **Ctrl+Tab** / **Ctrl+Shift+Tab** to cycle
tiles), close with `⌘W`. **⌘** (Cmd) is substituted with **Ctrl** on non-macOS
platforms. Use **⌘E** for the command palette and **⌘,** for the configuration
popup.

Two simulators are selectable, both exposing the **same statevector contract** —
every panel besides the editor and circuit visualizer reads off
`state.simulation.statevector`; probabilities and derived views are computed in pure Rust.


<img width="1470" height="923" alt="image" src="https://github.com/user-attachments/assets/fdcfbe22-a651-421c-86cb-bba0f4519449" />


| Mode       | Editor language | Files           | Runner |
| ---------- | --------------- | --------------- | ------ |
| `qiskit`   | OpenQASM 2.0    | `circuit.qasm`  | `python3 scripts/qiskit_run.py qasm <file>` (set `QISKIT_PYTHON` to use a venv) |
| `turbospin`| OpenQASM 2.0    | `circuit.qasm`  | Spinoza CLI in submodule: `--qasm <file> --comp-bit <0–8>` and optional `--compression-mode bacqs\|rpdq` |

See submodule [TurboSpin/README.md](https://github.com/killianAubry/TurboSpin/blob/main/README.md) for CLI flags, compression modes, and OpenQASM coverage.

## Keybinds

| Action | macOS |
| ------ | ----- |
| **Command Palette** | `⌘E` |
| **Open Tile Picker** | `⌘T` |
| **Open Specific Tile** | `⌘1` .. `⌘6` |
| **Run Simulation** | `⌘R` |
| **Open File** | `⌘O` |
| **Save File** | `⌘S` |
| **Close Tile** | `⌘W` |
| **Move Focus** | `⌘H/J/K/L` |
| **Cycle Focus** | `⌘Tab` / `⌘⇧Tab` (also **Ctrl+Tab** / **Ctrl+⇧Tab**) |
| **Configuration** | `⌘,` |
| **Split View** | `⌘⇧H/J/K/L` |

## Available Views

* **Editor**: Syntax-highlighted OpenQASM (and related) source.
* **Circuit**: Live gate grid from the parser while you type.
* **Probability**: Basis-state probabilities.
* **State vector**: Complex amplitudes.
* **Bloch 3D**: Per-qubit Bloch visualization.
* **Noise**: Noise model / device tuning with metric-oriented layout.
* **Fidelity**, **Entanglement**, **Density matrix**: Compare and analyze states (including compare-vs mode).
* Command palette `:open …` exposes short names (`fid`, `ent`, `dm`, etc.).

Switch simulator from the configuration popup (**⌘,**), via `:sim qiskit|turbospin`, or hints in the palette. Optionally **compare** against a second simulator in the same overlay; TurboSpin-vs-TurboSpin can use **different bit depth** (`:compare_compress`) and **compression mode** `:compare_tsmode` (`bacqs` / `rpdq`) independently from the primary run.

When `turbospin` is active, compression is **exact** (`--comp-bit 0`) or **1–8 bits** with an explicit **`bacqs`** vs **`rpdq`** mode whenever compression is on (see submodule README). Successful runs flash **runtime** in the bottom status bar.

The **circuit visualizer** parses the editor source on every keystroke
(OpenQASM and Python both, best-effort) so the gate grid follows live
edits. The simulation panels only refresh when you press `⌘R`.

## Run

```bash
cargo run --release

# OpenQASM / Qiskit modes need Python — install once:
python3 -m pip install qiskit
# (override interpreter via $QISKIT_PYTHON if needed)

# Initialise the TurboSpin submodule if needed:
git submodule update --init --recursive TurboSpin

# TurboSpin runs `cargo` against TurboSpin/Cargo.toml (submodule checkout).
# Pinned toolchain (usually nightly): see TurboSpin/rust-toolchain.toml.
#   $TURBOSPIN_MANIFEST  override path to TurboSpin/Cargo.toml
#   $TURBOSPIN_CARGO     override cargo invocation if required
```

> First build downloads `egui`/`eframe`; subsequent runs are incremental.

## Keybinds

| Key                  | Action                                                  |
| -------------------- | ------------------------------------------------------- |
| `⌘E`                 | command palette                                         |
| `⌘T`                 | tile picker (open a new tile next to the focused one)   |
| `⌘W`                 | close focused tile                                      |
| `⌘R`                 | run the editor source through Qiskit                    |
| `⌘S` / `⌘O`          | save / load `circuit.<ext>` to/from the workspace folder|
| `⌘H` `⌘J` `⌘K` `⌘L`  | focus tile to the left / down / up / right             |
| `⌘Tab` / `⌘⇧Tab`     | cycle tile focus forward / backward (layout order)      |
| `⌘1 … ⌘5`            | open circuit / probabilities / SV / bloch / editor tile |
| `Esc`                | dismiss palette / picker                                |

The folder glyph in the top-left opens a native folder picker — that
directory is where `⌘S` / `⌘O` read and write `circuit.<ext>`. Drag a tile
border to resize.

**Syntax highlighting:** OpenQASM rules apply when the simulator is `openqasm`
or `turbospin`, when the workspace save path is `circuit.qasm`, or when the
first substantive line of the buffer looks like OpenQASM (`OPENQASM`, `qreg`,
`include`). Otherwise the editor uses the Python highlighter for `qiskit` mode.

## Editor sources

### OpenQASM 2.0 (`circuit.qasm`)

```qasm
OPENQASM 2.0;
include "qelib1.inc";

qreg q[2];
creg c[2];

h q[0];
cx q[0], q[1];

measure q[0] -> c[0];
measure q[1] -> c[1];
```

### Qiskit (`circuit.py`)

```python
from qiskit import QuantumCircuit

qc = QuantumCircuit(2, 2)
qc.h(0)
qc.cx(0, 1)
qc.measure([0, 1], [0, 1])
```

In Qiskit mode the runner exec's the file in a fresh namespace and looks for
a `QuantumCircuit` named `qc` (preferred) or `circuit`. It falls back to the
last `QuantumCircuit` defined in the module if neither name is present.

Final `measure` / `barrier` instructions are stripped before evolving the
state vector so the probability / amplitude / Bloch panels reflect the
*pre-measurement* state.

### Syntax highlighting

| token         | color   |
| ------------- | ------- |
| keyword       | yellow  |
| gate / class  | purple  |
| number        | red     |
| string        | green   |
| comment       | dim     |
| punctuation   | muted   |

## Command palette (`⌘E`)

```
:sim <name>          qiskit | turbospin
:compare <name|off>  second simulator overlay (or off)
:open <view>         circuit | prob | sv | bloch | editor | noise | fid | ent | dm | …
:save / :load        workspace circuit.qasm
:close               close focused tile
:run                 run (also ⌘R)
:compress … / :compare_compress …   turbospin bit depth (primary / compare)
:tsmode … / :compare_tsmode …       bacqs | rpdq
:config              configuration popup (also ⌘,)
:reset / :clear / :help
```

## Layout & architecture

```
src/
├── main.rs                       entry point
├── app.rs                        eframe::App: keybinds + tile rendering
├── theme.rs                      design tokens + style installer
├── grid.rs                       reusable cell-grid + snap utility
├── tiling.rs                     tile tree (Leaf / Split) + drag/focus ops
├── workspace.rs                  native folder picker (rfd) + wasm stub
├── qiskit.rs                     Python runner dispatch + JSON parsing
├── turbospin.rs                  Spinoza CLI dispatch + statevector parsing
├── dsl/
│   ├── mod.rs
│   ├── lex.rs                    Token / TokenKind shared by all lexers
│   ├── qasm_lex.rs               OpenQASM 2 highlighter
│   ├── qasm_parse.rs             OpenQASM → Circuit (visualizer)
│   ├── python_lex.rs             Python highlighter
│   └── python_parse.rs           Python → Circuit (visualizer)
├── state/
│   ├── mod.rs                    AppState (composition root)
│   ├── circuit.rs                gate / qubit / step model (visualizer-only)
│   ├── simulation.rs             statevector + derived probs/Bloch
│   ├── simulator.rs              SimulatorKind + per-mode templates
│   └── ui_state.rs               modal flags, status flash
├── scripts/
│   └── qiskit_run.py             OpenQASM | Python → {num_qubits, statevector}
└── components/
    ├── mod.rs                    `render_view` dispatch + module exports
    ├── editor.rs                 syntax-highlighted text editor (main view)
    ├── circuit_visualizer.rs     read-only grid renderer
    ├── probability_panel.rs      bar viz, scrollable
    ├── state_vector.rs           amplitude list, monospace
    ├── bloch_3d.rs               3D Bloch sphere with mouse-drag rotation
    ├── status_bar.rs             top + bottom status strips
    ├── command_palette.rs        `:`-style overlay (⌘E)
    └── tile_picker.rs            view chooser overlay (⌘T)
```

### Design rules

- **No buttons.** Every action is a keybind or a `:`-command.
- **Editor first.** Source flows one-way: `editor_text → simulator
  → SimulationState`. Visual panels never mutate state.
- **Statevector is the source of truth.** Each runner only has to produce
  `(num_qubits, Vec<Complex>)`; `SimulationState::from_statevector` derives
  probabilities and per-qubit Bloch vectors in pure Rust, so the SV /
  probability / Bloch panels render identically across every backend.
- **Run is explicit.** Editing re-parses the circuit visualizer live, but
  the simulation panels refresh exclusively on `⌘R` (or `:run`) — so a
  half-typed circuit doesn't fire a runner every keystroke.
- **Trait-free dispatch.** Each view is a free `show` function;
  `components::render_view(view, ui, &mut state, &mut leaf)` picks the right
  one based on `ViewKind`. Adding a view is one match arm + one module.
- **Per-tile state on the leaf.** `LeafTile` carries its own `bloch_yaw` /
  `bloch_pitch`, so two Bloch tiles can show different angles.
- **Thin borders only.** Tile boundaries use a 1-pixel stroke (`GRID_LINE`,
  or `ACCENT_YELLOW` when focused). Splits are 1-pixel handles.

### 3D Bloch sphere

Software wireframe — three great circles (equator + two meridians) sampled at
64 points each, rotated by the tile's yaw/pitch, projected orthographically.
Depth-based alpha makes the back half read as occluded. Mouse-drag inside any
sphere updates the tile-wide rotation, so all spheres in a tile rotate
together. The Bloch vector itself is drawn last with the same depth shading.

### Adding a simulator

Every simulator only needs to produce `(num_qubits, Vec<Complex>)` — derived
panels follow automatically. Two contracts already plug into that:

`scripts/qiskit_run.py <mode> <file>` (mode = `qasm` | `py`):

```json
{ "num_qubits": 2, "statevector": [{"re": 0.707, "im": 0}, ...] }
```

`cargo run -p spinoza --bin spinoza -- --qasm <file> --comp-bit <0..8>` (TurboSpin) —
plain text on stdout, one line per basis state:
`<index> | <bin> | re=<f> | im=<f> | magnitude=<f> | probability=<f>`.
Some older Spinoza CLIs also prefix a `qubits:` / `statevector:` block; the UI
parser accepts both.

Adding another backend is one match arm in `state::run_simulator` plus a
`run_<name>_source` function returning `Result<SimulationState, String>`
(use `SimulationState::from_statevector` to build the result).
