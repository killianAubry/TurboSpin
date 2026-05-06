with open('spinoza/src/compression.rs', 'r') as f:
    content = f.read()

import re

# Replace statevector_size_bytes
content = re.sub(
    r'pub fn statevector_size_bytes\(state: &State\) -> usize \{.*?\n\}',
    '''pub fn statevector_size_bytes(state: &State) -> usize {
    // Heap memory: only the buffers are on the heap
    state.reals.capacity() * std::mem::size_of::<Float>() +
    state.imags.capacity() * std::mem::size_of::<Float>()
}''',
    content,
    flags=re.DOTALL
)

# Replace compressed_size_bytes
content = re.sub(
    r'pub fn compressed_size_bytes\(compressed: &CompressedState\) -> usize \{.*?\n\}',
    '''pub fn compressed_size_bytes(compressed: &CompressedState) -> usize {
    // Heap memory: only the packed_indices buffer is on the heap
    compressed.packed_indices.capacity()
}''',
    content,
    flags=re.DOTALL
)

with open('spinoza/src/compression.rs', 'w') as f:
    f.write(content)
