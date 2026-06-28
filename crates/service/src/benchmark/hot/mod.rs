pub mod cli;
pub mod result_verifier;
pub mod runner;
pub mod types;

pub use cli::parse_benchmark_args;
pub use runner::run_hot_benchmark;
