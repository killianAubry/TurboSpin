//! Symplectic tableau for Clifford circuit simulation.
//!
//! Implements the Aaronson-Gottesman algorithm (arxiv:quant-ph/0406196)
//! for tracking Clifford operations on n qubits without materializing
//! the statevector. Uses the (x|z|r) convention where each row stores
//! the X-part, Z-part, and phase bit of a Pauli group element.

/// Symplectic tableau representing a Clifford operation on n qubits.
///
/// Stores 2n rows of 2n+1 bits using the (x|z|r) convention.
/// Row i for i in 0..n represents the destabilizer generators D_i = R X_i R†.
/// Row i for i in n..2n represents the stabilizer generators S_{i-n} = R Z_{i-n} R†.
/// The last column r is the phase bit (0 = +1, 1 = -1).
///
/// Reference: Aaronson & Gottesman, "Improved Simulation of Stabilizer Circuits"
/// (arxiv:quant-ph/0406196).
pub struct CliffordTableau {
    n: usize,
    table: Vec<Vec<u8>>, // (2n) rows × (2n+1) cols, values in {0,1}
}

impl CliffordTableau {
    /// Initialize to the identity circuit (standard basis).
    ///
    /// Destabilizers D_i = X_i (x bit set at column i, phase 0).
    /// Stabilizers S_i = Z_i (z bit set at column i, phase 0).
    pub fn new(n: usize) -> Self {
        let num_rows = 2 * n;
        let num_cols = 2 * n + 1;
        let mut table = vec![vec![0u8; num_cols]; num_rows];

        for i in 0..n {
            // Destabilizer D_i = X_i: x bit at column i
            table[i][i] = 1;
            // Stabilizer S_i = Z_i: z bit at column n + i
            table[n + i][n + i] = 1;
        }

        CliffordTableau { n, table }
    }

    // ── accessors ──────────────────────────────────────────────────────

    fn get_x(&self, row: usize, col: usize) -> u8 {
        self.table[row][col]
    }

    fn set_x(&mut self, row: usize, col: usize, v: u8) {
        self.table[row][col] = v;
    }

    fn get_z(&self, row: usize, col: usize) -> u8 {
        self.table[row][col + self.n]
    }

    fn set_z(&mut self, row: usize, col: usize, v: u8) {
        self.table[row][col + self.n] = v;
    }

    fn get_r(&self, row: usize) -> u8 {
        self.table[row][2 * self.n]
    }

    fn set_r(&mut self, row: usize, v: u8) {
        self.table[row][2 * self.n] = v;
    }

    // ── rowsum helper ──────────────────────────────────────────────────

    /// Rowsum from Aaronson-Gottesman: set row h = row h · row i.
    ///
    /// Multiplies the Pauli operators represented by rows h and i,
    /// storing the result in row h. The phase is updated according to:
    ///
    /// P_h · P_i = (-1)^{r_h + r_i + Σ_j z_h[j]·x_i[j]}
    ///             X^{x_h⊕x_i} Z^{z_h⊕z_i}
    fn rowsum(&mut self, h: usize, i: usize) {
        let n = self.n;

        // Phase contribution: Σ_j z_h[j] · x_i[j]  (mod 2)
        let mut phase_contrib = 0u8;
        for j in 0..n {
            phase_contrib ^= self.get_z(h, j) & self.get_x(i, j);
        }

        // Update x and z bits
        for j in 0..n {
            let new_x = self.get_x(h, j) ^ self.get_x(i, j);
            self.set_x(h, j, new_x);
            let new_z = self.get_z(h, j) ^ self.get_z(i, j);
            self.set_z(h, j, new_z);
        }

        // Update phase
        self.set_r(h, self.get_r(h) ^ self.get_r(i) ^ phase_contrib);
    }

    // ── gate operations ────────────────────────────────────────────────

    /// Apply Hadamard on qubit i — O(n) time.
    ///
    /// H exchanges X_i and Z_i, mapping X → Z, Z → X, Y → -Y.
    /// For each row: swap x_i ↔ z_i, then r ⊕= x_i · z_i.
    pub fn h(&mut self, i: usize) {
        for row in 0..2 * self.n {
            let xi = self.get_x(row, i);
            let zi = self.get_z(row, i);
            self.set_x(row, i, zi);
            self.set_z(row, i, xi);
            // Phase flip when both x_i and z_i were set (Y → -Y)
            self.set_r(row, self.get_r(row) ^ (xi & zi));
        }
    }

    /// Apply S gate (phase) on qubit i — O(n) time.
    ///
    /// S = diag(1, i) maps X → Y, Y → -X, Z → Z.
    /// For each row: r ⊕= x_i · z_i, then z_i ⊕= x_i.
    pub fn s(&mut self, i: usize) {
        for row in 0..2 * self.n {
            let xi = self.get_x(row, i);
            let zi = self.get_z(row, i);
            // Phase flip for Paulis with X component (X and Y)
            // Uses old z_i before update
            self.set_r(row, self.get_r(row) ^ (xi & zi));
            // Z_i gets X_i added: maps X→Y, Y→X, Z stays
            self.set_z(row, i, zi ^ xi);
        }
    }

    /// Apply CNOT with control i, target j — O(n) time.
    ///
    /// CNOT maps X_c → X_c X_t, X_t → X_t, Z_c → Z_c, Z_t → Z_c Z_t.
    /// Phase contribution: x_c · z_t · (x_t ⊕ z_c ⊕ 1) for each row.
    pub fn cnot(&mut self, i: usize, j: usize) {
        for row in 0..2 * self.n {
            let xi = self.get_x(row, i);
            let xj = self.get_x(row, j);
            let zi = self.get_z(row, i);
            let zj = self.get_z(row, j);

            // Phase: x_c · z_t · (x_t ⊕ z_c ⊕ 1)
            self.set_r(row, self.get_r(row) ^ (xi & zj & (xj ^ zi ^ 1)));

            // X_t ⊕= X_c
            self.set_x(row, j, xj ^ xi);
            // Z_c ⊕= Z_t
            self.set_z(row, i, zi ^ zj);
        }
    }

    /// Apply X (Pauli) on qubit i — decompose as H·S·S·H.
    ///
    /// X = H Z H and Z = S², so X = H S² H.
    pub fn x(&mut self, i: usize) {
        self.h(i);
        self.s(i);
        self.s(i);
        self.h(i);
    }

    /// Apply Z (Pauli) on qubit i — decompose as S·S.
    ///
    /// Z = S².
    pub fn z(&mut self, i: usize) {
        self.s(i);
        self.s(i);
    }

    /// Apply Y (Pauli) on qubit i — decompose as X·Z.
    ///
    /// Y = i X Z, but the i phase is irrelevant for Clifford conjugation.
    /// Y = H S² H · S² (up to global phase which we ignore for tableau tracking).
    pub fn y(&mut self, i: usize) {
        self.z(i);
        self.x(i);
    }

    // ── core operation ─────────────────────────────────────────────────

    /// Transform a Pauli observable P under the current Clifford R:
    /// returns (transformed_x, transformed_z, phase) where phase is +1 or -1.
    ///
    /// Computes R·P·R† in O(n²) time by expanding P = Π X_i^{x_i} Z_i^{z_i}
    /// and substituting R X_i R† = D_i, R Z_i R† = S_i.
    /// The result is the product of destabilizer/stabilizer rows according to
    /// the bits of the input Pauli.
    pub fn conjugate_pauli(
        &self,
        x_bits: &[u8],
        z_bits: &[u8],
    ) -> (Vec<u8>, Vec<u8>, i8) {
        let n = self.n;
        assert_eq!(x_bits.len(), n);
        assert_eq!(z_bits.len(), n);

        // Work with a scratch row: x | z | r
        let mut scratch_x = vec![0u8; n];
        let mut scratch_z = vec![0u8; n];
        let mut scratch_r = 0u8;

        let mut first = true;

        // Multiply by D_i for each qubit where x_bits[i] = 1
        for i in 0..n {
            if x_bits[i] != 0 {
                if first {
                    // Copy destabilizer row i directly
                    for j in 0..n {
                        scratch_x[j] = self.get_x(i, j);
                        scratch_z[j] = self.get_z(i, j);
                    }
                    scratch_r = self.get_r(i);
                    first = false;
                } else {
                    // Multiply scratch by destabilizer row i
                    self.scratch_rowsum(&mut scratch_x, &mut scratch_z, &mut scratch_r, i);
                }
            }
        }

        // Multiply by S_i for each qubit where z_bits[i] = 1
        for i in 0..n {
            if z_bits[i] != 0 {
                if first {
                    for j in 0..n {
                        scratch_x[j] = self.get_x(n + i, j);
                        scratch_z[j] = self.get_z(n + i, j);
                    }
                    scratch_r = self.get_r(n + i);
                    first = false;
                } else {
                    self.scratch_rowsum(&mut scratch_x, &mut scratch_z, &mut scratch_r, n + i);
                }
            }
        }

        let phase: i8 = if scratch_r == 0 { 1 } else { -1 };
        (scratch_x, scratch_z, phase)
    }

    /// Rowsum using scratch buffers — computes scratch = scratch · tableau[row].
    fn scratch_rowsum(
        &self,
        sx: &mut [u8],
        sz: &mut [u8],
        sr: &mut u8,
        row: usize,
    ) {
        let n = self.n;

        // Phase contribution: Σ_j sz[j] · tableau.x[row][j]
        let mut phase_contrib = 0u8;
        for j in 0..n {
            phase_contrib ^= sz[j] & self.get_x(row, j);
        }

        // XOR x and z bits
        for j in 0..n {
            sx[j] ^= self.get_x(row, j);
            sz[j] ^= self.get_z(row, j);
        }

        *sr ^= self.get_r(row) ^ phase_contrib;
    }

    /// Returns true if the tableau represents the identity circuit.
    ///
    /// Checks that each row i < n has x bit only at column i,
    /// each row i+n has z bit only at column i, and all phases are 0.
    pub fn is_identity(&self) -> bool {
        let n = self.n;
        for row in 0..2 * n {
            if self.get_r(row) != 0 {
                return false;
            }
            for col in 0..n {
                let expected = if row < n {
                    // Destabilizer: X bit at matching column
                    (row == col) as u8
                } else {
                    0u8
                };
                if self.get_x(row, col) != expected {
                    return false;
                }
                let expected_z = if row >= n {
                    // Stabilizer: Z bit at matching column
                    ((row - n) == col) as u8
                } else {
                    0u8
                };
                if self.get_z(row, col) != expected_z {
                    return false;
                }
            }
        }
        true
    }

    /// Compose this tableau with another: self = other · self.
    ///
    /// This applies the Clifford operation represented by `other`
    /// on top of the current tableau. Used during basis reset to
    /// absorb a pending Clifford before re-compressing.
    ///
    /// The composition rule: if self represents R₁ and other represents R₂,
    /// then after composition, self represents R₂ · R₁.
    ///
    /// Algorithm: for each row in self, transform its Pauli through `other`
    /// using other.conjugate_pauli, then replace the row with the result.
    pub fn compose(&mut self, other: &CliffordTableau) {
        assert_eq!(self.n, other.n);
        let n = self.n;

        for row in 0..2 * n {
            let mut x_bits = Vec::with_capacity(n);
            let mut z_bits = Vec::with_capacity(n);
            for col in 0..n {
                x_bits.push(self.get_x(row, col));
                z_bits.push(self.get_z(row, col));
            }

            let (new_x, new_z, phase) = other.conjugate_pauli(&x_bits, &z_bits);

            for col in 0..n {
                self.set_x(row, col, new_x[col]);
                self.set_z(row, col, new_z[col]);
            }
            // Phase: conjugate_pauli returns ±1; map -1 to r=1
            self.set_r(row, if phase == -1 { 1 } else { 0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_tableau() {
        let t = CliffordTableau::new(4);
        assert!(t.is_identity());
    }

    #[test]
    fn test_h_is_self_inverse() {
        let mut t = CliffordTableau::new(4);
        t.h(0);
        t.h(0);
        assert!(t.is_identity());
    }

    #[test]
    fn test_s_is_self_inverse_after_four() {
        let mut t = CliffordTableau::new(4);
        for _ in 0..4 {
            t.s(0);
        }
        assert!(t.is_identity());
    }

    #[test]
    fn test_cnot_is_self_inverse() {
        let mut t = CliffordTableau::new(4);
        t.cnot(0, 1);
        t.cnot(0, 1);
        assert!(t.is_identity());
    }

    #[test]
    fn test_x_decomposition() {
        let mut t = CliffordTableau::new(4);
        t.x(0);
        // X = H·S·S·H should be idempotent: X·X = I
        t.x(0);
        assert!(t.is_identity());
    }

    #[test]
    fn test_z_decomposition() {
        let mut t = CliffordTableau::new(4);
        t.z(0);
        t.z(0);
        assert!(t.is_identity());
    }

    #[test]
    fn test_conjugate_pauli_identity() {
        let t = CliffordTableau::new(4);
        // Under identity, P = X_0 Z_1 should map to itself
        let mut x = vec![0u8; 4];
        let mut z = vec![0u8; 4];
        x[0] = 1; // X on qubit 0
        z[1] = 1; // Z on qubit 1
        let (tx, tz, phase) = t.conjugate_pauli(&x, &z);
        assert_eq!(tx, x);
        assert_eq!(tz, z);
        assert_eq!(phase, 1);
    }

    #[test]
    fn test_conjugate_pauli_h() {
        let mut t = CliffordTableau::new(4);
        t.h(0);
        // Under H, X_0 → Z_0
        let x = vec![1u8, 0, 0, 0];
        let z = vec![0u8; 4];
        let (tx, tz, phase) = t.conjugate_pauli(&x, &z);
        assert_eq!(tx, vec![0u8; 4]);
        assert_eq!(tz, vec![1u8, 0, 0, 0]);
        assert_eq!(phase, 1);
    }

    #[test]
    fn test_conjugate_pauli_h_on_z() {
        let mut t = CliffordTableau::new(4);
        t.h(0);
        // Under H, Z_0 → X_0
        let x = vec![0u8; 4];
        let z = vec![1u8, 0, 0, 0];
        let (tx, tz, phase) = t.conjugate_pauli(&x, &z);
        assert_eq!(tx, vec![1u8, 0, 0, 0]);
        assert_eq!(tz, vec![0u8; 4]);
        assert_eq!(phase, 1);
    }

    #[test]
    fn test_conjugate_pauli_cnot() {
        let mut t = CliffordTableau::new(2);
        t.cnot(0, 1);
        // Under CNOT(0,1), X_0 → X_0 X_1
        let x = vec![1u8, 0];
        let z = vec![0u8; 2];
        let (tx, tz, phase) = t.conjugate_pauli(&x, &z);
        assert_eq!(tx, vec![1u8, 1]);
        assert_eq!(tz, vec![0u8; 2]);
        assert_eq!(phase, 1);
    }

    #[test]
    fn test_cnot_on_z_t() {
        let mut t = CliffordTableau::new(2);
        t.cnot(0, 1);
        // Under CNOT(0,1), Z_1 → Z_0 Z_1
        let x = vec![0u8; 2];
        let z = vec![0u8, 1];
        let (tx, tz, phase) = t.conjugate_pauli(&x, &z);
        assert_eq!(tx, vec![0u8; 2]);
        assert_eq!(tz, vec![1u8, 1]);
        assert_eq!(phase, 1);
    }

    #[test]
    fn test_compose() {
        let mut t1 = CliffordTableau::new(3);
        t1.h(0);
        let mut t2 = CliffordTableau::new(3);
        t2.cnot(0, 1);
        // After compose, t2 = t1·t2 = H·CNOT
        t2.compose(&mut t1);
        // (H·CNOT)·X_0·(H·CNOT)† = H·(X_0 X_1)·H† = Z_0 X_1
        let x = vec![1u8, 0, 0];
        let z = vec![0u8; 3];
        let (tx, tz, phase) = t2.conjugate_pauli(&x, &z);
        assert_eq!(tx, vec![0u8, 1, 0]); // X_1
        assert_eq!(tz, vec![1u8, 0, 0]); // Z_0
        assert_eq!(phase, 1);
    }
}
