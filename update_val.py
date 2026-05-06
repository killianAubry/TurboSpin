with open('spinoza/src/bin/validate_compression.rs', 'r') as f:
    content = f.read()

content = content.replace(
'''use spinoza::compression::{
    compress, fidelity, pack_indices, unpack_indices, 
    compressed_size_bytes, theoretical_compression_ratio, compression_ratio, statevector_size_bytes
};''',
'''#[path = "../compression.rs"]
mod compression;
use compression::{
    compress, fidelity, pack_indices, unpack_indices, 
    compressed_size_bytes, theoretical_compression_ratio, compression_ratio, statevector_size_bytes
};'''
)

with open('spinoza/src/bin/validate_compression.rs', 'w') as f:
    f.write(content)
