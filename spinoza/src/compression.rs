use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use spinoza::{core::State, math::Float};

const DEFAULT_BITS: u8 = 4;
const MIN_SCALE: f64 = 1.0e-12;
const STANDARD_NORMAL_DENOMINATOR: f64 = 2.506_628_274_631_000_2;

/// Number of rotated real embedding values per quantization block.
pub const BLOCK_SIZE: usize = 32;

#[derive(Clone, Copy, Debug)]
struct GivensRotation {
    left: usize,
    right: usize,
    theta: f64,
}

#[derive(Clone, Debug)]
struct GaussianCodebook {
    levels: Vec<f64>,
    boundaries: Vec<f64>,
}

/// A compressed surrogate for a Spinoza statevector using seeded rotations and scalar quantization.
#[derive(Clone, Debug)]
pub struct CompressedState {
    /// The packed quantized codeword index for each rotated real coordinate.
    pub packed_indices: Vec<u8>,
    /// Per-block scale factors: each block of BLOCK_SIZE values gets its own local scale
    /// (max absolute value within the block) for adaptive quantization.
    pub block_scales: Vec<f64>,
    /// The input state norm before compression.
    pub norm: f64,
    /// The number of qubits in the represented state.
    pub n_qubits: usize,
    /// The scalar quantizer bit width.
    pub bits: u8,
    /// The deterministic seed that regenerates the Givens rotation schedule.
    pub rotation_seed: u64,
    /// An optional complex QJL sketch of the residual left after the Stage 1-3 reconstruction.
    #[cfg(feature = "qjl")]
    pub residual_sketch: Option<ResidualSketch>,
}

impl CompressedState {
    /// Decompresses the quantized real embedding, inverts the seeded orthogonal rotation,
    /// de-interleaves the complex amplitudes, and renormalizes the reconstructed state.
    pub fn decompress(&self) -> State {
        let count = 1_usize << (self.n_qubits + 1);
        let mut rotated = dequantize_adaptive(
            &self.packed_indices,
            &self.block_scales,
            self.bits,
            count,
        );
        apply_givens_rotation_inverse(&mut rotated, self.rotation_seed, self.n_qubits);

        let mut state = deinterleave_state(&rotated, self.n_qubits);
        renormalize_state(&mut state);
        state
    }
}

/// Flattens a split-complex Spinoza state into the real embedding
/// `[Re(a0), Im(a0), Re(a1), Im(a1), ...]`, applies the seeded structured rotation,
/// and encodes the rotated coordinates with per-block adaptive scalar quantization.
pub fn compress(state: &State, bits: u8, seed: u64) -> CompressedState {
    let bits = if bits == 0 { DEFAULT_BITS } else { bits };
    assert!((1..=8).contains(&bits), "bits must be between 1 and 8");

    let flattened = interleave_state(state);
    let norm = l2_norm(&flattened);

    let mut rotated = flattened.clone();
    apply_givens_rotation(&mut rotated, seed, usize::from(state.n));

    let (indices, block_scales) = quantize_adaptive(&rotated, bits);
    let packed_indices = pack_indices(&indices, bits);

    #[cfg(feature = "qjl")]
    let residual_sketch = {
        let stage_one_reconstruction = dequantize_adaptive(
            &packed_indices,
            &block_scales,
            bits,
            rotated.len(),
        );
        let mut stage_one_rotated = stage_one_reconstruction;
        apply_givens_rotation_inverse(&mut stage_one_rotated, seed, usize::from(state.n));
        let mut approx = deinterleave_state(&stage_one_rotated, usize::from(state.n));
        renormalize_state(&mut approx);
        let residual = residual_from_states(state, &approx);
        let sketch_seed = seed ^ 0xD1B5_4A32_D192_ED03;
        let sketch_size = (usize::from(state.n).max(1) * 8).max(32);
        Some(ResidualSketch::from_components(
            &residual.0,
            &residual.1,
            sketch_seed,
            sketch_size,
            usize::from(state.n),
        ))
    };

    CompressedState {
        packed_indices,
        block_scales,
        norm,
        n_qubits: usize::from(state.n),
        bits,
        rotation_seed: seed,
        #[cfg(feature = "qjl")]
        residual_sketch,
    }
}

/// Computes the squared state fidelity `|<psi|psi_hat>|^2` directly from the split real and
/// imaginary amplitude vectors.
pub fn fidelity(original: &State, reconstructed: &State) -> f64 {
    assert_eq!(original.len(), reconstructed.len());

    let (inner_re, inner_im) = original
        .reals
        .iter()
        .zip(original.imags.iter())
        .zip(reconstructed.reals.iter().zip(reconstructed.imags.iter()))
        .fold(
            (0.0_f64, 0.0_f64),
            |(acc_re, acc_im), ((&o_re, &o_im), (&r_re, &r_im))| {
                let o_re = o_re as f64;
                let o_im = o_im as f64;
                let r_re = r_re as f64;
                let r_im = r_im as f64;

                (
                    acc_re + o_re * r_re + o_im * r_im,
                    acc_im + o_re * r_im - o_im * r_re,
                )
            },
        );

    inner_re.mul_add(inner_re, inner_im * inner_im)
}

/// Applies a deterministic depth-`O(n_qubits)` product of random Givens rotations to the
/// real embedding, approximating a seeded orthogonal Gaussianizing transform without storing
/// a dense matrix.
pub fn apply_givens_rotation(v: &mut Vec<f64>, seed: u64, n_qubits: usize) {
    assert!(
        v.len().is_power_of_two(),
        "the flattened real embedding must have power-of-two length"
    );

    for rotation in givens_schedule(v.len(), seed, n_qubits) {
        apply_single_givens(v, rotation.theta, rotation.left, rotation.right);
    }
}

/// Applies the inverse of the seeded Givens rotation schedule by replaying the same
/// coordinate pairs in reverse order with negated angles.
pub fn apply_givens_rotation_inverse(v: &mut Vec<f64>, seed: u64, n_qubits: usize) {
    assert!(
        v.len().is_power_of_two(),
        "the flattened real embedding must have power-of-two length"
    );

    let schedule = givens_schedule(v.len(), seed, n_qubits);
    for rotation in schedule.into_iter().rev() {
        apply_single_givens(v, -rotation.theta, rotation.left, rotation.right);
    }
}

/// Quantizes a rotated real vector with a `2^bits`-level Gaussian Lloyd-Max scalar quantizer,
/// scaling the standard-normal codebook by the empirical root-mean-square of the input.
#[allow(dead_code)]
pub fn quantize_global(v: &[f64], bits: u8) -> (Vec<u8>, f64) {
    assert!((1..=8).contains(&bits), "bits must be between 1 and 8");

    if v.is_empty() {
        return (Vec::new(), 1.0);
    }

    let scale = empirical_scale(v);
    let codebook = gaussian_codebook(bits);
    let mut indices = Vec::with_capacity(v.len());

    for &value in v {
        let normalized = value / scale;
        indices.push(find_quantization_index(&codebook.boundaries, normalized) as u8);
    }

    (indices, scale)
}

/// Reconstructs a rotated real vector by mapping each stored codeword index back to its
/// Gaussian Lloyd-Max reproduction level and rescaling by the saved empirical standard deviation.
#[allow(dead_code)]
pub fn dequantize_global(indices: &[u8], scale: f64, bits: u8) -> Vec<f64> {
    assert!((1..=8).contains(&bits), "bits must be between 1 and 8");

    let codebook = gaussian_codebook(bits);
    indices
        .iter()
        .map(|&index| {
            let idx = usize::from(index);
            assert!(
                idx < codebook.levels.len(),
                "quantization index exceeds the configured codebook size"
            );
            scale * codebook.levels[idx]
        })
        .collect()
}

/// Quantize a rotated real embedding vector using per-block adaptive scaling.
///
/// Divides `v` into non-overlapping blocks of `BLOCK_SIZE` values. For each block,
/// computes a local scale = max(|v[i]|) within that block, normalizes the block
/// values to [-1, 1], then snaps each normalized value to the nearest Lloyd-Max
/// codebook entry. Returns the packed bit indices and a `Vec` of per-block scales.
///
/// The Lloyd-Max codebook is derived for a standard Gaussian N(0,1) and stored
/// as a slice of 2^bits reconstruction points in [-1, 1].
///
/// # Panics
/// Panics if `bits` is not one of 2, 3, 4, 6, 8.
pub fn quantize_adaptive(v: &[f64], bits: u8) -> (Vec<u8>, Vec<f64>) {
    assert!(!v.is_empty(), "input vector must not be empty");

    let codebook = get_codebook(bits);
    let n_blocks = v.len().div_ceil(BLOCK_SIZE);
    let mut block_scales = Vec::with_capacity(n_blocks);
    let mut indices = Vec::with_capacity(v.len());

    for block in v.chunks(BLOCK_SIZE) {
        // Compute local scale as max absolute value in the block.
        // If all values are zero, default to 1.0 to avoid division by zero
        // — a zero block has no signal, so any scale works; 1.0 is a safe neutral choice.
        let max_abs = block
            .iter()
            .fold(0.0_f64, |acc, &x| acc.max(x.abs()));
        let scale = if max_abs == 0.0 { 1.0 } else { max_abs };
        block_scales.push(scale);

        let inv_scale = 1.0 / scale;
        for &value in block {
            let normalized = value * inv_scale;
            indices.push(nearest_codebook_index(normalized, codebook));
        }
    }

    (indices, block_scales)
}

/// Dequantize a bit-packed index array using per-block scales.
///
/// Unpacks indices, looks up reconstruction points in the Lloyd-Max codebook,
/// and multiplies each reconstruction point by its block's local scale.
/// Must be the exact inverse of `quantize_adaptive` up to quantization error.
///
/// # Panics
/// Panics if `bits` is not one of 2, 3, 4, 6, 8.
pub fn dequantize_adaptive(
    packed: &[u8],
    block_scales: &[f64],
    bits: u8,
    count: usize,
) -> Vec<f64> {
    let codebook = get_codebook(bits);
    let indices = unpack_indices(packed, bits, count);
    assert_eq!(indices.len(), count);

    let mut result = Vec::with_capacity(count);
    let full_blocks = count / BLOCK_SIZE;
    let remainder = count % BLOCK_SIZE;

    for block_idx in 0..full_blocks {
        let scale = block_scales[block_idx];
        let start = block_idx * BLOCK_SIZE;
        for &idx in &indices[start..start + BLOCK_SIZE] {
            result.push(scale * codebook[idx as usize]);
        }
    }

    if remainder > 0 {
        let scale = block_scales[full_blocks];
        let start = full_blocks * BLOCK_SIZE;
        for &idx in &indices[start..] {
            result.push(scale * codebook[idx as usize]);
        }
    }

    result
}

#[cfg(feature = "qjl")]
/// A complex sign sketch of a residual vector under a deterministic complex Gaussian map.
#[derive(Clone, Debug)]
pub struct ResidualSketch {
    signs_re: Vec<i8>,
    signs_im: Vec<i8>,
    seed: u64,
    k: usize,
    n_qubits: usize,
}

#[cfg(feature = "qjl")]
impl ResidualSketch {
    /// Builds a complex Quantum Johnson-Lindenstrauss sign sketch by applying a seeded
    /// complex Gaussian matrix to a residual vector and storing only the signs of the
    /// projected real and imaginary components.
    pub fn from_components(
        reals: &[f64],
        imags: &[f64],
        seed: u64,
        k: usize,
        n_qubits: usize,
    ) -> Self {
        assert_eq!(reals.len(), imags.len());

        let mut signs_re = Vec::with_capacity(k);
        let mut signs_im = Vec::with_capacity(k);

        for projection in complex_gaussian_projection(reals, imags, seed, k) {
            signs_re.push(sign_bit(projection.0));
            signs_im.push(sign_bit(projection.1));
        }

        Self {
            signs_re,
            signs_im,
            seed,
            k,
            n_qubits,
        }
    }

    /// Estimates the complex overlap between the stored residual and another complex vector
    /// using the QJL sign-correlation estimator
    /// `c_hat = sin(pi/4 * (alpha + beta)) + i sin(pi/4 * (gamma - delta))`.
    pub fn estimate_overlap(&self, state: &State) -> ComplexEstimate {
        assert_eq!(usize::from(state.n), self.n_qubits);

        let reals: Vec<f64> = state.reals.iter().map(|&value| value as f64).collect();
        let imags: Vec<f64> = state.imags.iter().map(|&value| value as f64).collect();

        let mut alpha = 0.0;
        let mut beta = 0.0;
        let mut gamma = 0.0;
        let mut delta = 0.0;

        for (index, projection) in
            complex_gaussian_projection(&reals, &imags, self.seed, self.k).enumerate()
        {
            let t_re = f64::from(sign_bit(projection.0));
            let t_im = f64::from(sign_bit(projection.1));
            let s_re = f64::from(self.signs_re[index]);
            let s_im = f64::from(self.signs_im[index]);

            alpha += s_re * t_re;
            beta += s_im * t_im;
            gamma += s_re * t_im;
            delta += s_im * t_re;
        }

        let inv_k = 1.0 / self.k as f64;
        alpha *= inv_k;
        beta *= inv_k;
        gamma *= inv_k;
        delta *= inv_k;

        ComplexEstimate {
            re: (std::f64::consts::FRAC_PI_4 * (alpha + beta)).sin(),
            im: (std::f64::consts::FRAC_PI_4 * (gamma - delta)).sin(),
        }
    }
}

#[cfg(feature = "qjl")]
/// A complex overlap estimate recovered from the QJL sign sketch.
#[derive(Clone, Copy, Debug)]
pub struct ComplexEstimate {
    /// The estimated real component.
    pub re: f64,
    /// The estimated imaginary component.
    pub im: f64,
}

pub(crate) fn interleave_state(state: &State) -> Vec<f64> {
    let mut flattened = Vec::with_capacity(state.len() * 2);
    for (&re, &im) in state.reals.iter().zip(state.imags.iter()) {
        flattened.push(re as f64);
        flattened.push(im as f64);
    }
    flattened
}

pub(crate) fn deinterleave_state(flattened: &[f64], n_qubits: usize) -> State {
    assert_eq!(
        flattened.len(),
        1_usize << (n_qubits + 1),
        "the flattened vector length must match the real embedding dimension"
    );

    let amplitudes = 1_usize << n_qubits;
    let mut reals = Vec::with_capacity(amplitudes);
    let mut imags = Vec::with_capacity(amplitudes);

    for chunk in flattened.chunks_exact(2) {
        reals.push(chunk[0] as Float);
        imags.push(chunk[1] as Float);
    }

    State {
        reals,
        imags,
        n: n_qubits as u8,
    }
}

pub(crate) fn renormalize_state(state: &mut State) {
    let norm = state_norm(state);
    if norm <= MIN_SCALE {
        return;
    }

    let inv_norm = 1.0 / norm;
    for value in &mut state.reals {
        *value = (*value as f64 * inv_norm) as Float;
    }
    for value in &mut state.imags {
        *value = (*value as f64 * inv_norm) as Float;
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

pub(crate) fn l2_norm(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn empirical_scale(values: &[f64]) -> f64 {
    let mean_square = values.iter().map(|value| value * value).sum::<f64>() / values.len() as f64;
    mean_square.sqrt().max(MIN_SCALE)
}

fn givens_schedule(len: usize, seed: u64, n_qubits: usize) -> Vec<GivensRotation> {
    let depth = n_qubits.max(1);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut ordering: Vec<usize> = (0..len).collect();
    let mut rotations = Vec::with_capacity(depth * (len / 2));

    for _ in 0..depth {
        ordering.shuffle(&mut rng);

        for pair in ordering.chunks_exact(2) {
            rotations.push(GivensRotation {
                left: pair[0],
                right: pair[1],
                theta: rng.gen_range(-std::f64::consts::PI..std::f64::consts::PI),
            });
        }
    }

    rotations
}

fn apply_single_givens(v: &mut [f64], theta: f64, left: usize, right: usize) {
    let (sin_theta, cos_theta) = theta.sin_cos();
    let left_value = v[left];
    let right_value = v[right];

    v[left] = cos_theta.mul_add(left_value, -sin_theta * right_value);
    v[right] = sin_theta.mul_add(left_value, cos_theta * right_value);
}

// ── Lloyd-Max codebooks for N(0,1) ──────────────────────────────────────

// 2-bit Lloyd-Max for N(0,1): 4 reconstruction points
static LLOYD_MAX_2: [f64; 4] = [-1.510, -0.453, 0.453, 1.510];

// 3-bit Lloyd-Max for N(0,1): 8 reconstruction points
static LLOYD_MAX_3: [f64; 8] = [
    -2.152, -1.344, -0.756, -0.245, 0.245, 0.756, 1.344, 2.152,
];

// 4-bit Lloyd-Max for N(0,1): 16 reconstruction points
static LLOYD_MAX_4: [f64; 16] = [
    -2.733, -2.069, -1.618, -1.256, -0.942, -0.657, -0.388, -0.130,
     0.130,  0.388,  0.657,  0.942,  1.256,  1.618,  2.069,  2.733,
];

// 6-bit Lloyd-Max for N(0,1): 64 reconstruction points
// Uniformly spaced between -3.5 and 3.5 as approximation
static LLOYD_MAX_6: [f64; 64] = [
         -3.5,  -3.38889,  -3.27778,  -3.16667,  -3.05556,  -2.94444,  -2.83333,  -2.72222,
     -2.61111,      -2.5,  -2.38889,  -2.27778,  -2.16667,  -2.05556,  -1.94444,  -1.83333,
     -1.72222,  -1.61111,      -1.5,  -1.38889,  -1.27778,  -1.16667,  -1.05556, -0.944444,
    -0.833333, -0.722222, -0.611111,      -0.5, -0.388889, -0.277778, -0.166667, -0.0555556,
    0.0555556,  0.166667,  0.277778,  0.388889,       0.5,  0.611111,  0.722222,  0.833333,
     0.944444,   1.05556,   1.16667,   1.27778,   1.38889,       1.5,   1.61111,   1.72222,
      1.83333,   1.94444,   2.05556,   2.16667,   2.27778,   2.38889,       2.5,   2.61111,
      2.72222,   2.83333,   2.94444,   3.05556,   3.16667,   3.27778,   3.38889,       3.5,
];

// 8-bit Lloyd-Max for N(0,1): 256 reconstruction points
// Uniformly spaced between -4.0 and 4.0 as approximation
static LLOYD_MAX_8: [f64; 256] = [
          -4.0,  -3.968627,  -3.937255,  -3.905882,   -3.87451,  -3.843137,  -3.811765,  -3.780392,
      -3.74902,  -3.717647,  -3.686275,  -3.654902,  -3.623529,  -3.592157,  -3.560784,  -3.529412,
     -3.498039,  -3.466667,  -3.435294,  -3.403922,  -3.372549,  -3.341176,  -3.309804,  -3.278431,
     -3.247059,  -3.215686,  -3.184314,  -3.152941,  -3.121569,  -3.090196,  -3.058824,  -3.027451,
     -2.996078,  -2.964706,  -2.933333,  -2.901961,  -2.870588,  -2.839216,  -2.807843,  -2.776471,
     -2.745098,  -2.713725,  -2.682353,   -2.65098,  -2.619608,  -2.588235,  -2.556863,   -2.52549,
     -2.494118,  -2.462745,  -2.431373,       -2.4,  -2.368627,  -2.337255,  -2.305882,   -2.27451,
     -2.243137,  -2.211765,  -2.180392,   -2.14902,  -2.117647,  -2.086275,  -2.054902,  -2.023529,
     -1.992157,  -1.960784,  -1.929412,  -1.898039,  -1.866667,  -1.835294,  -1.803922,  -1.772549,
     -1.741176,  -1.709804,  -1.678431,  -1.647059,  -1.615686,  -1.584314,  -1.552941,  -1.521569,
     -1.490196,  -1.458824,  -1.427451,  -1.396078,  -1.364706,  -1.333333,  -1.301961,  -1.270588,
     -1.239216,  -1.207843,  -1.176471,  -1.145098,  -1.113725,  -1.082353,   -1.05098,  -1.019608,
    -0.9882353, -0.9568627, -0.9254902, -0.8941176, -0.8627451, -0.8313725,       -0.8, -0.7686275,
    -0.7372549, -0.7058824, -0.6745098, -0.6431373, -0.6117647, -0.5803922, -0.5490196, -0.5176471,
    -0.4862745,  -0.454902, -0.4235294, -0.3921569, -0.3607843, -0.3294118, -0.2980392, -0.2666667,
    -0.2352941, -0.2039216,  -0.172549, -0.1411765, -0.1098039, -0.07843137, -0.04705882, -0.01568627,
    0.01568627, 0.04705882, 0.07843137,  0.1098039,  0.1411765,   0.172549,  0.2039216,  0.2352941,
     0.2666667,  0.2980392,  0.3294118,  0.3607843,  0.3921569,  0.4235294,   0.454902,  0.4862745,
     0.5176471,  0.5490196,  0.5803922,  0.6117647,  0.6431373,  0.6745098,  0.7058824,  0.7372549,
     0.7686275,        0.8,  0.8313725,  0.8627451,  0.8941176,  0.9254902,  0.9568627,  0.9882353,
      1.019608,    1.05098,   1.082353,   1.113725,   1.145098,   1.176471,   1.207843,   1.239216,
      1.270588,   1.301961,   1.333333,   1.364706,   1.396078,   1.427451,   1.458824,   1.490196,
      1.521569,   1.552941,   1.584314,   1.615686,   1.647059,   1.678431,   1.709804,   1.741176,
      1.772549,   1.803922,   1.835294,   1.866667,   1.898039,   1.929412,   1.960784,   1.992157,
      2.023529,   2.054902,   2.086275,   2.117647,    2.14902,   2.180392,   2.211765,   2.243137,
       2.27451,   2.305882,   2.337255,   2.368627,        2.4,   2.431373,   2.462745,   2.494118,
       2.52549,   2.556863,   2.588235,   2.619608,    2.65098,   2.682353,   2.713725,   2.745098,
      2.776471,   2.807843,   2.839216,   2.870588,   2.901961,   2.933333,   2.964706,   2.996078,
      3.027451,   3.058824,   3.090196,   3.121569,   3.152941,   3.184314,   3.215686,   3.247059,
      3.278431,   3.309804,   3.341176,   3.372549,   3.403922,   3.435294,   3.466667,   3.498039,
      3.529412,   3.560784,   3.592157,   3.623529,   3.654902,   3.686275,   3.717647,    3.74902,
      3.780392,   3.811765,   3.843137,    3.87451,   3.905882,   3.937255,   3.968627,        4.0,
];

pub(crate) fn get_codebook(bits: u8) -> &'static [f64] {
    match bits {
        2 => &LLOYD_MAX_2,
        3 => &LLOYD_MAX_3,
        4 => &LLOYD_MAX_4,
        6 => &LLOYD_MAX_6,
        8 => &LLOYD_MAX_8,
        _ => panic!("unsupported bit depth: {}", bits),
    }
}

pub(crate) fn nearest_codebook_index(value: f64, codebook: &[f64]) -> u8 {
    let pos = codebook.partition_point(|&x| x < value);
    if pos == 0 {
        return 0;
    }
    if pos == codebook.len() {
        return (codebook.len() - 1) as u8;
    }
    let left_err = (value - codebook[pos - 1]).abs();
    let right_err = (value - codebook[pos]).abs();
    if left_err <= right_err {
        (pos - 1) as u8
    } else {
        pos as u8
    }
}

fn gaussian_codebook(bits: u8) -> GaussianCodebook {
    let levels = 1_usize << bits;
    let mut boundaries = vec![f64::NEG_INFINITY; levels + 1];
    boundaries[levels] = f64::INFINITY;
    for (index, boundary) in boundaries.iter_mut().enumerate().take(levels).skip(1) {
        *boundary = standard_normal_inv_cdf(index as f64 / levels as f64);
    }

    let mut values = vec![0.0; levels];
    update_gaussian_centroids(&boundaries, &mut values);
    enforce_symmetry(&mut values);

    for _ in 0..32 {
        for boundary_index in 1..levels {
            boundaries[boundary_index] =
                0.5 * (values[boundary_index - 1] + values[boundary_index]);
        }
        update_gaussian_centroids(&boundaries, &mut values);
        enforce_symmetry(&mut values);
    }

    for boundary_index in 1..levels {
        boundaries[boundary_index] = 0.5 * (values[boundary_index - 1] + values[boundary_index]);
    }

    GaussianCodebook {
        levels: values,
        boundaries,
    }
}

fn update_gaussian_centroids(boundaries: &[f64], centroids: &mut [f64]) {
    for (index, centroid) in centroids.iter_mut().enumerate() {
        *centroid = truncated_normal_mean(boundaries[index], boundaries[index + 1]);
    }
}

fn enforce_symmetry(values: &mut [f64]) {
    if values.is_empty() {
        return;
    }

    let last = values.len() - 1;
    for index in 0..(values.len() / 2) {
        let magnitude = 0.5 * (values[index].abs() + values[last - index].abs());
        values[index] = -magnitude;
        values[last - index] = magnitude;
    }

    if values.len() % 2 == 1 {
        values[values.len() / 2] = 0.0;
    }
}

fn truncated_normal_mean(lower: f64, upper: f64) -> f64 {
    let pdf_lower = if lower.is_infinite() {
        0.0
    } else {
        standard_normal_pdf(lower)
    };
    let pdf_upper = if upper.is_infinite() {
        0.0
    } else {
        standard_normal_pdf(upper)
    };
    let cdf_lower = if lower.is_infinite() && lower.is_sign_negative() {
        0.0
    } else {
        standard_normal_cdf(lower)
    };
    let cdf_upper = if upper.is_infinite() && upper.is_sign_positive() {
        1.0
    } else {
        standard_normal_cdf(upper)
    };
    let mass = (cdf_upper - cdf_lower).max(MIN_SCALE);

    (pdf_lower - pdf_upper) / mass
}

fn find_quantization_index(boundaries: &[f64], value: f64) -> usize {
    let mut low = 0;
    let mut high = boundaries.len() - 1;

    while low + 1 < high {
        let middle = (low + high) / 2;
        if value < boundaries[middle] {
            high = middle;
        } else {
            low = middle;
        }
    }

    low.min(boundaries.len() - 2)
}

fn standard_normal_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / STANDARD_NORMAL_DENOMINATOR
}

fn standard_normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf_approximation(x / std::f64::consts::SQRT_2))
}

fn erf_approximation(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let polynomial =
        (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t;
    sign * (1.0 - polynomial * (-x * x).exp())
}

fn standard_normal_inv_cdf(probability: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&probability),
        "inverse CDF probability must lie in [0, 1]"
    );
    if probability <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if probability >= 1.0 {
        return f64::INFINITY;
    }

    const P_LOW: f64 = 0.024_25;
    const P_HIGH: f64 = 1.0 - P_LOW;

    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];

    if probability < P_LOW {
        let q = (-2.0 * probability.ln()).sqrt();
        return (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }

    if probability > P_HIGH {
        let q = (-2.0 * (1.0 - probability).ln()).sqrt();
        return -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }

    let q = probability - 0.5;
    let r = q * q;
    (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
        / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
}

#[cfg(feature = "qjl")]
fn residual_from_states(original: &State, reconstructed: &State) -> (Vec<f64>, Vec<f64>) {
    assert_eq!(original.len(), reconstructed.len());

    let reals = original
        .reals
        .iter()
        .zip(reconstructed.reals.iter())
        .map(|(&lhs, &rhs)| lhs as f64 - rhs as f64)
        .collect();
    let imags = original
        .imags
        .iter()
        .zip(reconstructed.imags.iter())
        .map(|(&lhs, &rhs)| lhs as f64 - rhs as f64)
        .collect();

    (reals, imags)
}

#[cfg(feature = "qjl")]
fn complex_gaussian_projection(
    reals: &[f64],
    imags: &[f64],
    seed: u64,
    k: usize,
) -> impl Iterator<Item = (f64, f64)> {
    assert_eq!(reals.len(), imags.len());

    let mut rng = StdRng::seed_from_u64(seed);
    let mut outputs = Vec::with_capacity(k);

    for _ in 0..k {
        let mut projected_re = 0.0;
        let mut projected_im = 0.0;

        for (&re, &im) in reals.iter().zip(imags.iter()) {
            let gaussian_re = sample_standard_normal(&mut rng);
            let gaussian_im = sample_standard_normal(&mut rng);

            projected_re += gaussian_re.mul_add(re, -gaussian_im * im);
            projected_im += gaussian_re.mul_add(im, gaussian_im * re);
        }

        outputs.push((projected_re, projected_im));
    }

    outputs.into_iter()
}

#[cfg(feature = "qjl")]
fn sample_standard_normal(rng: &mut StdRng) -> f64 {
    let u1 = rng.gen_range(f64::EPSILON..1.0);
    let u2 = rng.gen_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

#[cfg(feature = "qjl")]
fn sign_bit(value: f64) -> i8 {
    if value >= 0.0 {
        1
    } else {
        -1
    }
}


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
    // Heap memory: only the buffers are on the heap
    state.reals.capacity() * std::mem::size_of::<Float>() +
    state.imags.capacity() * std::mem::size_of::<Float>()
}

/// Returns the exact heap memory used by a CompressedState in bytes.
/// Must account for: packed_indices Vec, scale f64, norm f64,
/// n_qubits usize, bits u8, rotation_seed u64.
pub fn compressed_size_bytes(compressed: &CompressedState) -> usize {
    let index_bytes = compressed.packed_indices.len();
    let scale_bytes = compressed.block_scales.len() * std::mem::size_of::<f64>();
    let metadata_bytes = std::mem::size_of::<f64>()   // norm
                       + std::mem::size_of::<usize>() // n_qubits
                       + std::mem::size_of::<u8>()    // bits
                       + std::mem::size_of::<u64>();  // rotation_seed
    index_bytes + scale_bytes + metadata_bytes
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
