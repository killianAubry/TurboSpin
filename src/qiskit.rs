// Python/Qiskit backend.
//
// Single Rust entry point — `run_circuit_source(kind, source)` — that
// dispatches to the bundled Python runner with the right mode (`qasm` for
// OpenQASM source, `py` for Python source) and parses the JSON it returns
// into `SimulationState`.

use std::process::Command;

use serde::Deserialize;

use crate::state::simulation::{Complex, SimulationState};
use crate::state::SimulatorKind;

const QISKIT_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/qiskit_run.py"
));

/// Best-effort `qreg name[N];` parse — used to size the empty Circuit grid
/// while no run has been performed yet.
pub fn scan_qasm_qubits(src: &str) -> usize {
    let mut total = 0usize;
    for line in src.lines() {
        let line = line.split("//").next().unwrap_or(line).trim();
        let line = line.split(';').next().unwrap_or(line).trim();
        let Some(rest) = line.strip_prefix("qreg").map(str::trim) else {
            continue;
        };
        let Some(open) = rest.find('[') else { continue };
        let Some(close) = rest[open + 1..].find(']') else {
            continue;
        };
        let inner = &rest[open + 1..open + 1 + close];
        if let Ok(n) = inner.parse::<usize>() {
            total += n;
        }
    }
    total.clamp(1, 20)
}

/// Best-effort scan for `QuantumCircuit(N)` in Python source — used the
/// same way as `scan_qasm_qubits`.
pub fn scan_python_qubits(src: &str) -> usize {
    const NEEDLE: &[u8] = b"QuantumCircuit(";
    let bytes = src.as_bytes();
    let mut best = 1usize;
    let mut i = 0;
    while i + NEEDLE.len() <= bytes.len() {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            i += NEEDLE.len();
            // skip whitespace
            while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            // strip optional QuantumRegister(...) — we just want the leading int when present
            let start = i;
            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                i += 1;
            }
            if start != i {
                if let Ok(n) = std::str::from_utf8(&bytes[start..i])
                    .unwrap_or("0")
                    .parse::<usize>()
                {
                    best = best.max(n);
                }
            }
        } else {
            i += 1;
        }
    }
    best.clamp(1, 20)
}

/// Pick a qubit-count heuristic for the editor's current source kind.
#[allow(dead_code)]
pub fn detect_qubits(kind: SimulatorKind, src: &str) -> usize {
    match kind {
        SimulatorKind::Qiskit | SimulatorKind::TurboSpin => scan_qasm_qubits(src),
    }
}

/// Run the editor source through Python + Qiskit. Returns a fresh
/// `SimulationState` on success; a single-line error string on failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_circuit_source(kind: SimulatorKind, source: &str) -> Result<SimulationState, String> {
    let python = python_executable();
    let mut dir = std::env::temp_dir();
    dir.push("qsim_ui_qiskit");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let py_path = dir.join(format!("runner_{stamp}.py"));
    std::fs::write(&py_path, QISKIT_SCRIPT.as_bytes())
        .map_err(|e| format!("write runner: {e}"))?;

    let src_path = dir.join(format!("circuit_{stamp}.{}", kind.circuit_extension()));
    std::fs::write(&src_path, source.as_bytes()).map_err(|e| format!("write src: {e}"))?;

    let out = Command::new(&python)
        .arg(&py_path)
        .arg(kind.runner_mode())
        .arg(&src_path)
        .output()
        .map_err(|e| format!("{python}: {e}"))?;

    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&py_path);

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(first_line_or_all(err.trim()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_qiskit_json(stdout.trim())
}

#[cfg(target_arch = "wasm32")]
pub fn run_circuit_source(_kind: SimulatorKind, _source: &str) -> Result<SimulationState, String> {
    Err("Qiskit/OpenQASM run on desktop builds only".into())
}

fn first_line_or_all(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

fn python_executable() -> String {
    std::env::var("QISKIT_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

#[derive(Deserialize)]
struct QiskitOut {
    num_qubits: usize,
    statevector: Vec<Amp>,
}

#[derive(Deserialize)]
struct Amp {
    re: f64,
    im: f64,
}

fn parse_qiskit_json(s: &str) -> Result<SimulationState, String> {
    let v: QiskitOut = serde_json::from_str(s).map_err(|e| format!("json: {e}"))?;
    let n = v.num_qubits;
    let expected = 1usize << n.min(20);
    if v.statevector.len() != expected {
        return Err(format!(
            "shape mismatch: {} amplitudes for {} qubits",
            v.statevector.len(),
            n,
        ));
    }
    let sv: Vec<Complex> = v
        .statevector
        .iter()
        .map(|a| Complex {
            re: a.re as f32,
            im: a.im as f32,
        })
        .collect();
    Ok(SimulationState::from_statevector(n, sv))
}
