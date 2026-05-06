//! Residual-Predictive Dithered Quantization (RPDQ).
//!
//! Prevents cascaded quantization fidelity collapse across repeated
//! reset/recompression cycles by:
//!
//! 1. Tracking the residual from each quantization step
//! 2. Applying Haar-dithered phase randomization before re-quantization
//! 3. Using seed-stabilized stochastic rounding for unbiased quantization
//!
//! Expected behavior: additive infidelity growth in expectation over resets,
//! rather than multiplicative fidelity decay.

use spinoza::core::State;
use spinoza::gates::{apply, c_apply, Gate};
use spinoza::math::Float;

use crate::bacqs::{classify_gate, CliffordOp, GateType};
use crate::compression::{
    apply_givens_rotation, apply_givens_rotation_inverse, deinterleave_state,
    dequantize_adaptive, get_codebook, interleave_state, l2_norm, pack_indices,
    renormalize_state, CompressedState, BLOCK_SIZE,
};
use crate::tableau::CliffordTableau;

// ── Deterministic hashing for seed-stabilized randomness ──────────────

/// Deterministic u64 hash from (seed, reset_index, j).
///
/// Uses a splitmix64-style cascade so that different seeds or indices
/// produce decorrelated outputs. The same (seed, reset_index, j) always
/// gives the same result — all stochasticity is seed-derived.
fn hash_u64(seed: u64, reset_index: u64, j: u64) -> u64 {
    let mut x = seed;
    x ^= reset_index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = x.wrapping_add(j);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

/// Deterministic f64 in [0, 1) from seed-derived hash.
fn deterministic_f64(seed: u64, reset_index: u64, j: u64) -> f64 {
    let bits = hash_u64(seed, reset_index, j);
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// Deterministic dither phase angle for amplitude index j.
fn dither_phase(seed: u64, reset_index: u64, j: u64) -> f64 {
    2.0 * std::f64::consts::PI * deterministic_f64(seed, reset_index ^ 0xAAAA_AAAA, j)
}

// ── Stochastic rounding ───────────────────────────────────────────────

/// Stochastic rounding: given a value `x` and a codebook, probabilistically
/// round to one of the two nearest codebook levels such that
/// `E[round_result | x] = x` (unbiasedness).
///
/// `u` is a deterministic random value in [0, 1) derived from the seed.
/// Uses `hash_u64(seed, reset_index, j)` with an offset to generate `u`.
fn stochastic_round(value: f64, codebook: &[f64], seed: u64, reset_index: u64, j: u64) -> u8 {
    let u = deterministic_f64(seed, reset_index, j);
    stochastic_round_with_u(value, codebook, u)
}

/// Core stochastic rounding logic, separated for testability.
fn stochastic_round_with_u(value: f64, codebook: &[f64], u: f64) -> u8 {
    let pos = codebook.partition_point(|&x| x < value);
    if pos == 0 {
        return 0;
    }
    if pos == codebook.len() {
        return (codebook.len() - 1) as u8;
    }
    let a = codebook[pos - 1];
    let b = codebook[pos];
    let p_b = (value - a) / (b - a);
    if u < p_b {
        pos as u8
    } else {
        (pos - 1) as u8
    }
}

// ── Haar-dithered phase randomization ────────────────────────────────

/// Apply Haar-dithered phase randomization in the computational basis.
///
/// D = ⊗ D_i where each D_i ∈ Haar(U(2)) is a random single-qubit unitary.
/// We implement this as a random phase per computational basis state:
/// D|j⟩ = e^{iθ_j}|j⟩ where θ_j is derived deterministically from the seed.
///
/// This is equivalent to a tensor product of diagonal unitaries, a subgroup
/// of Haar(U(2)^⊗n) sufficient for decorrelating quantization error phases.
pub fn apply_dither(state: &mut State, seed: u64, reset_index: u64) {
    for j in 0..state.len() {
        let theta = dither_phase(seed, reset_index, j as u64);
        let (sin_t, cos_t) = theta.sin_cos();
        let re = state.reals[j] as f64;
        let im = state.imags[j] as f64;
        state.reals[j] = (cos_t.mul_add(re, -sin_t * im)) as Float;
        state.imags[j] = (sin_t.mul_add(re, cos_t * im)) as Float;
    }
}

/// Inverse dither: D^{-1}|j⟩ = e^{-iθ_j}|j⟩.
pub fn apply_inverse_dither(state: &mut State, seed: u64, reset_index: u64) {
    for j in 0..state.len() {
        let theta = -dither_phase(seed, reset_index, j as u64);
        let (sin_t, cos_t) = theta.sin_cos();
        let re = state.reals[j] as f64;
        let im = state.imags[j] as f64;
        state.reals[j] = (cos_t.mul_add(re, -sin_t * im)) as Float;
        state.imags[j] = (sin_t.mul_add(re, cos_t * im)) as Float;
    }
}

// ── RPDQ-aware compression with stochastic rounding ──────────────────

/// Compress a state using stochastic rounding instead of deterministic
/// nearest-neighbor quantization.
///
/// The flow matches `compress()` but replaces deterministic quantizer
/// with seed-stabilized stochastic rounding so that
/// E[Q^{-1}(Q(x)) | x] = x component-wise.
fn rpdq_compress(state: &State, bits: u8, rotation_seed: u64, dither_seed: u64, reset_index: u64) -> (CompressedState, Vec<f64>, Vec<f64>) {
    let n_qubits = usize::from(state.n);
    let flattened = interleave_state(state);
    let norm = l2_norm(&flattened);

    let mut rotated = flattened.clone();
    apply_givens_rotation(&mut rotated, rotation_seed, n_qubits);

    let (indices, block_scales) = stochastic_quantize_adaptive(&rotated, bits, dither_seed, reset_index);
    let packed_indices = pack_indices(&indices, bits);

    // Reconstruct to compute residual (in rotated basis)
    let reconstruction = dequantize_adaptive(&packed_indices, &block_scales, bits, rotated.len());

    // Residual in rotated basis
    let residual_rotated: Vec<f64> = rotated
        .iter()
        .zip(reconstruction.iter())
        .map(|(&orig, &recon)| orig - recon)
        .collect();

    // Transform residual back to computational basis
    let mut residual_real_embedding = residual_rotated;
    apply_givens_rotation_inverse(&mut residual_real_embedding, rotation_seed, n_qubits);
    let residual_state = deinterleave_state(&residual_real_embedding, n_qubits);
    let residual_re: Vec<f64> = residual_state.reals.iter().map(|&v| v as f64).collect();
    let residual_im: Vec<f64> = residual_state.imags.iter().map(|&v| v as f64).collect();

    let compressed = CompressedState {
        packed_indices,
        block_scales,
        norm,
        n_qubits,
        bits,
        rotation_seed,
        #[cfg(feature = "qjl")]
        residual_sketch: None,
    };

    (compressed, residual_re, residual_im)
}

/// Stochastic adaptive quantization: same structure as `quantize_adaptive`
/// but uses `stochastic_round` instead of nearest-neighbor.
fn stochastic_quantize_adaptive(
    v: &[f64],
    bits: u8,
    seed: u64,
    reset_index: u64,
) -> (Vec<u8>, Vec<f64>) {
    let codebook = get_codebook(bits);
    let n_blocks = v.len().div_ceil(BLOCK_SIZE);
    let mut block_scales = Vec::with_capacity(n_blocks);
    let mut indices = Vec::with_capacity(v.len());

    for (block_idx, block) in v.chunks(BLOCK_SIZE).enumerate() {
        let max_abs = block
            .iter()
            .fold(0.0_f64, |acc, &x| acc.max(x.abs()));
        let scale = if max_abs == 0.0 { 1.0 } else { max_abs };
        block_scales.push(scale);
        let inv_scale = 1.0 / scale;

        for (j, &value) in block.iter().enumerate() {
            let normalized = value * inv_scale;
            let global_j = (block_idx * BLOCK_SIZE + j) as u64;
            // Use a different j-domain for rounding vs dither by adding a large offset
            indices.push(stochastic_round(normalized, codebook, seed, reset_index, global_j + (1 << 40)));
        }
    }

    (indices, block_scales)
}

// ── RPDQ State ───────────────────────────────────────────────────────

/// RPDQ hybrid simulator state.
///
/// Maintains:
/// - A coarse quantized state (c_tilde) as a `CompressedState`
/// - A residual in the computational basis tracking what was lost in quantization
/// - A Clifford tableau for tracking Clifford operations
/// - Seed-stabilized dither/rounding state
///
/// On each reset event, the residual is transformed, dithered, and fed back
/// into the quantization step, preventing coherent error accumulation.
pub struct RpdqState {
    /// The coarse quantized state.
    pub compressed: CompressedState,
    /// Residual real components in the computational basis.
    residual_re: Vec<f64>,
    /// Residual imaginary components in the computational basis.
    residual_im: Vec<f64>,
    /// Accumulated Clifford operations.
    pub tableau: CliffordTableau,
    /// Sequence of Clifford ops for replay during reset.
    clifford_ops: Vec<CliffordOp>,
    /// Count of T-gates applied since last reset.
    pub t_gate_count: usize,
    /// Total gates applied.
    pub total_gates: usize,
    /// How many T-gates before forcing a reset.
    pub t_gate_threshold: usize,
    /// Compression bit depth.
    pub bits: u8,
    /// RNG seed for rotation (same as BACQS rotation seed).
    pub rotation_seed: u64,
    /// RNG seed for dither and stochastic rounding.
    pub dither_seed: u64,
    /// Number of qubits.
    pub n_qubits: usize,
    /// How many times the full statevector was materialised.
    pub decompression_count: usize,
    /// How many reset events occurred.
    pub reset_index: u64,
    /// Per-reset residual L2 norms for instrumentation.
    pub residual_norms: Vec<f64>,
    /// Per-reset estimated fidelity contribution.
    pub fidelity_estimates: Vec<f64>,
}

impl RpdqState {
    /// Initialize from a Spinoza State.
    ///
    /// Applies initial dither, compresses with stochastic rounding,
    /// undoes dither, and computes the initial residual.
    pub fn new(state: &State, bits: u8, rotation_seed: u64, dither_seed: u64, t_gate_threshold: usize) -> Self {
        let n_qubits = usize::from(state.n);

        // Apply dither so compression matches the commit_reset flow
        let mut dithered = state.clone();
        apply_dither(&mut dithered, dither_seed, 0);

        let (compressed, _, _) =
            rpdq_compress(&dithered, bits, rotation_seed, dither_seed, 0);

        // Decompress and undo dither to get the undithered approximation
        let mut approx = compressed.decompress();
        apply_inverse_dither(&mut approx, dither_seed, 0);

        // Residual = exact - undithered_approx (in computational basis)
        let residual_re: Vec<f64> = state
            .reals
            .iter()
            .zip(approx.reals.iter())
            .map(|(&ex, &ap)| ex as f64 - ap as f64)
            .collect();
        let residual_im: Vec<f64> = state
            .imags
            .iter()
            .zip(approx.imags.iter())
            .map(|(&ex, &ap)| ex as f64 - ap as f64)
            .collect();

        let residual_norm: f64 = residual_re
            .iter()
            .zip(residual_im.iter())
            .map(|(&re, &im)| re.mul_add(re, im * im))
            .sum::<f64>()
            .sqrt();

        RpdqState {
            compressed,
            residual_re,
            residual_im,
            tableau: CliffordTableau::new(n_qubits),
            clifford_ops: Vec::new(),
            t_gate_count: 0,
            total_gates: 0,
            t_gate_threshold,
            bits,
            rotation_seed,
            dither_seed,
            n_qubits,
            decompression_count: 0,
            reset_index: 0,
            residual_norms: vec![residual_norm],
            fidelity_estimates: Vec::new(),
        }
    }

    // ── gate application ─────────────────────────────────────────────

    /// Apply a single-qubit gate.
    ///
    /// Clifford gates update only the tableau (O(n^2)).
    /// T-gates queue and count; trigger reset when threshold met.
    /// Arbitrary gates force a full decompress → apply → commit cycle.
    pub fn apply_gate(&mut self, gate: Gate, target: usize) {
        self.total_gates += 1;

        match classify_gate(&gate) {
            GateType::Clifford => {
                self.apply_clifford_to_tableau(&gate, target);
            }
            GateType::TGate => {
                self.t_gate_count += 1;
                if self.t_gate_count >= self.t_gate_threshold {
                    self.rpdq_reset(gate, target);
                } else {
                    self.clifford_ops.push(CliffordOp::T(target));
                }
            }
            GateType::Arbitrary => {
                let mut state = self.to_state();
                apply(gate, &mut state, target);
                self.commit_reset(&state);
                // Reset tableau since we recompressed the full state
                self.tableau = CliffordTableau::new(self.n_qubits);
                self.clifford_ops.clear();
                self.t_gate_count = 0;
            }
        }
    }

    /// Apply a controlled gate.
    ///
    /// CNOT is Clifford — update tableau only.
    /// Other controlled gates: decompress, apply, commit.
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
                        let mut state = self.to_state();
                        c_apply(gate, &mut state, control, target);
                        self.commit_reset(&state);
                        self.tableau = CliffordTableau::new(self.n_qubits);
                        self.clifford_ops.clear();
                        self.t_gate_count = 0;
                    }
                }
            }
            GateType::TGate | GateType::Arbitrary => {
                let mut state = self.to_state();
                c_apply(gate, &mut state, control, target);
                self.commit_reset(&state);
                self.tableau = CliffordTableau::new(self.n_qubits);
                self.clifford_ops.clear();
                self.t_gate_count = 0;
            }
        }
    }

    /// Apply a Clifford gate to the tableau only — O(n^2), statevector untouched.
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
            }
            Gate::SWAP(a, b) => {
                let a = *a;
                let b = *b;
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

    // ── RPDQ reset ───────────────────────────────────────────────────

    /// RPDQ reset event.
    ///
    /// 1. Decompress c_tilde_old and add residual → reconstructed state
    /// 2. Apply accumulated Clifford ops and pending T-gate → new state
    /// 3. Dither, compress with stochastic rounding, track new residual
    ///
    /// The residual is fed back into each reset so that quantization
    /// error does not compound coherently. Dither decorrelates the
    /// error phase; stochastic rounding keeps it unbiased.
    pub fn rpdq_reset(&mut self, pending_gate: Gate, pending_target: usize) {
        self.decompression_count += 1;
        self.reset_index += 1;

        // 1. Reconstruct: decompress coarse state, undo dither, add residual
        // compressed stores D(exact) — decompress gives D(exact)_approx
        // residual = exact - D^{-1}(D(exact)_approx)  (in computational basis)
        // So: D^{-1}(decompress(compressed)) + residual ≈ exact
        let mut state = self.compressed.decompress();
        apply_inverse_dither(&mut state, self.dither_seed, self.reset_index - 1);
        for ((re, im), (&r_re, &r_im)) in state
            .reals
            .iter_mut()
            .zip(state.imags.iter_mut())
            .zip(self.residual_re.iter().zip(self.residual_im.iter()))
        {
            *re = (*re as f64 + r_re) as Float;
            *im = (*im as f64 + r_im) as Float;
        }
        renormalize_state(&mut state);

        // 2. Apply accumulated Clifford ops
        for op in &self.clifford_ops {
            op.apply_to(&mut state);
        }

        // 3. Apply the pending T-gate
        apply(pending_gate, &mut state, pending_target);

        // 4. Commit: dither, compress with stochastic rounding, track residual
        self.commit_reset(&state);

        // 5. Reset tableau and tracking
        self.tableau = CliffordTableau::new(self.n_qubits);
        self.clifford_ops.clear();
        self.t_gate_count = 0;
    }

    /// Commit a reset: given the exact state in the computational basis,
    /// apply dither, compress with stochastic rounding, undo dither, and
    /// compute the new residual.
    fn commit_reset(&mut self, exact_state: &State) {
        // Apply dither in the computational basis
        let mut dithered = exact_state.clone();
        apply_dither(&mut dithered, self.dither_seed, self.reset_index);

        // Compress the dithered state with stochastic rounding
        let (new_compressed, _, _) =
            rpdq_compress(&dithered, self.bits, self.rotation_seed, self.dither_seed, self.reset_index);

        // Decompress and undo dither to get the undithered approximation
        let mut approx = new_compressed.decompress();
        apply_inverse_dither(&mut approx, self.dither_seed, self.reset_index);

        // New residual = exact_state - approx (in computational basis)
        self.residual_re = exact_state
            .reals
            .iter()
            .zip(approx.reals.iter())
            .map(|(&ex, &ap)| ex as f64 - ap as f64)
            .collect();
        self.residual_im = exact_state
            .imags
            .iter()
            .zip(approx.imags.iter())
            .map(|(&ex, &ap)| ex as f64 - ap as f64)
            .collect();

        // Compute residual norm for instrumentation
        let residual_norm: f64 = self
            .residual_re
            .iter()
            .zip(self.residual_im.iter())
            .map(|(&re, &im)| re.mul_add(re, im * im))
            .sum::<f64>()
            .sqrt();
        self.residual_norms.push(residual_norm);

        // Fidelity estimate: 1 - ||r||^2
        let infidelity = self
            .residual_re
            .iter()
            .zip(self.residual_im.iter())
            .map(|(&re, &im)| re.mul_add(re, im * im))
            .sum::<f64>();
        self.fidelity_estimates.push((1.0 - infidelity).max(0.0));

        self.compressed = new_compressed;
    }

    // ── state materialization ────────────────────────────────────────

    /// Decompress to full State.
    ///
    /// Reconstructs the best approximation: decompress(c_tilde) + residual,
    /// then applies accumulated Clifford ops.
    pub fn to_state(&mut self) -> State {
        self.decompression_count += 1;

        let mut state = self.compressed.decompress();

        // Undo dither: compressed stores D(exact); decompress gives D(exact)_approx
        apply_inverse_dither(&mut state, self.dither_seed, self.reset_index);
        // Add back the residual (which is in computational basis)
        for ((re, im), (&r_re, &r_im)) in state
            .reals
            .iter_mut()
            .zip(state.imags.iter_mut())
            .zip(self.residual_re.iter().zip(self.residual_im.iter()))
        {
            *re = (*re as f64 + r_re) as Float;
            *im = (*im as f64 + r_im) as Float;
        }
        renormalize_state(&mut state);

        // Apply accumulated Clifford ops
        for op in &self.clifford_ops {
            op.apply_to(&mut state);
        }

        state
    }

    // ── reporting ────────────────────────────────────────────────────

    /// Print an instrumentation report including RPDQ-specific metrics.
    pub fn print_report(&self) {
        let compressed_bytes = self.compressed.packed_indices.capacity();
        let raw_sv_bytes = (1usize << self.n_qubits) * 2 * std::mem::size_of::<Float>();
        let ratio = raw_sv_bytes as f64 / compressed_bytes.max(1) as f64;

        println!("=== RPDQ INSTRUMENTATION REPORT ===");
        println!("  n_qubits          : {}", self.n_qubits);
        println!("  total_gates       : {}", self.total_gates);
        println!("  reset_count       : {}", self.reset_index);
        println!("  decompressions    : {}", self.decompression_count);
        println!("  bits              : {}", self.bits);
        println!("  rotation_seed     : 0x{:016X}", self.rotation_seed);
        println!("  dither_seed       : 0x{:016X}", self.dither_seed);
        println!("  compressed_bytes  : {}", compressed_bytes);
        println!("  raw_sv_bytes      : {}", raw_sv_bytes);
        println!("  compression_ratio : {:.2}x", ratio);

        if !self.residual_norms.is_empty() {
            println!("  --- residual norms per reset ---");
            for (i, norm) in self.residual_norms.iter().enumerate() {
                println!("    reset {:3}: ||r|| = {:.12}", i, norm);
            }
        }

        if !self.fidelity_estimates.is_empty() {
            println!("  --- fidelity estimates per reset ---");
            for (i, fid) in self.fidelity_estimates.iter().enumerate() {
                println!("    reset {:3}: F ≈ {:.12}", i + 1, fid);
            }
        }

        println!("=====================================");
    }

    /// Snapshot of the compressed bytes.
    pub fn compressed_bytes(&self) -> Vec<u8> {
        self.compressed.packed_indices.clone()
    }
}

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that stochastic rounding is empirically unbiased.
    #[test]
    fn test_stochastic_rounding_unbiasedness() {
        let codebook = get_codebook(4); // 16 levels
        let n_trials = 100_000;

        // Pick a value between two codebook levels
        let a = codebook[7]; // -0.13
        let b = codebook[8]; // 0.13
        let target = 0.5 * (a + b); // exactly midway

        let mut sum = 0.0f64;
        for j in 0..n_trials {
            let u = deterministic_f64(0xDEAD_BEEF, 0, j);
            let idx = stochastic_round_with_u(target, codebook, u);
            sum += codebook[idx as usize];
        }
        let mean = sum / n_trials as f64;
        // Mean should converge to target (unbiased)
        assert!((mean - target).abs() < 0.01,
            "stochastic rounding biased: mean={:.6}, target={:.6}", mean, target);
    }

    /// Test deterministic reproducibility for fixed seed/index.
    #[test]
    fn test_deterministic_reproducibility() {
        let codebook = get_codebook(4);
        let value = 0.3;

        // Two runs with same parameters should give identical results
        let results1: Vec<u8> = (0..100)
            .map(|j| stochastic_round(value, codebook, 0x1234, 5, j))
            .collect();
        let results2: Vec<u8> = (0..100)
            .map(|j| stochastic_round(value, codebook, 0x1234, 5, j))
            .collect();

        assert_eq!(results1, results2);
    }

    /// Test that different reset indices decorrelate results.
    #[test]
    fn test_seed_change_causes_decorrelation() {
        let codebook = get_codebook(4);
        let value = 0.3;
        let n = 1000;

        let results_a: Vec<u8> = (0..n)
            .map(|j| stochastic_round(value, codebook, 0x1234, 0, j))
            .collect();
        let results_b: Vec<u8> = (0..n)
            .map(|j| stochastic_round(value, codebook, 0x1234, 1, j))
            .collect();

        // Different reset indices should produce different results
        // Count how many positions differ
        let diffs = results_a.iter().zip(results_b.iter())
            .filter(|(a, b)| a != b)
            .count();

        // With proper decorrelation, a significant fraction should differ
        // (allow some tolerance for random collisions)
        assert!(diffs > n as usize / 4,
            "only {}/{} positions differed between reset indices", diffs, n);
    }

    /// Test dither + inverse dither round-trip consistency.
    #[test]
    fn test_dither_roundtrip() {
        let n_qubits = 4;
        let mut state = State::new(n_qubits);
        // Set some non-trivial amplitudes
        for j in 0..state.len() {
            let angle = j as f64 * 0.7;
            state.reals[j] = angle.cos() as Float;
            state.imags[j] = angle.sin() as Float;
        }

        let original = state.clone();

        apply_dither(&mut state, 0xBEEF, 42);
        apply_inverse_dither(&mut state, 0xBEEF, 42);

        // Should recover original state
        for j in 0..state.len() {
            assert!((state.reals[j] as f64 - original.reals[j] as f64).abs() < 1e-12,
                "dither round-trip failed at index {}", j);
            assert!((state.imags[j] as f64 - original.imags[j] as f64).abs() < 1e-12,
                "dither round-trip failed at index {}", j);
        }
    }

    /// Test that different seeds produce different dither results.
    #[test]
    fn test_different_seeds_decorrelate() {
        let n_qubits = 3;
        let mut state_a = State::new(n_qubits);
        let mut state_b = State::new(n_qubits);

        // Set both to the same initial state
        for j in 0..state_a.len() {
            state_a.reals[j] = 0.5;
            state_a.imags[j] = 0.5;
            state_b.reals[j] = 0.5;
            state_b.imags[j] = 0.5;
        }

        apply_dither(&mut state_a, 0xAAAA, 0);
        apply_dither(&mut state_b, 0xBBBB, 0);

        // Different seeds should produce different results
        let mut max_diff = 0.0f64;
        for j in 0..state_a.len() {
            let dr = state_a.reals[j] as f64 - state_b.reals[j] as f64;
            let di = state_a.imags[j] as f64 - state_b.imags[j] as f64;
            max_diff = max_diff.max(dr.abs()).max(di.abs());
        }

        assert!(max_diff > 1e-6, "different seeds produced identical dither results");
    }

    /// Integration test: basic RPDQ state creation and compression cycle.
    #[test]
    fn test_rpdq_state_creation() {
        let n_qubits = 4;
        let state = State::new(n_qubits);
        let rpdq = RpdqState::new(&state, 4, 0x5EED, 0xD1B5, 1);

        assert_eq!(rpdq.n_qubits, n_qubits);
        assert_eq!(rpdq.bits, 4);
        assert_eq!(rpdq.reset_index, 0);
        assert!(!rpdq.residual_norms.is_empty());
    }

    /// Integration test: apply Clifford gates without reset.
    #[test]
    fn test_rpdq_clifford_gates() {
        let n_qubits = 4;
        let state = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&state, 4, 0x5EED, 0xD1B5, 10);

        // Apply several Clifford gates — should NOT trigger reset
        rpdq.apply_gate(Gate::H, 0);
        rpdq.apply_gate(Gate::X, 1);
        rpdq.apply_gate(Gate::Z, 2);

        assert_eq!(rpdq.reset_index, 0, "Clifford gates should not trigger reset");
    }

    /// Integration test: T-gate triggers reset.
    #[test]
    fn test_rpdq_t_gate_triggers_reset() {
        let n_qubits = 4;
        let state = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&state, 4, 0x5EED, 0xD1B5, 1);

        // Apply H first (Clifford, no reset)
        rpdq.apply_gate(Gate::H, 0);
        assert_eq!(rpdq.reset_index, 0);

        // T-gate with threshold=1 should trigger reset
        rpdq.apply_gate(Gate::P(std::f64::consts::PI / 4.0), 0);
        assert_eq!(rpdq.reset_index, 1, "T-gate should trigger reset with threshold=1");
    }

    /// Integration test: verify output format compatibility.
    #[test]
    fn test_rpdq_output_format_compatible() {
        let n_qubits = 3;
        let state = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&state, 4, 0x5EED, 0xD1B5, 1);

        let final_state = rpdq.to_state();

        // State should have the correct format
        assert_eq!(final_state.len(), 1 << n_qubits);
        assert_eq!(usize::from(final_state.n), n_qubits);

        // Verify normalization
        let norm: f64 = final_state.reals.iter()
            .zip(final_state.imags.iter())
            .map(|(&re, &im)| {
                let r = re as f64;
                let i = im as f64;
                r.mul_add(r, i * i)
            })
            .sum();
        assert!((norm - 1.0).abs() < 1e-6, "state not normalized: norm={}", norm);
    }

    /// Test that residual norms are tracked correctly.
    #[test]
    fn test_rpdq_residual_norm_tracking() {
        let n_qubits = 4;
        let state = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&state, 4, 0x5EED, 0xD1B5, 1);

        // Apply gates that will trigger resets
        rpdq.apply_gate(Gate::H, 0);
        rpdq.apply_gate(Gate::P(std::f64::consts::PI / 4.0), 0); // T → reset #1
        rpdq.apply_gate(Gate::H, 1);
        rpdq.apply_gate(Gate::P(std::f64::consts::PI / 4.0), 1); // T → reset #2

        // Should have initial + 2 reset norms
        assert_eq!(rpdq.residual_norms.len(), 3);
        assert_eq!(rpdq.fidelity_estimates.len(), 2);
    }

    /// Multi-reset fidelity comparison: RPDQ vs BACQS.
    ///
    /// Applies a sequence of T-gates (each triggering a reset) and compares
    /// the fidelity of the compressed state to an exact reference simulation.
    /// RPDQ should show slower cumulative fidelity degradation than BACQS.
    #[test]
    fn test_multi_reset_fidelity_comparison() {
        use crate::bacqs::BACQSState;
        use crate::compression::fidelity;

        let n_qubits = 4;
        let bits = 4;
        let rotation_seed = 0xC0DE;
        let dither_seed = 0xBEEF;
        let threshold = 1; // Reset on every T-gate

        // Build the exact reference state with standard Spinoza
        let mut exact = State::new(n_qubits);
        // Create superposition
        for i in 0..n_qubits {
            apply(Gate::H, &mut exact, i);
        }

        // Prepare BACQS and RPDQ states from the same initial superposition
        let init_bacqs = exact.clone();
        let init_rpdq = exact.clone();

        let mut bacqs = BACQSState::new(&init_bacqs, bits, rotation_seed, threshold);
        let mut rpdq = RpdqState::new(&init_rpdq, bits, rotation_seed, dither_seed, threshold);

        // Apply T gates on qubit 0, each triggering a reset
        let n_resets = 5;
        let mut bacqs_fidelities = Vec::with_capacity(n_resets);
        let mut rpdq_fidelities = Vec::with_capacity(n_resets);
        let mut bacqs_running_fidelity: f64 = 1.0;
        let mut rpdq_running_fidelity: f64 = 1.0;

        for i in 0..n_resets {
            // Apply T to exact reference
            apply(Gate::P(std::f64::consts::PI as Float / 4.0), &mut exact, i % n_qubits);

            // Apply T to BACQS (triggers reset)
            bacqs.apply_gate(Gate::P(std::f64::consts::PI as Float / 4.0), i % n_qubits);

            // Apply T to RPDQ (triggers reset)
            rpdq.apply_gate(Gate::P(std::f64::consts::PI as Float / 4.0), i % n_qubits);

            // Measure fidelity
            let bacqs_state = bacqs.to_state();
            let rpdq_state = rpdq.to_state();

            let bacqs_fid = fidelity(&exact, &bacqs_state);
            let rpdq_fid = fidelity(&exact, &rpdq_state);

            bacqs_fidelities.push(bacqs_fid);
            rpdq_fidelities.push(rpdq_fid);
            bacqs_running_fidelity = bacqs_running_fidelity.min(bacqs_fid);
            rpdq_running_fidelity = rpdq_running_fidelity.min(rpdq_fid);
        }

        // Both should maintain reasonable fidelity
        assert!(bacqs_running_fidelity > 0.5,
            "BACQS fidelity too low: {}", bacqs_running_fidelity);
        assert!(rpdq_running_fidelity > 0.5,
            "RPDQ fidelity too low: {}", rpdq_running_fidelity);

        // RPDQ should not be worse than BACQS in the worst case
        // (allow some tolerance since stochastic rounding adds variance)
        assert!(rpdq_running_fidelity >= bacqs_running_fidelity - 0.05,
            "RPDQ fidelity ({:.6}) significantly worse than BACQS ({:.6})",
            rpdq_running_fidelity, bacqs_running_fidelity);

        // Print comparison for manual inspection
        println!("\n=== Multi-Reset Fidelity Comparison ===");
        println!("{:>6} {:>16} {:>16}", "Reset", "BACQS Fidelity", "RPDQ Fidelity");
        for i in 0..n_resets {
            println!("{:>6} {:>16.12} {:>16.12}", i + 1, bacqs_fidelities[i], rpdq_fidelities[i]);
        }
        println!("=======================================\n");
    }

    /// Diagnostic: 8-qubit state with multiple arbitrary gates (no T-gates).
    /// This mirrors the test0.qasm circuit pattern.
    #[test]
    fn test_rpdq_eight_qubit_arbitrary_gates() {
        let n_qubits = 8;
        let bits = 4;
        let rotation_seed = 0xC0DE;
        let dither_seed = 0xBEEF;

        // Create the initial state and run exact reference
        let mut exact = State::new(n_qubits);
        // Apply Clifford gates
        apply(Gate::H, &mut exact, 0);
        apply(Gate::X, &mut exact, 1);
        apply(Gate::Y, &mut exact, 2);
        apply(Gate::Z, &mut exact, 3);
        // Apply arbitrary gates
        apply(Gate::RX(1.0), &mut exact, 4);
        apply(Gate::RY(2.0), &mut exact, 5);
        apply(Gate::RZ(3.0), &mut exact, 6);
        apply(Gate::U(1.0, 2.0, 3.0), &mut exact, 7);

        // Run same circuit with RPDQ
        let init = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&init, bits, rotation_seed, dither_seed, 1);

        // Clifford gates (tableau only)
        rpdq.apply_gate(Gate::H, 0);
        rpdq.apply_gate(Gate::X, 1);
        rpdq.apply_gate(Gate::Y, 2);
        rpdq.apply_gate(Gate::Z, 3);

        // Arbitrary gates (trigger decompress+apply+commit)
        rpdq.apply_gate(Gate::RX(1.0), 4);
        rpdq.apply_gate(Gate::RY(2.0), 5);
        rpdq.apply_gate(Gate::RZ(3.0), 6);
        rpdq.apply_gate(Gate::U(1.0, 2.0, 3.0), 7);

        // Get final state
        let final_state = rpdq.to_state();

        // Verify normalization
        let norm: f64 = final_state.reals.iter()
            .zip(final_state.imags.iter())
            .map(|(&re, &im)| {
                let r = re as f64;
                let i = im as f64;
                r.mul_add(r, i * i)
            })
            .sum();
        assert!((norm - 1.0).abs() < 1e-6,
            "State not normalized: norm={}", norm);

        // Check fidelity with exact
        use crate::compression::fidelity;
        let fid = fidelity(&exact, &final_state);
        println!("RPDQ 8-qubit arbitrary gates fidelity: {:.12}", fid);
        assert!(fid > 0.01, "Fidelity too low: {}", fid);
    }

    /// Diagnostic: run the exact CLI path for test0.qasm.
    #[test]
    fn test_rpdq_cli_path_mirrors_test0() {
        use spinoza::circuit::Controls;
        use spinoza::openqasm;

        let bits = 4;
        let rotation_seed = 0x5EED_CAFE_D15C_A11E;
        let dither_seed = 0xD1B5_4A32_D192_ED03;

        // Load and parse the QASM string (same as CLI does from file)
        let mut circuit = openqasm::loads(
            "OPENQASM 2.0;\nqreg q[8];\nh q[0];\nx q[1];\ny q[2];\nz q[3];\nrx(1.0) q[4];\nry(2.0) q[5];\nrz(3.0) q[6];\nu(1.0,2.0,3.0) q[7];\n"
        );

        let n_qubits: usize = circuit.quantum_registers_info.iter().sum();
        let init = State::new(n_qubits);
        let mut rpdq = RpdqState::new(&init, bits, rotation_seed, dither_seed, 1);

        let transformations: Vec<_> = circuit.transformations.drain(..).collect();
        for tr in &transformations {
            match &tr.controls {
                Controls::None => {
                    rpdq.apply_gate(tr.gate.clone(), tr.target);
                }
                Controls::Single(control) => {
                    rpdq.apply_controlled_gate(tr.gate.clone(), *control, tr.target);
                }
                _ => {
                    let mut state = rpdq.to_state();
                    // Inline the dispatch_gate logic
                    match &tr.controls {
                        Controls::None => apply(tr.gate.clone(), &mut state, tr.target),
                        Controls::Single(c) => c_apply(tr.gate.clone(), &mut state, *c, tr.target),
                        _ => {}
                    }
                    rpdq = RpdqState::new(&state, bits, rotation_seed, dither_seed, 1);
                }
            }
        }

        let final_state = rpdq.to_state();
        let norm: f64 = final_state.reals.iter()
            .zip(final_state.imags.iter())
            .map(|(&re, &im)| {
                let r = re as f64;
                let i = im as f64;
                r.mul_add(r, i * i)
            })
            .sum();
        println!("CLI path test: n_qubits={}, norm={}", n_qubits, norm);
        assert!((norm - 1.0).abs() < 1e-6,
            "State not normalized via CLI path: norm={}", norm);
    }
}
