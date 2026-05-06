# TurboSpin

**TurboSpin** is a quantum circuit simulator workspace based on **[Spinoza](https://github.com/QuState/spinoza)** — a high-performance statevector simulator in Rust ([paper](https://arxiv.org/pdf/2303.01493.pdf)). This repository extends upstream Spinoza with a **`spinoza` CLI**, **optional compressed simulation** (BACQS and RPDQ), and a broader **OpenQASM 2.0** gate surface for running real circuits from disk.

Upstream credits: Spinoza is developed by QuState et al.; this fork layers TurboSpin-specific tooling and parsers while keeping the same core simulation model.

---

## Features

### `spinoza` CLI

Run OpenQASM 2 programs and print the final statevector as plain text (one row per basis state: index, bit string, real/imag amplitude, magnitude, probability).

```bash
cargo run --release -p spinoza --bin spinoza -- \
  --qasm path/to/circuit.qasm \
  --comp-bit <0–8> \
  [--compression-mode bacqs|rpdq]
```

| Flag | Meaning |
|------|---------|
| `--qasm FILE` | OpenQASM 2 source file (`include "qelib1.inc"` resolved relative to the process cwd when possible). |
| `--comp-bit N` | `0` — exact simulation (no compression path): full Spinoza state evolution. `1`–`8` — compressed execution at that bit width using the selected `--compression-mode`. |
| `--compression-mode MODE` | `bacqs` (default) — Clifford-tableau–style BACQS hybrid path. `rpdq` — residual-predictive dithered quantization path. Ignored when `--comp-bit` is `0`. |

Older binaries without `--compression-mode` are supported by some callers via a fallback; new builds should pass the flag whenever `comp_bit > 0`.

### Compression modes (when `--comp-bit` is 1–8)

- **BACQS** — Hybrid simulation that keeps compressed state between steps where possible and expands for unsupported operations (see `spinoza/src/bacqs.rs`).
- **RPDQ** — Alternative compressed pipeline with residual/predictive dither (see `spinoza/src/rpdq.rs`).

Both paths execute the parsed gate list from OpenQASM; multi-control gates are materialized into a dense state when needed, then re-compressed.

### OpenQASM 2 support (extended)

Beyond the gates typically present in upstream examples, TurboSpin’s parser (`spinoza/src/openqasm.rs`) recognizes (non-exhaustive):

- Single-qubit: `id`, `h`, `x`, `y`, `z`, `s`, `sdg`, `t`, `tdg`, `rx`, `ry`, `rz`, `u`, `u1`, `p`-style phases where mapped
- Two-qubit: `cx`, `cz` (implemented as H–CX–H on the target), `swap`, controlled phase `cp`
- Measurement and unsupported constructs produce clear errors rather than silent `todo!()` where applicable

Uses standard `OPENQASM 2.0;` headers and register declarations; **`qelib1.inc`** can be bundled in-repo or next to your circuit depending on layout.

### Library & Python bindings

- **Rust crate** `spinoza` — library API compatible with upstream patterns (gates, circuits, simulation).
- **Python** — **`spynoza`** workspace member exposes PyO3 bindings (same layout as upstream). Install-from-git style workflows match Spinoza’s subdirectory layout.

### Toolchain

`rust-toolchain.toml` pins the **nightly** channel for this workspace (`spynoza` / full build). Adjust if you maintain a subset that builds on stable.

---

## Quickstart

### Build (release CLI)

From the workspace root (`TurboSpin/`):

```bash
cargo build --release -p spinoza --bin spinoza
```

### Run example

```bash
cargo run --release -p spinoza --bin spinoza -- --qasm qasm/your_file.qasm --comp-bit 0
```

Compressed run (BACQS):

```bash
cargo run --release -p spinoza --bin spinoza -- --qasm qasm/your_file.qasm --comp-bit 4 --compression-mode bacqs
```

### Python (`spynoza`)

```bash
pip install git+https://github.com/killianAubry/TurboSpin.git#subdirectory=spynoza
```

This points at **this** repository; replace the URL if you fork again.

---

## Repository layout

| Path | Purpose |
|------|---------|
| `spinoza/` | Core simulator + BACQS/RPDQ + `spinoza` binary |
| `spynoza/` | Python bindings |
| `qasm/` | Sample / test circuits |
| `CHANGELOG.md`, `CONTRIBUTING.md`, `INSTALL.md`, `LICENSE` | Upstream-aligned project docs |

---

## Integration with other tools

Any host (IDE plugin, bench harness, or GUI) can drive TurboSpin by spawning the `spinoza` binary with `--qasm`, `--comp-bit`, and optionally `--compression-mode`. Parse stdout lines for per–basis-state amplitudes and probabilities.

---

## References

```bibtex
@misc{yusufov2023designing,
      title={Designing a Fast and Flexible Quantum State Simulator},
      author={Saveliy Yusufov and Charlee Stefanski and Constantin Gonciulea},
      year={2023},
      eprint={2303.01493},
      archivePrefix={arXiv},
      primaryClass={quant-ph}
}
```

---

## License & contributing

 SPDX / license files are preserved from the Spinoza upstream tree (`LICENSE`). See `CONTRIBUTING.md` for collaboration norms.
