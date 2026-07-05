mod cli;
mod runner;
pub mod types;

pub use cli::parse_benchmark_native_args;
pub use runner::{run_core_worker_from_input_path, run_native_benchmark};
