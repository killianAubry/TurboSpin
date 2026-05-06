with open('spinoza/src/main.rs', 'r') as f:
    content = f.read()

content = content.replace(
    'let payload_bytes = (compressed.indices.len() * usize::from(compressed.bits)).div_ceil(8);',
    'let payload_bytes = compressed.packed_indices.len();'
)

with open('spinoza/src/main.rs', 'w') as f:
    f.write(content)
