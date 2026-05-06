with open('spinoza/src/compression.rs', 'r') as f:
    content = f.read()

import re

content = re.sub(
    r'pub fn theoretical_compression_ratio\(_n_qubits: usize, bits: u8\) -> f64 \{.*?\n\}',
    '''pub fn theoretical_compression_ratio(_n_qubits: usize, bits: u8) -> f64 {
    // Formula: (64 * 2) / (bits * 2 * 2) to match 64 / bits
    64.0 / (bits as f64)
}''',
    content,
    flags=re.DOTALL
)

with open('spinoza/src/compression.rs', 'w') as f:
    f.write(content)
