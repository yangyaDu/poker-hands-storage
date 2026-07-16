use std::env;

use poker_hands_storage_service::config::ServiceConfig;
use poker_hands_storage_service::errors::AppError;
use poker_hands_storage_service::http;
use poker_hands_storage_service::http::healthcheck::{
    run_http_healthcheck, HttpHealthcheckOptions,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), AppError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("healthcheck") => run_healthcheck(args.collect()),
        Some("serve") => {
            let remaining: Vec<_> = args.collect();
            if !remaining.is_empty() {
                return Err(AppError::invalid_argument(format!(
                    "Unknown serve option: {}",
                    remaining.join(" ")
                )));
            }
            http::serve(ServiceConfig::from_env()?).await
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(AppError::invalid_argument(format!(
            "Unknown command: {command}"
        ))),
    }
}

fn run_healthcheck(args: Vec<String>) -> Result<(), AppError> {
    let mut options = HttpHealthcheckOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--url" => options.url = next_value(&args, &mut index)?.to_owned(),
            "--timeout-ms" => {
                let timeout_ms = next_value(&args, &mut index)?
                    .parse::<u64>()
                    .map_err(|_| AppError::invalid_argument("--timeout-ms must be an integer"))?;
                if timeout_ms == 0 {
                    return Err(AppError::invalid_argument(
                        "--timeout-ms must be greater than 0",
                    ));
                }
                options.timeout = std::time::Duration::from_millis(timeout_ms);
            }
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown healthcheck option: {option}"
                )))
            }
        }
        index += 1;
    }
    run_http_healthcheck(&options)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}

fn print_help() {
    println!(
        "poker-hands-storage-service

Commands:
  serve
        Environment: PHS_BIND, PHS_DATA_DIR, PHS_MAX_OPEN_HANDLES,
        PHS_METADATA_CACHE_BYTES, PHS_STRATEGY_CACHE_BYTES,
        PHS_VERIFY_CHECKSUMS, PHS_PREWARM, RUST_LOG

  healthcheck [--url <http-url>] [--timeout-ms <milliseconds>]

  help  Print this help message"
    );
}
