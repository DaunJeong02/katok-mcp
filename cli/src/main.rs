use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;
use katok::{
    config::KatokConfig,
    paths::{default_data_dir, ensure_private_dir},
};

mod cli;
mod commands;
mod support;

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => exit_on_parse_error(error),
    };
    let json = commands::command_requests_json(&cli.command);
    if let Err(error) = run(cli) {
        if json {
            print_json_error(
                error_code(&error),
                &error.to_string(),
                &format!("{error:#}"),
            );
        } else {
            eprintln!("Error: {error:#}");
        }
        std::process::exit(1);
    }
}

/// Print the `{"ok": false, "error": {...}}` envelope every `--json` failure uses.
fn print_json_error(code: &str, message: &str, cause: &str) {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message,
                "cause": cause
            }
        }))
        .unwrap_or_else(|_| {
            "{\"ok\":false,\"error\":{\"code\":\"serialization_failed\"}}".to_string()
        })
    );
}

/// Leave on an argument error, the way `Cli::parse` would, except that a caller who asked for
/// `--json` gets the same structured error envelope as any other failure instead of prose on
/// stderr, and a known flag conflict gets a sentence that says what to do about it.
///
/// `--help` and `--version` arrive here as well; they are not failures and exit as clap
/// intends.
fn exit_on_parse_error(error: clap::Error) -> ! {
    if !error.use_stderr() {
        error.exit();
    }
    // Parsing failed, so there is no `Cli` to ask: the flag can only be read off argv.
    let json = std::env::args_os()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--json");
    let friendly = cli::conflict_message(&error);
    if json {
        let cause = error.render().to_string();
        let message = friendly.unwrap_or_else(|| {
            cause
                .lines()
                .next()
                .unwrap_or("invalid arguments")
                .trim_start_matches("error: ")
                .to_string()
        });
        print_json_error("invalid_arguments", &message, cause.trim_end());
        std::process::exit(error.exit_code());
    }
    match friendly {
        Some(message) => {
            eprintln!("Error: {message}\n\nFor more information, try '--help'.");
            std::process::exit(error.exit_code());
        }
        None => error.exit(),
    }
}

fn run(cli: Cli) -> Result<()> {
    let config = KatokConfig::load(cli.config.as_deref()).context("load config")?;
    let data_dir = match cli.data_dir {
        Some(path) => path,
        None => default_data_dir().context("resolve default data directory")?,
    };
    ensure_private_dir(&data_dir).context("create private data directory")?;
    let archive_path = data_dir.join("archive.sqlite3");
    let semantic_dir = if config.semantic_dir.is_absolute() {
        config.semantic_dir.clone()
    } else {
        data_dir.join(&config.semantic_dir)
    };

    commands::run(cli.command, config, data_dir, archive_path, semantic_dir)
}

fn error_code(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<katok::Error>() {
        Some(katok::Error::SemanticIndexMissing) => "semantic_index_missing",
        Some(katok::Error::SemanticIndexStale(_)) => "semantic_index_stale",
        Some(katok::Error::SemanticIndexBusy(_)) => "semantic_index_busy",
        Some(katok::Error::EmptyQuery) => "empty_query",
        Some(katok::Error::SemanticFeatureDisabled) => "semantic_feature_disabled",
        Some(katok::Error::Sql(_)) => "sqlite_error",
        _ => "command_failed",
    }
}
