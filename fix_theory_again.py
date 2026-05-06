with open('spinoza/src/compression.rs', 'r') as f:
    content = f.read()

import re

content = re.sub(
    r'pub fn theoretical_compression_ratio\(_n_qubits: usize, bits: u8\) -> f64 \{.*?\n\}',
    '''pub fn theoretical_compression_ratio(_n_qubits: usize, bits: u8) -> f64 {
    (64.0 * 2.0) / ((bits as f64) * 2.0)
}''',
    content,
    flags=re.DOTALL
)

with open('spinoza/src/compression.rs', 'w') as f:
    f.write(content)
