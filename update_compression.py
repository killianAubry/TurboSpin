import re

with open('spinoza/src/compression.rs', 'r') as f:
    content = f.read()

# Replace CompressedState struct
content = content.replace(
'''pub struct CompressedState {
    /// The quantized codeword index for each rotated real coordinate.
    pub indices: Vec<u8>,''',
'''pub struct CompressedState {
    /// The packed quantized codeword index for each rotated real coordinate.
    pub packed_indices: Vec<u8>,'''
)

# Replace decompress
content = content.replace(
'''    pub fn decompress(&self) -> State {
        let mut rotated = dequantize(&self.indices, self.scale, self.bits);''',
'''    pub fn decompress(&self) -> State {
        let num_indices = 1_usize << (self.n_qubits + 1);
        let indices = unpack_indices(&self.packed_indices, self.bits, num_indices);
        let mut rotated = dequantize(&indices, self.scale, self.bits);'''
)

# Replace compress
content = content.replace(
'''    let (indices, scale) = quantize(&rotated, bits);

    #[cfg(feature = "qjl")]''',
'''    let (indices, scale) = quantize(&rotated, bits);
    let packed_indices = pack_indices(&indices, bits);

    #[cfg(feature = "qjl")]'''
)
content = content.replace(
'''    CompressedState {
        indices,
        scale,''',
'''    CompressedState {
        packed_indices,
        scale,'''
)

# Add new functions at the end
new_funcs = """

/// Pack a slice of quantized indices (each in range [0, 2^bits)) into a
/// tightly packed byte array. Uses `bits` bits per index with no padding.
/// Example: 256 indices at 3 bits = 768 bits = 96 bytes (not 256 bytes).
pub fn pack_indices(indices: &[u8], bits: u8) -> Vec<u8> {
    assert!((1..=8).contains(&bits), "bits must be between 1 and 8");
    if bits == 8 {
        return indices.to_vec();
    }
    
    let total_bits = indices.len() * (bits as usize);
    let total_bytes = (total_bits + 7) / 8;
    let mut packed = Vec::with_capacity(total_bytes);
    
    let mut accumulator: u16 = 0;
    let mut bits_in_acc: u8 = 0;
    
    for &idx in indices {
        accumulator |= (idx as u16) << bits_in_acc;
        bits_in_acc += bits;
        
        while bits_in_acc >= 8 {
            packed.push((accumulator & 0xFF) as u8);
            accumulator >>= 8;
            bits_in_acc -= 8;
        }
    }
    
    if bits_in_acc > 0 {
        packed.push((accumulator & 0xFF) as u8);
    }
    
    packed
}

/// Unpack a tightly packed byte array back into a Vec<u8> of indices.
/// `count` is the number of indices to extract.
/// Must be the exact inverse of pack_indices.
pub fn unpack_indices(packed: &[u8], bits: u8, count: usize) -> Vec<u8> {
    assert!((1..=8).contains(&bits), "bits must be between 1 and 8");
    if bits == 8 {
        let mut res = packed.to_vec();
        res.truncate(count);
        return res;
    }
    
    let mut unpacked = Vec::with_capacity(count);
    let mut byte_idx = 0;
    let mut accumulator: u16 = 0;
    let mut bits_in_acc: u8 = 0;
    
    let mask = (1 << bits) - 1;
    
    for _ in 0..count {
        while bits_in_acc < bits {
            if byte_idx < packed.len() {
                accumulator |= (packed[byte_idx] as u16) << bits_in_acc;
                bits_in_acc += 8;
                byte_idx += 1;
            } else {
                break; 
            }
        }
        
        unpacked.push((accumulator & mask) as u8);
        accumulator >>= bits;
        bits_in_acc -= bits;
    }
    
    unpacked
}

/// Returns the exact heap memory used by a Spinoza State in bytes.
/// Accounts for both reals and imags Vec<f64> buffers.
pub fn statevector_size_bytes(state: &State) -> usize {
    state.reals.capacity() * std::mem::size_of::<Float>() +
    state.imags.capacity() * std::mem::size_of::<Float>() +
    std::mem::size_of::<State>()
}

/// Returns the exact heap memory used by a CompressedState in bytes.
/// Must account for: packed_indices Vec, scale f64, norm f64,
/// n_qubits usize, bits u8, rotation_seed u64.
pub fn compressed_size_bytes(compressed: &CompressedState) -> usize {
    compressed.packed_indices.capacity() * std::mem::size_of::<u8>() +
    std::mem::size_of::<CompressedState>()
}

/// Returns the theoretical compression ratio given bit depth and qubit count.
/// Formula: (64 * 2) / (bits * 2) — original f64 re+im vs quantized re+im.
pub fn theoretical_compression_ratio(_n_qubits: usize, bits: u8) -> f64 {
    (64.0 * 2.0) / ((bits as f64) * 2.0)
}

/// Returns actual compression ratio: statevector_size / compressed_size.
pub fn compression_ratio(state: &State, compressed: &CompressedState) -> f64 {
    statevector_size_bytes(state) as f64 / compressed_size_bytes(compressed) as f64
}
"""

with open('spinoza/src/compression.rs', 'w') as f:
    f.write(content + new_funcs)
