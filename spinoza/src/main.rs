mod compression;
pub mod tableau;
pub mod bacqs;
pub mod rpdq;

use bacqs::BACQSState;
use clap::Parser;
use rpdq::RpdqState;
use std::path::PathBuf;

use spinoza::{
    circuit::{Controls, QuantumCircuit},
    core::State,
    gates::{apply, c_apply, Gate},
    openqasm,
};

const ROTATION_SEED: u64 = 0x5EED_CAFE_D15C_A11E;
const DITHER_SEED: u64 = 0xD1B5_4A32_D192_ED03;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Run a QASM circuit with optional hybrid Clifford-tableau compression"
)]
struct Args {
    /// OpenQASM 2.0 program file to execute.
    #[arg(long, value_name = "FILE")]
    qasm: PathBuf,

    /// Compression bit depth (1-8). 0 = no compression, use raw Spinoza.
    #[arg(long, value_name = "BITS", default_value = "0")]
    comp_bit: u8,

    /// Compression mode: bacqs (default) or rpdq (residual-predictive dithered).
    #[arg(long, value_name = "MODE", default_value = "bacqs")]
    compression_mode: String,
}

fn main() {
    let args = Args::parse();

    // Load and parse the QASM file
    let circuit = openqasm::load(&args.qasm);

    let n_qubits = get_circuit_qubits(&circuit);
    let final_state = if args.comp_bit == 0 {
        run_spinoza(circuit)
    } else {
        match args.compression_mode.as_str() {
            "rpdq" => run_rpdq(circuit, args.comp_bit),
            _ => run_bacqs(circuit, args.comp_bit),
        }
    };

    print_statevector(&final_state, n_qubits);
}

fn run_spinoza(mut circuit: QuantumCircuit) -> State {
    circuit.execute();
    circuit.get_statevector().clone()
}

fn run_bacqs(mut circuit: QuantumCircuit, bits: u8) -> State {
    let n_qubits = get_circuit_qubits(&circuit);
    let init = State::new(n_qubits);
    let mut bacqs = BACQSState::new(&init, bits, ROTATION_SEED, 1);

    // Drain transformations one by one
    let transformations: Vec<_> = circuit.transformations.drain(..).collect();

    for tr in &transformations {
        match &tr.controls {
            Controls::None => {
                bacqs.apply_gate(tr.gate.clone(), tr.target);
            }
            Controls::Single(control) => {
                bacqs.apply_controlled_gate(tr.gate.clone(), *control, tr.target);
            }
            _ => {
                // Multi-control / mixed-control: materialise, apply, recompress
                let mut state = bacqs.to_state();
                dispatch_gate(&tr.gate, &tr.controls, tr.target, &mut state);
                bacqs = BACQSState::new(&state, bits, ROTATION_SEED, 1);
            }
        }
    }

    bacqs.to_state()
}

fn run_rpdq(mut circuit: QuantumCircuit, bits: u8) -> State {
    let n_qubits = get_circuit_qubits(&circuit);
    let init = State::new(n_qubits);
    let mut rpdq = RpdqState::new(&init, bits, ROTATION_SEED, DITHER_SEED, 1);

    // Drain transformations one by one
    let transformations: Vec<_> = circuit.transformations.drain(..).collect();

    for tr in &transformations {
        match &tr.controls {
            Controls::None => {
                rpdq.apply_gate(tr.gate.clone(), tr.target);
            }
            Controls::Single(control) => {
                rpdq.apply_controlled_gate(tr.gate.clone(), *control, tr.target);
            }
            _ => {
                // Multi-control / mixed-control: materialise, apply, recompress
                let mut state = rpdq.to_state();
                dispatch_gate(&tr.gate, &tr.controls, tr.target, &mut state);
                rpdq = RpdqState::new(&state, bits, ROTATION_SEED, DITHER_SEED, 1);
            }
        }
    }

    rpdq.to_state()
}

fn dispatch_gate(gate: &Gate, controls: &Controls, target: usize, state: &mut State) {
    match controls {
        Controls::None => {
            apply(gate.clone(), state, target);
        }
        Controls::Single(c) => {
            c_apply(gate.clone(), state, *c, target);
        }
        Controls::Ones(controls_vec) => {
            spinoza::gates::mc_apply(
                gate.clone(),
                state,
                controls_vec,
                None,
                target,
            );
        }
        Controls::Mixed { controls, zeros } => {
            spinoza::gates::mc_apply(
                gate.clone(),
                state,
                controls,
                Some(zeros.clone()),
                target,
            );
        }
    }
}

fn get_circuit_qubits(circuit: &QuantumCircuit) -> usize {
    circuit.quantum_registers_info.iter().sum()
}

fn print_statevector(state: &State, n_qubits: usize) {
    for (index, (&re, &im)) in state.reals.iter().zip(state.imags.iter()).enumerate() {
        let re = re as f64;
        let im = im as f64;
        let magnitude = re.mul_add(re, im * im).sqrt();
        let probability = re.mul_add(re, im * im);
        println!(
            "{} | {} | re={:+.12} | im={:+.12} | magnitude={:.12} | probability={:.12}",
            index,
            format!("{:0width$b}", index, width = n_qubits),
            re,
            im,
            magnitude,
            probability
        );
    }
}
