use spinoza::core::State;
use spinoza::utils::gen_random_state;
#[path = "../compression.rs"]
mod compression;
use compression::{
    compress, fidelity, pack_indices, unpack_indices, 
    compressed_size_bytes, theoretical_compression_ratio, compression_ratio, statevector_size_bytes
};
use spinoza::circuit::{QuantumCircuit, QuantumRegister};
use spinoza::gates::{apply, c_apply, Gate};
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;

fn main() {
    let bit_depths: [u8; 5] = [2, 3, 4, 6, 8];
    
    // Step A
    let mut step_a_failed = false;
    for &bits in &bit_depths {
        for i in 0..100 {
            let state = gen_random_state(8);
            let compressed = compress(&state, bits, i as u64);
            let num_indices = 1_usize << (compressed.n_qubits + 1);
            let unpacked = unpack_indices(&compressed.packed_indices, bits, num_indices);
            let repacked = pack_indices(&unpacked, bits);
            
            if repacked != compressed.packed_indices {
                println!("FAIL: Bit depth {}, state {}: repack != compressed.packed_indices", bits, i);
                step_a_failed = true;
                break;
            }
            
            let decompressed = compressed.decompress();
            let fid = fidelity(&state, &decompressed);
            if fid <= 0.0 {
                println!("FAIL: Bit depth {}, state {}: fidelity is {} (<= 0.0)", bits, i, fid);
                step_a_failed = true;
                break;
            }
        }
        if !step_a_failed {
            println!("PASS: Bit depth {} correctness tests passed", bits);
        }
    }

// Step B
    let mut bit_packing_broken = false;
    let mut all_sixteen = true;
    for &bits in &bit_depths {
        let state = gen_random_state(8);
        let compressed = compress(&state, bits, 42);
        
        let n_amplitudes = 1_usize << compressed.n_qubits;
        let comp_size = compressed_size_bytes(&compressed);
        
        let actual_bits_per_amplitude = (comp_size * 8) as f64 / n_amplitudes as f64;
        let expected_bits_per_amplitude = (bits as f64) * 2.0;
        
        if (actual_bits_per_amplitude - 16.0).abs() > 1e-6 {
            all_sixteen = false;
        }
        
        let diff_ratio = (actual_bits_per_amplitude - expected_bits_per_amplitude).abs() / expected_bits_per_amplitude;
        if diff_ratio <= 0.10 {
            println!("PASS: Bit depth {} efficiency: Actual: {:.2} bits/amp, Expected: {:.2} bits/amp", 
                bits, actual_bits_per_amplitude, expected_bits_per_amplitude);
        } else {
            println!("FAIL: Bit depth {} efficiency: Actual: {:.2} bits/amp, Expected: {:.2} bits/amp", 
                bits, actual_bits_per_amplitude, expected_bits_per_amplitude);
            bit_packing_broken = true;
        }
    }
    
    if all_sixteen {
        println!("BIT PACKING NOT WORKING — indices stored as full bytes");
        bit_packing_broken = true;
    }
    
    if bit_packing_broken {
        std::process::exit(1);
    }
    
    // Step C
    println!("\n=== TURBOSPIN COMPRESSION VALIDATION ===");
    println!("bits | raw_bytes | comp_bytes | actual_ratio | theory_ratio | efficiency | fidelity");
    println!("-----|-----------|------------|--------------|--------------|------------|----------");
    
    let mut state = {
        let mut qr = QuantumRegister::new(8);
        let mut qc = QuantumCircuit::new(&mut [&mut qr]);
        qc.h(0);
        qc.cx(0, 1);
        for i in 2..8 {
            qc.h(i);
        }
        qc.execute();
        qc.state
    };
    
    let raw_bytes = statevector_size_bytes(&state);
    
    for &bits in &bit_depths {
        let compressed = compress(&state, bits, 42);
        let comp_bytes = compressed_size_bytes(&compressed);
        let decompressed = compressed.decompress();
        let fid = fidelity(&state, &decompressed);
        
        let actual_ratio = compression_ratio(&state, &compressed);
        let theory_ratio = theoretical_compression_ratio(8, bits);
        let efficiency = (actual_ratio / theory_ratio) * 100.0;
        
        println!("{:>4} | {:>9} | {:>10} | {:>9.2}x | {:>10.2}x | {:>8.1}% | {:.6}", 
            bits, raw_bytes, comp_bytes, actual_ratio, theory_ratio, efficiency, fid);
    }
    
    // Step D
    println!("\n=== FIDELITY DECAY (4-bit, compress after every gate) ===");
    println!("depth |  fidelity | expected_min");
    println!("------|-----------|-------------");
    
    let mut rng = StdRng::seed_from_u64(12345);
    let mut state_ref = gen_random_state(8);
    let mut state_comp = state_ref.clone();
    
    let depths_to_check = [1, 5, 10, 15, 20];
    let mut decay_anomalous_depth = None;
    
    for depth in 1..=20 {
        if depth % 2 == 1 {
            let t = rng.gen_range(0..8);
            apply(Gate::H, &mut state_ref, t);
            apply(Gate::H, &mut state_comp, t);
        } else {
            let mut c = rng.gen_range(0..8);
            let mut t = rng.gen_range(0..8);
            while c == t {
                t = rng.gen_range(0..8);
            }
            c_apply(Gate::X, &mut state_ref, c, t);
            c_apply(Gate::X, &mut state_comp, c, t);
        }
        
        state_comp = compress(&state_comp, 4, depth as u64).decompress();
        
        if depths_to_check.contains(&depth) {
            let fid = fidelity(&state_ref, &state_comp);
            let expected_min = 0.95_f64.powi(depth as i32);
            
            println!("{:>5} | {:.6} | {:.3}", depth, fid, expected_min);
            
            if fid < expected_min && decay_anomalous_depth.is_none() {
                decay_anomalous_depth = Some(depth);
            }
        }
    }
    
    if let Some(d) = decay_anomalous_depth {
        println!("FIDELITY DECAY: ANOMALOUS AT DEPTH {}", d);
    } else {
        println!("FIDELITY DECAY: NORMAL");
    }
}
