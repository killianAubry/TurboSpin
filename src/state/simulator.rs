// Simulator (= editor source language + matching runner).
//
// Each variant pairs:
//   * a source language for the editor (OpenQASM 2 or Python/Qiskit),
//   * a workspace file extension,
//   * a runner — either the bundled Python script (Qiskit) or a separate
//     native binary (TurboSpin → Spinoza).
//
// Adding a new simulator is a single match arm here plus a `run_*` function.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulatorKind {
    /// OpenQASM 2 source — runs through Qiskit's `from_qasm_str`.
    Qiskit,
    /// OpenQASM 2 source — runs through the Spinoza-based TurboSpin native
    /// binary (`cargo run -p spinoza --bin spinoza -- --qasm … --comp-bit …`).
    TurboSpin,
}

impl SimulatorKind {
    pub const ALL: &'static [SimulatorKind] = &[
        SimulatorKind::Qiskit,
        SimulatorKind::TurboSpin,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SimulatorKind::Qiskit => "qiskit",
            SimulatorKind::TurboSpin => "turbospin",
        }
    }

    /// File extension for `circuit.<ext>` in the workspace folder.
    pub fn circuit_extension(self) -> &'static str {
        match self {
            SimulatorKind::Qiskit => "qasm",
            SimulatorKind::TurboSpin => "qasm",
        }
    }

    /// Mode arg passed to the bundled Python runner.
    pub fn runner_mode(self) -> &'static str {
        match self {
            SimulatorKind::Qiskit => "qasm",
            SimulatorKind::TurboSpin => "qasm",
        }
    }

    /// Which highlighter / structural parser handles the editor buffer.
    pub fn source_kind(self) -> SourceKind {
        match self {
            SimulatorKind::Qiskit | SimulatorKind::TurboSpin => SourceKind::OpenQasm,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "qiskit" | "py" | "python" | "q" => Some(Self::Qiskit),
            "turbospin" | "spin" | "spinoza" | "ts" => Some(Self::TurboSpin),
            _ => None,
        }
    }

    /// Default template the editor is seeded with for this mode.
    pub fn default_template(self) -> &'static str {
        match self {
            SimulatorKind::Qiskit | SimulatorKind::TurboSpin => OPENQASM_TEMPLATE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurboSpinCompression {
    Lossless,
    Bits(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurboSpinMode {
    Bacqs,
    Rpdq,
}

impl Default for TurboSpinMode {
    fn default() -> Self {
        Self::Bacqs
    }
}

impl TurboSpinMode {
    pub const ALL: &'static [TurboSpinMode] = &[TurboSpinMode::Bacqs, TurboSpinMode::Rpdq];

    pub fn label(self) -> &'static str {
        match self {
            TurboSpinMode::Bacqs => "bacqs",
            TurboSpinMode::Rpdq => "rpdq",
        }
    }

    pub fn cli_arg(self) -> &'static str {
        self.label()
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bacqs" | "b" => Some(Self::Bacqs),
            "rpdq" | "residual" | "rp" | "r" => Some(Self::Rpdq),
            _ => None,
        }
    }
}

impl Default for TurboSpinCompression {
    fn default() -> Self {
        Self::Lossless
    }
}

impl TurboSpinCompression {
    pub const ALL: &'static [TurboSpinCompression] = &[
        TurboSpinCompression::Lossless,
        TurboSpinCompression::Bits(8),
        TurboSpinCompression::Bits(7),
        TurboSpinCompression::Bits(6),
        TurboSpinCompression::Bits(5),
        TurboSpinCompression::Bits(4),
        TurboSpinCompression::Bits(3),
        TurboSpinCompression::Bits(2),
        TurboSpinCompression::Bits(1),
    ];

    pub fn bits(self) -> Option<u8> {
        match self {
            TurboSpinCompression::Lossless => None,
            TurboSpinCompression::Bits(bits) => Some(bits),
        }
    }

    pub fn label(self) -> String {
        match self {
            TurboSpinCompression::Lossless => "exact".to_string(),
            TurboSpinCompression::Bits(bits) => format!("{bits}-bit"),
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "exact" | "lossless" | "off" | "none" | "0" => Some(Self::Lossless),
            _ => {
                let digits = s
                    .strip_suffix("-bit")
                    .or_else(|| s.strip_suffix("bit"))
                    .unwrap_or(&s);
                let bits = digits.parse::<u8>().ok()?;
                (1..=8).contains(&bits).then_some(Self::Bits(bits))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TurboSpinCompression, TurboSpinMode};

    #[test]
    fn parses_turbospin_compression_strings() {
        assert_eq!(
            TurboSpinCompression::from_str("exact"),
            Some(TurboSpinCompression::Lossless)
        );
        assert_eq!(
            TurboSpinCompression::from_str("4"),
            Some(TurboSpinCompression::Bits(4))
        );
        assert_eq!(
            TurboSpinCompression::from_str("7-bit"),
            Some(TurboSpinCompression::Bits(7))
        );
        assert_eq!(TurboSpinCompression::from_str("9"), None);
    }

    #[test]
    fn parses_turbospin_mode_strings() {
        assert_eq!(TurboSpinMode::from_str("bacqs"), Some(TurboSpinMode::Bacqs));
        assert_eq!(TurboSpinMode::from_str("rpdq"), Some(TurboSpinMode::Rpdq));
        assert_eq!(TurboSpinMode::from_str("unknown"), None);
    }
}

/// Family of source language the editor is showing — drives highlighter
/// and circuit-visualizer parser selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    OpenQasm,
    Python,
}

const OPENQASM_TEMPLATE: &str = r#"// Bell pair — OpenQASM 2.0
// keys: ⌘R run · ⌘S save · ⌘O load · ⌘E palette · ⌘T new tile

OPENQASM 2.0;
include "qelib1.inc";

qreg q[2];
creg c[2];

h q[0];
cx q[0], q[1];

measure q[0] -> c[0];
measure q[1] -> c[1];
"#;
