use std::env;
use std::path::PathBuf;

use poker_hands_storage_service::config::ServiceConfig;
use poker_hands_storage_service::domain::dimension::DimensionRef;
use poker_hands_storage_service::errors::AppError;
use poker_hands_storage_service::http;
use poker_hands_storage_service::http::healthcheck::{
    run_http_healthcheck, HttpHealthcheckOptions,
};
use poker_hands_storage_service::query::QueryService;
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
        Some("query") => run_query(args.collect()),
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

fn run_query(args: Vec<String>) -> Result<(), AppError> {
    let mut data_dir = None;
    let mut strategy = "default".to_owned();
    let mut player_count = None;
    let mut depth_bb = None;
    let mut concrete_line_id = None;
    let mut hole_cards = None;
    let mut verify_checksum = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--strategy" => strategy = next_value(&args, &mut index)?.to_owned(),
            "--player-count" => {
                player_count = Some(parse_u32("--player-count", next_value(&args, &mut index)?)?)
            }
            "--depth-bb" => {
                depth_bb = Some(parse_u32("--depth-bb", next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hole-cards" => hole_cards = Some(next_value(&args, &mut index)?.to_owned()),
            "--verify-checksum" => verify_checksum = true,
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown query option: {option}"
                )))
            }
        }
        index += 1;
    }
    let service = QueryService::open(
        data_dir.ok_or_else(|| AppError::invalid_argument("--data-dir is required"))?,
        3,
        verify_checksum,
    )?;
    let result = service.query(
        &DimensionRef::new(
            strategy,
            player_count.ok_or_else(|| AppError::invalid_argument("--player-count is required"))?,
            depth_bb.ok_or_else(|| AppError::invalid_argument("--depth-bb is required"))?,
        ),
        concrete_line_id
            .ok_or_else(|| AppError::invalid_argument("--concrete-line-id is required"))?,
        &hole_cards.ok_or_else(|| AppError::invalid_argument("--hole-cards is required"))?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result)
            .map_err(|error| AppError::invalid_format(error.to_string()))?
    );
    Ok(())
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}

fn parse_u32(name: &str, value: &str) -> Result<u32, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}

fn print_help() {
    println!(
        "poker-hands-storage-service

Commands:
  query --data-dir <dir> --player-count <count> --depth-bb <bb>
        --concrete-line-id <id> --hole-cards <AA|AKs|AsKh>
        [--strategy <name>] [--verify-checksum]

  healthcheck [--url <http-url>] [--timeout-ms <milliseconds>]

  serve
        Environment: PHS_BIND, PHS_DATA_DIR, PHS_META_DB,
        PHS_MAX_OPEN_HANDLES, PHS_VERIFY_CHECKSUMS, PHS_PREWARM, RUST_LOG"
    );
}
