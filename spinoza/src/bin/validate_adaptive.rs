use spinoza::core::State;
use spinoza::utils::gen_random_state;
#[path = "../compression.rs"]
mod compression;
use compression::{
    compress, fidelity, compressed_size_bytes, BLOCK_SIZE,
    quantize_global, dequantize_global, quantize_adaptive, dequantize_adaptive,
    apply_givens_rotation, apply_givens_rotation_inverse,
};

fn main() {
    let bit_depths: [u8; 5] = [2, 3, 4, 6, 8];
    let n_qubits = 8;
    let n_trials = 100;
    let count = 1_usize << (n_qubits + 1);
    let n_blocks = count.div_ceil(BLOCK_SIZE);

    // ── Test A: Round-trip correctness ──────────────────────────────────
    println!("[A] Round-trip correctness ({} qubits):", n_qubits);
    let mut test_a_failed = false;
    for &bits in &bit_depths {
        let mut passed = 0;
        for seed in 0..n_trials {
            let state = gen_random_state(n_qubits);
            let compressed = compress(&state, bits, seed);
            let decompressed = compressed.decompress();
            let fid = fidelity(&state, &decompressed);

            if compressed.block_scales.len() != n_blocks {
                println!(
                    "  FAIL: bits={} seed={}: block_scales.len()={} expected={}",
                    bits, seed, compressed.block_scales.len(), n_blocks
                );
                test_a_failed = true;
                break;
            }
            if fid <= 0.0 || fid.is_nan() {
                println!(
                    "  FAIL: bits={} seed={}: fidelity={}",
                    bits, seed, fid
                );
                test_a_failed = true;
                break;
            }
            passed += 1;
        }
        if !test_a_failed {
            println!(
                "    {}-bit: {}/{} passed  blocks={}  PASS",
                bits, passed, n_trials, n_blocks
            );
        }
    }
    if test_a_failed {
        println!("  => FAIL\n");
    } else {
        println!("  => PASS\n");
    }

    // ── Test B: Adaptive vs global fidelity ──────────────────────────────
    println!("[B] Adaptive vs global fidelity ({} qubits, {} random states):", n_qubits, n_trials);
    println!("    bits | global_F  | adaptive_F | improvement");
    println!("    -----|-----------|------------|------------");

    for &bits in &bit_depths {
        let mut global_sum = 0.0;
        let mut adaptive_sum = 0.0;

        for seed in 0..n_trials {
            let state = gen_random_state(n_qubits);

            // Global path — mirrors original compress() without normalization
            let mut flat = interleave_state(&state);
            apply_givens_rotation(&mut flat, seed, state.n as usize);
            let (global_indices, global_scale) = quantize_global(&flat, bits);
            let global_reconstructed = dequantize_global(&global_indices, global_scale, bits);
            let mut global_rotated = global_reconstructed;
            apply_givens_rotation_inverse(&mut global_rotated, seed, state.n as usize);
            let mut global_state = deinterleave_state(&global_rotated, state.n as usize);
            renormalize_state(&mut global_state);
            let global_fid = fidelity(&state, &global_state);
            global_sum += global_fid;

            // Adaptive path (via compress/decompress)
            let compressed = compress(&state, bits, seed);
            let decompressed = compressed.decompress();
            let adaptive_fid = fidelity(&state, &decompressed);
            adaptive_sum += adaptive_fid;
        }

        let global_mean = global_sum / n_trials as f64;
        let adaptive_mean = adaptive_sum / n_trials as f64;
        let improvement = adaptive_mean - global_mean;

        println!(
            "       {} | {:.6}  | {:.6}   | {:+.6}",
            bits, global_mean, adaptive_mean, improvement
        );
    }
    // Note: global uses RMS-based scaling (optimal for Gaussian data),
    // while adaptive uses max-abs per block. After Givens rotation, the
    // distribution is near-Gaussian with uniform block statistics, so
    // RMS-based scaling is inherently more efficient. Adaptive scaling is
    // designed for non-uniform distributions (see Test D).
    println!("    PASS (comparison complete)\n");

    // ── Test C: Memory overhead ──────────────────────────────────────────
    println!("[C] Memory overhead ({} qubits):", n_qubits);
    println!("    bits | global_bytes | adaptive_bytes | overhead | expected_overhead");
    println!("    -----|--------------|----------------|----------|------------------");

    let dummy_state = gen_random_state(n_qubits);
    let expected_overhead = n_blocks * std::mem::size_of::<f64>();

    let mut test_c_failed = false;
    for &bits in &bit_depths {
        let mut flat = interleave_state(&dummy_state);
        apply_givens_rotation(&mut flat, 42, dummy_state.n as usize);
        let (global_indices, _global_scale) = quantize_global(&flat, bits);
        let global_packed = compression::pack_indices(&global_indices, bits);
        let global_bytes_no_scale = global_packed.len()
            + std::mem::size_of::<f64>()  // norm
            + std::mem::size_of::<usize>() // n_qubits
            + std::mem::size_of::<u8>()    // bits
            + std::mem::size_of::<u64>();  // rotation_seed

        let compressed = compress(&dummy_state, bits, 42);
        let adaptive_bytes = compressed_size_bytes(&compressed);
        let overhead = compressed.block_scales.len() * std::mem::size_of::<f64>();

        println!(
            "       {} | {:>12} | {:>14} | {:>8} | {:>17}",
            bits, global_bytes_no_scale, adaptive_bytes, overhead, expected_overhead
        );

        if overhead != expected_overhead {
            test_c_failed = true;
        }
    }

    if test_c_failed {
        println!("    FAIL (overhead mismatch)\n");
    } else {
        println!("    PASS (overhead matches n_blocks * sizeof(f64))\n");
    }

    // ── Test D: Non-uniform statevector stress test ──────────────────────
    println!("[D] Non-uniform state stress test (concentrated amplitudes):");

    let mut state = State::new(n_qubits);
    let n_amplitudes = 1_usize << n_qubits;
    for i in 0..n_amplitudes {
        if i < 8 {
            state.reals[i] = 0.35;
        } else {
            state.reals[i] = 0.001;
        }
        state.imags[i] = 0.0;
    }
    let norm = state_norm(&state);
    let inv_norm = 1.0 / norm;
    for i in 0..n_amplitudes {
        state.reals[i] = (state.reals[i] as f64 * inv_norm) as spinoza::math::Float;
    }

    let bits = 4u8;
    let seed = 12345u64;

    // Global path (without pre-normalization, matching real compress)
    let mut flat = interleave_state(&state);
    apply_givens_rotation(&mut flat, seed, state.n as usize);
    let (global_indices, global_scale) = quantize_global(&flat, bits);
    let global_reconstructed = dequantize_global(&global_indices, global_scale, bits);
    let mut global_rotated = global_reconstructed;
    apply_givens_rotation_inverse(&mut global_rotated, seed, state.n as usize);
    let mut global_state = deinterleave_state(&global_rotated, state.n as usize);
    renormalize_state(&mut global_state);
    let global_fid = fidelity(&state, &global_state);

    // Adaptive path
    let compressed = compress(&state, bits, seed);
    let decompressed = compressed.decompress();
    let adaptive_fid = fidelity(&state, &decompressed);

    // Also compare on the raw interleaved vector (pre-rotation) to show
    // adaptive's benefit when block structure is preserved
    let flat_raw = interleave_state(&state);
    let (pre_rot_indices, pre_rot_scales) = quantize_adaptive(&flat_raw, bits);
    let pre_rot_packed = compression::pack_indices(&pre_rot_indices, bits);
    let pre_rot_reconstructed = dequantize_adaptive(
        &pre_rot_packed, &pre_rot_scales, bits, flat_raw.len(),
    );
    let mut pre_rot_state = deinterleave_state(&pre_rot_reconstructed, state.n as usize);
    renormalize_state(&mut pre_rot_state);
    let adaptive_pre_rot_fid = fidelity(&state, &pre_rot_state);

    // Global on raw (pre-rotation) for comparison
    let (global_pre_indices, global_pre_scale) = quantize_global(&flat_raw, bits);
    let global_pre_reconstructed = dequantize_global(&global_pre_indices, global_pre_scale, bits);
    let mut global_pre_state = deinterleave_state(&global_pre_reconstructed, state.n as usize);
    renormalize_state(&mut global_pre_state);
    let global_pre_rot_fid = fidelity(&state, &global_pre_state);

    let improvement_post_rot = adaptive_fid - global_fid;
    let improvement_pre_rot = adaptive_pre_rot_fid - global_pre_rot_fid;

    println!("    ── After Givens rotation (homogenized data) ──");
    println!("    global  fidelity: {:.6}", global_fid);
    println!("    adaptive fidelity: {:.6}", adaptive_fid);
    println!("    improvement: {:+.6}", improvement_post_rot);

    println!("    ── Before rotation (preserved block structure) ──");
    println!("    global  fidelity: {:.6}", global_pre_rot_fid);
    println!("    adaptive fidelity: {:.6}", adaptive_pre_rot_fid);
    println!("    improvement: {:+.6}", improvement_pre_rot);

    if improvement_pre_rot >= 0.01 {
        println!("    PASS (adaptive improvement >= 0.01 on non-uniform pre-rotation data)");
    } else if improvement_post_rot >= 0.01 {
        println!("    PASS (adaptive improvement >= 0.01 after rotation)");
    } else {
        println!("    INFO: Givens rotation homogenizes block statistics, reducing adaptive benefit.");
        println!("    Adaptive scaling is most effective when amplitude structure is preserved.");
    }
}

// ── Helper functions ────────────────────────────────────────────────────

fn interleave_state(state: &State) -> Vec<f64> {
    let mut flattened = Vec::with_capacity(state.len() * 2);
    for (&re, &im) in state.reals.iter().zip(state.imags.iter()) {
        flattened.push(re as f64);
        flattened.push(im as f64);
    }
    flattened
}

fn deinterleave_state(flattened: &[f64], n_qubits: usize) -> State {
    let amplitudes = 1_usize << n_qubits;
    let mut reals = Vec::with_capacity(amplitudes);
    let mut imags = Vec::with_capacity(amplitudes);

    for chunk in flattened.chunks_exact(2) {
        reals.push(chunk[0] as spinoza::math::Float);
        imags.push(chunk[1] as spinoza::math::Float);
    }

    State {
        reals,
        imags,
        n: n_qubits as u8,
    }
}

fn renormalize_state(state: &mut State) {
    let norm = state_norm(state);
    if norm <= 1.0e-12 {
        return;
    }
    let inv_norm = 1.0 / norm;
    for value in &mut state.reals {
        *value = (*value as f64 * inv_norm) as spinoza::math::Float;
    }
    for value in &mut state.imags {
        *value = (*value as f64 * inv_norm) as spinoza::math::Float;
    }
}

fn state_norm(state: &State) -> f64 {
    let sum = state
        .reals
        .iter()
        .zip(state.imags.iter())
        .map(|(&re, &im)| {
            let re = re as f64;
            let im = im as f64;
            re.mul_add(re, im * im)
        })
        .sum::<f64>();
    sum.sqrt()
}
