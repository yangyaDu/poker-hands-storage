pub mod cli;
pub mod report;
pub mod runner;
pub mod types;

pub use cli::parse_benchmark_compare_args;
pub use runner::run_benchmark_compare;
