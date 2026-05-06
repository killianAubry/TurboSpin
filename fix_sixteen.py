with open('spinoza/src/bin/validate_compression.rs', 'r') as f:
    content = f.read()

import re

# Fix step B
new_step_b = '''
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
'''

content = re.sub(
    r'    // Step B.*?    if bit_packing_broken \{\n        std::process::exit\(1\);\n    \}',
    new_step_b.strip(),
    content,
    flags=re.DOTALL
)

with open('spinoza/src/bin/validate_compression.rs', 'w') as f:
    f.write(content)
