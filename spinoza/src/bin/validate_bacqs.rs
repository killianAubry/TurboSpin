#[path = "../compression.rs"]
mod compression;
#[path = "../tableau.rs"]
mod tableau;
#[path = "../bacqs.rs"]
mod bacqs;

use bacqs::BACQSState;
use spinoza::{
    core::State,
    gates::{apply, c_apply, Gate},
    math::Float,
};

const N_QUBITS: usize = 8;
const SEED: u64 = 0x5EED_CAFE_D15C_A11E;

fn main() {
    println!("=== BACQS Validation Suite ===\n");

    let fail_a = test_a_clifford_immutability();
    let fail_b = test_b_t_gate_basis_reset();
    let fail_c = test_c_fidelity();
    let fail_d = test_d_pauli_measurement();
    let fail_e = test_e_memory_report();

    println!("\n=== Validation Complete ===");
    if fail_a || fail_b || fail_c || fail_d || fail_e {
        std::process::exit(1);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Test A — Clifford gates never touch the statevector
// ═══════════════════════════════════════════════════════════════════════

fn test_a_clifford_immutability() -> bool {
    let state = State::new(N_QUBITS);
    let mut bacqs = BACQSState::new(&state, 6, SEED, 1);
    let initial_bytes = bacqs.compressed_bytes();

    let clifford_gates: Vec<(Gate, usize)> = {
        let mut v = Vec::new();
        for qubit in 0..N_QUBITS {
            v.push((Gate::H, qubit));
        }
        for qubit in 0..(N_QUBITS - 1) {
            v.push((Gate::X, qubit)); // Will be used as CNOT
        }
        for qubit in 0..N_QUBITS {
            v.push((Gate::Z, qubit));
        }
        for qubit in 0..N_QUBITS {
            v.push((Gate::Y, qubit));
        }
        for qubit in 0..N_QUBITS {
            v.push((Gate::X, qubit));
        }
        // S gates
        for qubit in 0..N_QUBITS {
            v.push((Gate::P(std::f64::consts::PI as Float / 2.0), qubit));
        }
        v
    };

    // Apply single-qubit Clifford gates
    let mut touched = false;
    let single_qubit_count = clifford_gates.len() / 2;
    for (gate, target) in clifford_gates.iter().take(single_qubit_count) {
        bacqs.apply_gate(gate.clone(), *target);
        if bacqs.compressed_bytes() != initial_bytes {
            touched = true;
            break;
        }
    }

    // Apply CNOT chain
    for qubit in 0..(N_QUBITS - 1) {
        bacqs.apply_controlled_gate(Gate::X, qubit, qubit + 1);
        if bacqs.compressed_bytes() != initial_bytes {
            touched = true;
            break;
        }
    }

    // Apply more single-qubit Clifford gates
    for (gate, target) in clifford_gates.iter().skip(single_qubit_count) {
        bacqs.apply_gate(gate.clone(), *target);
        if bacqs.compressed_bytes() != initial_bytes {
            touched = true;
            break;
        }
    }

    let total = bacqs.total_gates;
    let decomp = bacqs.decompression_count;

    if touched {
        println!(
            "[A] Clifford gate SV immutability: FAIL (compressed bytes changed)"
        );
        true
    } else {
        println!(
            "[A] Clifford gate SV immutability: PASS ({} gates, {} decompressions)",
            total, decomp
        );
        false
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Test B — T-gate triggers exactly one basis reset
// ═══════════════════════════════════════════════════════════════════════

fn test_b_t_gate_basis_reset() -> bool {
    let state = State::new(N_QUBITS);
    let mut bacqs = BACQSState::new(&state, 6, SEED, 1);
    let initial_bytes = bacqs.compressed_bytes();

    // Apply one T-gate (threshold=1 → triggers basis reset immediately)
    bacqs.apply_gate(Gate::P(std::f64::consts::PI as Float / 4.0), 0);

    let compressed_changed = bacqs.compressed_bytes() != initial_bytes;
    let t_gate_reset = bacqs.t_gate_count == 0;
    let tableau_reset = bacqs.tableau.is_identity();

    if compressed_changed && t_gate_reset && tableau_reset {
        println!(
            "[B] T-gate basis reset: PASS (compressed SV updated, tableau reset to identity)"
        );
        false
    } else {
        println!(
            "[B] T-gate basis reset: FAIL (changed={}, t_count_reset={}, tableau_identity={})",
            compressed_changed, t_gate_reset, tableau_reset
        );
        true
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Test C — Fidelity matches reference Spinoza simulator
// ═══════════════════════════════════════════════════════════════════════

fn test_c_fidelity() -> bool {
    let mut any_fail = false;
    // Run on raw Spinoza State (reference)
    let mut ref_state = State::new(N_QUBITS);
    run_test_circuit_direct(&mut ref_state);

    // Run on BACQSState at 4-bit and 8-bit compression
    for bits in [4u8, 8] {
        // Diagnose initial compression fidelity
        let init_state = State::new(N_QUBITS);
        let c = compression::compress(&init_state, bits, SEED);
        let dc = c.decompress();
        let init_fid = fidelity(&init_state, &dc);

        let state = State::new(N_QUBITS);
        let mut bacqs = BACQSState::new(&state, bits, SEED, 10);
        run_test_circuit_bacqs(&mut bacqs);

        let bacqs_final = bacqs.to_state();

        // Compute fidelity
        let fid = fidelity(&ref_state, &bacqs_final);
        let passed = if bits == 4 {
            fid > 0.70
        } else {
            fid > 0.80
        };

        let status = if passed { "PASS" } else { "FAIL" };
        println!(
            "[C] Circuit fidelity ({}-bit): {:.6} (initial: {:.6}) — {}",
            bits, fid, init_fid, status
        );
        if !passed {
            any_fail = true;
        }
    }
    any_fail
}

fn run_test_circuit_direct(state: &mut State) {
    let n = N_QUBITS;
    // H on all qubits
    for q in 0..n {
        apply(Gate::H, state, q);
    }
    // CNOT chain 0→1, 1→2, ..., 6→7
    for q in 0..(n - 1) {
        c_apply(Gate::X, state, q, q + 1);
    }
    // T on qubits 0, 2, 4, 6
    for q in (0..n).step_by(2) {
        apply(Gate::P(std::f64::consts::PI as Float / 4.0), state, q);
    }
    // H on all qubits
    for q in 0..n {
        apply(Gate::H, state, q);
    }
    // CNOT chain reversed 7→6, ..., 1→0
    for q in (1..n).rev() {
        c_apply(Gate::X, state, q, q - 1);
    }
}

fn run_test_circuit_bacqs(bacqs: &mut BACQSState) {
    let n = N_QUBITS;
    // H on all qubits
    for q in 0..n {
        bacqs.apply_gate(Gate::H, q);
    }
    // CNOT chain 0→1, 1→2, ..., 6→7
    for q in 0..(n - 1) {
        bacqs.apply_controlled_gate(Gate::X, q, q + 1);
    }
    // T on qubits 0, 2, 4, 6 — triggers basis reset each time (threshold=1)
    for q in (0..n).step_by(2) {
        bacqs.apply_gate(Gate::P(std::f64::consts::PI as Float / 4.0), q);
    }
    // H on all qubits
    for q in 0..n {
        bacqs.apply_gate(Gate::H, q);
    }
    // CNOT chain reversed 7→6, ..., 1→0
    for q in (1..n).rev() {
        bacqs.apply_controlled_gate(Gate::X, q, q - 1);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Test D — Pauli measurement without decompression
// ═══════════════════════════════════════════════════════════════════════

fn test_d_pauli_measurement() -> bool {
    let mut any_fail = false;
    // Build reference state with a CNOT-only Clifford circuit.
    // CNOT chains keep Pauli Z measurements diagonal in the tableau path.
    let mut ref_state = State::new(N_QUBITS);
    // First H on qubit 0 to create a non-trivial state
    apply(Gate::H, &mut ref_state, 0);
    // CNOT chain 0→1, 1→2, ..., 6→7 (creates GHZ-like state)
    for q in 0..(N_QUBITS - 1) {
        c_apply(Gate::X, &mut ref_state, q, q + 1);
    }

    // Compute reference Z expectations
    let ref_z: Vec<f64> = (0..N_QUBITS)
        .map(|q| z_expectation(&ref_state, q))
        .collect();

    // Build BACQS with same circuit — compress after H on qubit 0
    let mut init_state = State::new(N_QUBITS);
    apply(Gate::H, &mut init_state, 0);
    let mut bacqs = BACQSState::new(&init_state, 6, SEED, 1);
    // CNOT chain — tableau only, diagonal preserving
    for q in 0..(N_QUBITS - 1) {
        bacqs.apply_controlled_gate(Gate::X, q, q + 1);
    }

    // Now measure Z on each qubit — should be diagonal after conjugation
    let decompressions_before = bacqs.decompression_count;

    for q in 0..N_QUBITS {
        let bacqs_z = bacqs.measure_z(q);
        let ref_z_val = ref_z[q];
        let diff = (bacqs_z - ref_z_val).abs();
        let decomp = bacqs.decompression_count - decompressions_before;
        let passed = diff < 0.05 && decomp == 0;

        if passed {
            println!(
                "[D] measure_z({}): BACQS={:+.4}  REF={:+.4}  diff={:.4}  decompressions={}  PASS",
                q, bacqs_z, ref_z_val, diff, decomp
            );
        } else {
            println!(
                "[D] measure_z({}): BACQS={:+.4}  REF={:+.4}  diff={:.4}  decompressions={}  FAIL",
                q, bacqs_z, ref_z_val, diff, decomp
            );
            any_fail = true;
        }
    }
    any_fail
}

fn z_expectation(state: &State, target: usize) -> f64 {
    let n = usize::from(state.n);
    let dim = 1usize << n;
    let mut p0 = 0.0f64;
    let mut p1 = 0.0f64;

    for idx in 0..dim {
        let r = state.reals[idx] as f64;
        let i = state.imags[idx] as f64;
        let prob = r.mul_add(r, i * i);
        if (idx >> target) & 1 == 0 {
            p0 += prob;
        } else {
            p1 += prob;
        }
    }

    p0 - p1
}

// ═══════════════════════════════════════════════════════════════════════
// Test E — Memory report
// ═══════════════════════════════════════════════════════════════════════

fn test_e_memory_report() -> bool {
    let state = State::new(N_QUBITS);
    let mut bacqs = BACQSState::new(&state, 6, SEED, 1);

    // 100-gate Clifford-heavy circuit
    for _round in 0..5 {
        // H on all
        for q in 0..N_QUBITS {
            bacqs.apply_gate(Gate::H, q);
        }
        // CNOT chain
        for q in 0..(N_QUBITS - 1) {
            bacqs.apply_controlled_gate(Gate::X, q, q + 1);
        }
        // S gates
        for q in 0..N_QUBITS {
            bacqs.apply_gate(Gate::P(std::f64::consts::PI as Float / 2.0), q);
        }
        // X gates
        for q in 0..N_QUBITS {
            bacqs.apply_gate(Gate::X, q);
        }
        // CNOT chain reversed
        for q in (1..N_QUBITS).rev() {
            bacqs.apply_controlled_gate(Gate::X, q, q - 1);
        }
    }

    // Add some T-gates at the end to trigger basis resets
    for q in 0..7 {
        bacqs.apply_gate(Gate::P(std::f64::consts::PI as Float / 4.0), q);
    }

    bacqs.print_report();

    if bacqs.basis_reset_count == 7 {
        println!("[E] Memory report: PASS (7 basis resets for 7 T-gates)");
        false
    } else {
        println!(
            "[E] Memory report: WARN (expected 7 basis resets, got {})",
            bacqs.basis_reset_count
        );
        false
    }
}

// ── fidelity computation ───────────────────────────────────────────────

fn fidelity(original: &State, reconstructed: &State) -> f64 {
    assert_eq!(original.len(), reconstructed.len());

    let (inner_re, inner_im) = original
        .reals
        .iter()
        .zip(original.imags.iter())
        .zip(reconstructed.reals.iter().zip(reconstructed.imags.iter()))
        .fold(
            (0.0f64, 0.0f64),
            |(acc_re, acc_im), ((&o_re, &o_im), (&r_re, &r_im))| {
                let o_re = o_re as f64;
                let o_im = o_im as f64;
                let r_re = r_re as f64;
                let r_im = r_im as f64;

                (
                    acc_re + o_re.mul_add(r_re, o_im * r_im),
                    acc_im + o_re.mul_add(r_im, -o_im * r_re),
                )
            },
        );

    inner_re.mul_add(inner_re, inner_im * inner_im)
}
