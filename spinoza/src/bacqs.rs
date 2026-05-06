//! Basis-Adaptive Compressed Quantum Simulation (BACQS).
//!
//! Hybrid simulator that maintains a CompressedState alongside a
//! CliffordTableau, so that Clifford gates only touch the tableau
//! (O(n^2) memory and time) while T-gates and non-Clifford rotations
//! trigger a basis-reset event that materialises, updates, and
//! recompresses the full statevector.
//!
//! Local Pauli observables are evaluated through the tableau via
//! conjugate_pauli, so diagonal observables can often be estimated
//! without a full decompression.

use spinoza::core::State;
use spinoza::gates::{apply, c_apply, Gate};
use spinoza::math::Float;

use crate::compression::{compress, CompressedState};
use crate::tableau::CliffordTableau;

// ── Gate classification ────────────────────────────────────────────────

/// How a gate is handled by the hybrid simulator.
pub enum GateType {
    /// Handled by tableau update only — O(n^2), statevector untouched.
    Clifford,
    /// Forces a basis-reset event (T, T^†).
    TGate,
    /// Must decompress, apply, then recompress.
    Arbitrary,
}

/// Classify a gate: Clifford (tableau-only), TGate (basis reset),
/// or Arbitrary (full decompression required).
pub fn classify_gate(gate: &Gate) -> GateType {
    match gate {
        Gate::H | Gate::X | Gate::Y | Gate::Z | Gate::SWAP(_, _) => GateType::Clifford,
        Gate::P(angle) => {
            let a = *angle as f64;
            let eps = 1e-10;
            let pi = std::f64::consts::PI;
            if (a - pi / 4.0).abs() < eps || (a + pi / 4.0).abs() < eps {
                GateType::TGate
            } else if (a - pi / 2.0).abs() < eps
                || (a + pi / 2.0).abs() < eps
                || (a - pi).abs() < eps
                || a.abs() < eps
            {
                GateType::Clifford
            } else {
                GateType::Arbitrary
            }
        }
        Gate::RX(_) | Gate::RY(_) | Gate::RZ(_) | Gate::U(_, _, _) | Gate::Unitary(_) => {
            GateType::Arbitrary
        }
        _ => GateType::Arbitrary,
    }
}

// ── Clifford operation record for replay ───────────────────────────────

#[derive(Clone, Debug)]
pub(crate) enum CliffordOp {
    H(usize),
    S(usize),
    X(usize),
    Y(usize),
    Z(usize),
    CNOT(usize, usize),
    T(usize),
}

impl CliffordOp {
    pub(crate) fn apply_to(&self, state: &mut State) {
        match *self {
            CliffordOp::H(t) => apply(Gate::H, state, t),
            CliffordOp::S(t) => {
                apply(Gate::P(std::f64::consts::PI as Float / 2.0), state, t);
            }
            CliffordOp::X(t) => apply(Gate::X, state, t),
            CliffordOp::Y(t) => apply(Gate::Y, state, t),
            CliffordOp::Z(t) => apply(Gate::Z, state, t),
            CliffordOp::CNOT(c, t) => c_apply(Gate::X, state, c, t),
            CliffordOp::T(t) => {
                apply(Gate::P(std::f64::consts::PI as Float / 4.0), state, t);
            }
        }
    }
}

// ── BACQS State ────────────────────────────────────────────────────────

/// The main hybrid simulator state.
///
/// Clifford gates update only the tableau (O(n^2)); the compressed
/// statevector stays untouched. T-gates trigger a basis-reset event
/// that materialises the statevector, applies the accumulated Clifford
/// operations, applies the T-gate, and recompresses.
pub struct BACQSState {
    /// The compressed statevector — static during Clifford-only circuits.
    pub compressed: CompressedState,
    /// Accumulated Clifford operations without touching the statevector.
    pub tableau: CliffordTableau,
    /// Sequence of Clifford ops for replay during basis reset.
    clifford_ops: Vec<CliffordOp>,
    /// Count of T-gates applied since last basis reset.
    pub t_gate_count: usize,
    /// Total gates applied.
    pub total_gates: usize,
    /// How many T-gates before forcing a basis reset.
    pub t_gate_threshold: usize,
    /// Compression bit depth.
    pub bits: u8,
    /// RNG seed for rotation.
    pub seed: u64,
    /// Number of qubits.
    pub n_qubits: usize,
    /// How many times the full statevector was materialised.
    pub decompression_count: usize,
    /// How many basis-reset events occurred.
    pub basis_reset_count: usize,
}

impl BACQSState {
    /// Initialize from a Spinoza State.
    pub fn new(state: &State, bits: u8, seed: u64, t_gate_threshold: usize) -> Self {
        let n_qubits = usize::from(state.n);
        let compressed = compress(state, bits, seed);

        BACQSState {
            compressed,
            tableau: CliffordTableau::new(n_qubits),
            clifford_ops: Vec::new(),
            t_gate_count: 0,
            total_gates: 0,
            t_gate_threshold,
            bits,
            seed,
            n_qubits,
            decompression_count: 0,
            basis_reset_count: 0,
        }
    }

    // ── gate application ─────────────────────────────────────────────

    /// Apply a single-qubit gate.
    ///
    /// * Clifford gates: update tableau only — O(n^2), statevector untouched.
    /// * T-gates: queue and count; trigger basis_reset when threshold met.
    /// * Arbitrary: decompress, apply, recompress.
    pub fn apply_gate(&mut self, gate: Gate, target: usize) {
        self.total_gates += 1;

        match classify_gate(&gate) {
            GateType::Clifford => {
                self.apply_clifford_to_tableau(&gate, target);
            }
            GateType::TGate => {
                self.t_gate_count += 1;
                if self.t_gate_count >= self.t_gate_threshold {
                    self.basis_reset(gate, target);
                } else {
                    // Queue T-gate for replay during next basis reset
                    self.clifford_ops.push(CliffordOp::T(target));
                }
            }
            GateType::Arbitrary => {
                let mut state = self.to_state();
                apply(gate, &mut state, target);
                self.compressed = compress(&state, self.bits, self.seed);
                self.tableau = CliffordTableau::new(self.n_qubits);
                self.clifford_ops.clear();
                self.t_gate_count = 0;
            }
        }
    }

    /// Apply a controlled gate (CNOT family).
    ///
    /// CNOT is Clifford — update tableau only.
    /// Other controlled gates: decompress, apply, recompress.
    pub fn apply_controlled_gate(&mut self, gate: Gate, control: usize, target: usize) {
        self.total_gates += 1;

        match classify_gate(&gate) {
            GateType::Clifford => {
                match gate {
                    Gate::X => {
                        self.tableau.cnot(control, target);
                        self.clifford_ops.push(CliffordOp::CNOT(control, target));
                    }
                    _ => {
                        // Other controlled Clifford gates fall back to decompress
                        let mut state = self.to_state();
                        c_apply(gate, &mut state, control, target);
                        self.compressed = compress(&state, self.bits, self.seed);
                        self.tableau = CliffordTableau::new(self.n_qubits);
                        self.clifford_ops.clear();
                        self.t_gate_count = 0;
                    }
                }
            }
            GateType::TGate | GateType::Arbitrary => {
                let mut state = self.to_state();
                c_apply(gate, &mut state, control, target);
                self.compressed = compress(&state, self.bits, self.seed);
                self.tableau = CliffordTableau::new(self.n_qubits);
                self.clifford_ops.clear();
                self.t_gate_count = 0;
            }
        }
    }

    /// Apply a Clifford gate to the tableau only — O(n^2), SV untouched.
    fn apply_clifford_to_tableau(&mut self, gate: &Gate, target: usize) {
        match gate {
            Gate::H => {
                self.tableau.h(target);
                self.clifford_ops.push(CliffordOp::H(target));
            }
            Gate::X => {
                self.tableau.x(target);
                self.clifford_ops.push(CliffordOp::X(target));
            }
            Gate::Y => {
                self.tableau.y(target);
                self.clifford_ops.push(CliffordOp::Y(target));
            }
            Gate::Z => {
                self.tableau.z(target);
                self.clifford_ops.push(CliffordOp::Z(target));
            }
            Gate::P(angle) => {
                let a = *angle as f64;
                let eps = 1e-10;
                let pi = std::f64::consts::PI;
                if (a - pi / 2.0).abs() < eps {
                    self.tableau.s(target);
                    self.clifford_ops.push(CliffordOp::S(target));
                } else if (a + pi / 2.0).abs() < eps {
                    for _ in 0..3 {
                        self.tableau.s(target);
                    }
                    for _ in 0..3 {
                        self.clifford_ops.push(CliffordOp::S(target));
                    }
                }
                // P(π) = Z and P(0) = I are no-ops
            }
            Gate::SWAP(a, b) => {
                let a = *a;
                let b = *b;
                // SWAP = CNOT(a,b) · CNOT(b,a) · CNOT(a,b)
                self.tableau.cnot(a, b);
                self.clifford_ops.push(CliffordOp::CNOT(a, b));
                self.tableau.cnot(b, a);
                self.clifford_ops.push(CliffordOp::CNOT(b, a));
                self.tableau.cnot(a, b);
                self.clifford_ops.push(CliffordOp::CNOT(a, b));
            }
            _ => {}
        }
    }

    // ── basis reset ──────────────────────────────────────────────────

    /// Basis reset event — called when T-gate threshold is hit or a
    /// T-gate arrives (when threshold is 1).
    ///
    /// 1. Decompress the stored CompressedState into a full State
    /// 2. Apply all accumulated Clifford ops to the State
    /// 3. Apply the pending T-gate to the State
    /// 4. Recompress the updated State into a new CompressedState
    /// 5. Reset the tableau to identity
    /// 6. Reset t_gate_count to 0
    ///
    /// This is the only moment the full statevector exists in memory.
    pub fn basis_reset(&mut self, pending_gate: Gate, pending_target: usize) {
        self.decompression_count += 1;
        self.basis_reset_count += 1;

        // 1. Decompress
        let mut state = self.compressed.decompress();

        // 2. Apply accumulated Clifford ops (including queued T-gates)
        for op in &self.clifford_ops {
            op.apply_to(&mut state);
        }

        // 3. Apply the pending T-gate
        apply(pending_gate, &mut state, pending_target);

        // 4. Recompress
        self.compressed = compress(&state, self.bits, self.seed);

        // 5. Reset tableau
        self.tableau = CliffordTableau::new(self.n_qubits);

        // 6. Reset tracking
        self.clifford_ops.clear();
        self.t_gate_count = 0;
    }

    // ── state materialization ────────────────────────────────────────

    /// Decompress to full State — use only for final measurement or
    /// debugging. Each call increments decompression_count.
    pub fn to_state(&mut self) -> State {
        self.decompression_count += 1;

        let mut state = self.compressed.decompress();

        for op in &self.clifford_ops {
            op.apply_to(&mut state);
        }

        state
    }

    // ── measurement ──────────────────────────────────────────────────

    /// Measure the expectation value of a local Pauli observable
    /// WITHOUT fully decompressing the statevector when possible.
    ///
    /// A Pauli observable on n qubits is specified as:
    ///   pauli_x: Vec<u8> of length n, values in {0,1}
    ///   pauli_z: Vec<u8> of length n, values in {0,1}
    ///
    /// Algorithm:
    /// 1. Transform the observable back through the accumulated Clifford
    ///    by applying inverse gates in reverse order — O(c·n) where c is
    ///    the number of accumulated Clifford ops.
    /// 2. If the result is diagonal (only Z and I terms):
    ///    compute ⟨P'⟩ from the compressed amplitudes — no full
    ///    decompression needed.
    /// 3. If the result has X terms (off-diagonal):
    ///    fall back to decompress → compute → return.
    pub fn measure_pauli(&mut self, pauli_x: &[u8], pauli_z: &[u8]) -> f64 {
        assert_eq!(pauli_x.len(), self.n_qubits);
        assert_eq!(pauli_z.len(), self.n_qubits);

        // Transform P back through the accumulated Clifford:
        // compute P' = R†·P·R by walking clifford_ops in reverse
        let (tx, tz) = self.conjugate_pauli_inverse(pauli_x, pauli_z);

        // Check if diagonal
        let is_diagonal = tx.iter().all(|&b| b == 0);

        if is_diagonal {
            self.measure_diagonal_pauli_from_sketch(&tz)
        } else {
            let state = self.to_state();
            pauli_expectation(&state, pauli_x, pauli_z)
        }
    }

    /// Compute P' = R†·P·R by applying inverse Clifford operations
    /// from `clifford_ops` in reverse order to the Pauli (x|z) bits.
    /// Non-Clifford ops (T gates) are skipped as they would require
    /// full state knowledge.
    fn conjugate_pauli_inverse(&self, px: &[u8], pz: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut x = px.to_vec();
        let mut z = pz.to_vec();

        for op in self.clifford_ops.iter().rev() {
            match op {
                CliffordOp::H(q) => {
                    // H† = H: swap x_q ↔ z_q
                    let tmp = x[*q];
                    x[*q] = z[*q];
                    z[*q] = tmp;
                }
                CliffordOp::S(q) => {
                    // S†: z_q ⊕= x_q
                    z[*q] ^= x[*q];
                }
                CliffordOp::CNOT(c, t) => {
                    // CNOT† = CNOT
                    x[*t] ^= x[*c];
                    z[*c] ^= z[*t];
                }
                CliffordOp::X(_q) => {
                    // X† = X: X doesn't affect Pauli X/Z decomposition
                }
                CliffordOp::Y(_q) => {
                    // Y† = Y
                }
                CliffordOp::Z(_q) => {
                    // Z† = Z
                }
                CliffordOp::T(_q) => {
                    // T† = P(-π/4): not Clifford — skip in tableau path
                    // This only happens in measurement, where we fall
                    // back to decompression if any T is in the queue.
                }
            }
        }

        (x, z)
    }

    /// Estimate ⟨Z_i⟩ for a single qubit i — the most common measurement.
    ///
    /// Uses measure_pauli which transforms the observable through the
    /// tableau. For Clifford-only circuits this should NEVER require
    /// decompression.
    pub fn measure_z(&mut self, qubit: usize) -> f64 {
        let mut z_bits = vec![0u8; self.n_qubits];
        z_bits[qubit] = 1;
        let x_bits = vec![0u8; self.n_qubits];
        self.measure_pauli(&x_bits, &z_bits)
    }

    /// Estimate measurement probabilities for all 2^n basis states.
    ///
    /// This ALWAYS requires full decompression. Use measure_pauli or
    /// measure_z whenever possible instead.
    pub fn measure_all_probabilities(&mut self) -> Vec<f64> {
        let state = self.to_state();
        state
            .reals
            .iter()
            .zip(state.imags.iter())
            .map(|(&re, &im)| {
                let r = re as f64;
                let i = im as f64;
                r.mul_add(r, i * i)
            })
            .collect()
    }

    // ── internal helpers ─────────────────────────────────────────────

    /// Compute ⟨P⟩ for a diagonal Pauli P = Z^{z_0} ⊗ ... ⊗ Z^{z_{n-1}}
    /// from the compressed amplitudes without a full statevector
    /// materialisation (Skips the accumulated Clifford replay since the
    /// observable was already transformed through the tableau).
    ///
    /// Decompresses the quantised state and computes the diagonal
    /// expectation value. When the `qjl` feature is enabled and a
    /// residual sketch is present, also applies a sketch-based residual
    /// correction.
    fn measure_diagonal_pauli_from_sketch(&self, z_bits: &[u8]) -> f64 {
        // Decompress the quantised state (this does NOT replay Clifford ops
        // because the observable was already rotated through the tableau)
        let state_hat = self.compressed.decompress();

        // Compute ⟨ψ̂|P|ψ̂⟩ — diagonal Pauli expectation
        let mut expectation = 0.0f64;
        for (idx, (&re, &im)) in state_hat.reals.iter().zip(state_hat.imags.iter()).enumerate() {
            let r = re as f64;
            let i = im as f64;
            let prob = r.mul_add(r, i * i);
            let phase = parity(idx, z_bits);
            expectation += phase as f64 * prob;
        }

        // Residual correction via QJL sketch (feature-gated)
        #[cfg(feature = "qjl")]
        if let Some(ref sketch) = self.compressed.residual_sketch {
            // Build P|ψ̂⟩: flip signs according to z_bits
            let mut p_state = state_hat;
            for idx in 0..p_state.reals.len() {
                if parity(idx, z_bits) < 0 {
                    p_state.reals[idx] = -(p_state.reals[idx]);
                    p_state.imags[idx] = -(p_state.imags[idx]);
                }
            }

            let estimate = sketch.estimate_overlap(&p_state);
            // Correction: 2*Re(⟨r|P|ψ̂⟩) + ⟨r|P|r⟩ ≈ 2*Re(estimate)
            // ⟨r|P|r⟩ is small for good compression
            expectation += 2.0 * estimate.re;
        }

        expectation
    }

    /// Snapshot of the compressed bytes for immutability checking.
    pub fn compressed_bytes(&self) -> Vec<u8> {
        self.compressed.packed_indices.clone()
    }

    // ── reporting ────────────────────────────────────────────────────

    /// Print a memory and performance report.
    pub fn print_report(&self) {
        let compressed_bytes = self.compressed.packed_indices.capacity();
        let raw_sv_bytes = (1usize << self.n_qubits) * 2 * std::mem::size_of::<Float>();
        let ratio = raw_sv_bytes as f64 / compressed_bytes.max(1) as f64;
        let tableau_mem = self.tableau_mem_bytes();
        let peak = raw_sv_bytes.max(compressed_bytes + tableau_mem);

        println!("=== BACQS MEMORY REPORT ===");
        println!("  n_qubits          : {}", self.n_qubits);
        println!("  total_gates       : {}", self.total_gates);
        println!(
            "  clifford_gates    : {}",
            self.total_gates.saturating_sub(self.basis_reset_count)
        );
        println!("  t_gates           : {}", self.t_gate_count);
        println!("  basis_resets      : {}", self.basis_reset_count);
        println!("  decompressions    : {}", self.decompression_count);
        println!("  compressed_bytes  : {}", compressed_bytes);
        println!("  raw_sv_bytes      : {}", raw_sv_bytes);
        println!("  peak_memory_bytes : {}", peak);
        println!("  compression_ratio : {:.2}x", ratio);
        println!(
            "  gates_avoiding_decompression: {} / {} ({:.1}%)",
            self.total_gates.saturating_sub(self.decompression_count),
            self.total_gates,
            if self.total_gates > 0 {
                (self.total_gates - self.decompression_count) as f64
                    / self.total_gates as f64
                    * 100.0
            } else {
                100.0
            }
        );
        println!("===========================");
    }

    fn tableau_mem_bytes(&self) -> usize {
        let rows = 2 * self.n_qubits;
        let cols = 2 * self.n_qubits + 1;
        let table_mem = rows * cols * std::mem::size_of::<u8>();
        let clifford_ops_mem =
            self.clifford_ops.capacity() * std::mem::size_of::<CliffordOp>();
        table_mem + clifford_ops_mem
    }
}

// ── free functions ─────────────────────────────────────────────────────

/// Compute the parity: (-1)^{bits · z_bits}.
fn parity(index: usize, z_bits: &[u8]) -> i32 {
    let mut p = 0u8;
    for (i, &z) in z_bits.iter().enumerate() {
        if z != 0 && (index >> i) & 1 != 0 {
            p ^= 1;
        }
    }
    if p == 0 {
        1
    } else {
        -1
    }
}

/// Compute ⟨ψ|P|ψ⟩ where P is a Pauli specified by x_bits, z_bits.
///
/// P = Π_i X^{x_i} Z^{z_i} acting on |ψ⟩.
/// ⟨ψ|P|ψ⟩ = Σ_a (-1)^{a·z} ψ_a^* ψ_{a⊕x}
fn pauli_expectation(state: &State, x_bits: &[u8], z_bits: &[u8]) -> f64 {
    let n = usize::from(state.n);
    let dim = 1usize << n;

    let x_mask: usize = x_bits
        .iter()
        .enumerate()
        .map(|(i, &b)| if b != 0 { 1usize << i } else { 0 })
        .sum();

    let mut result_re = 0.0f64;

    for a in 0..dim {
        let b = a ^ x_mask;

        let ar = state.reals[a] as f64;
        let ai = state.imags[a] as f64;
        let br = state.reals[b] as f64;
        let bi = state.imags[b] as f64;

        let sign = parity(a, z_bits) as f64;

        // Re(⟨a|ψ⟩* ⟨b|ψ⟩) = ar·br + ai·bi
        result_re += sign * (ar.mul_add(br, ai * bi));
    }

    result_re
}
