mod cli;
mod livestream;
mod mux;
mod utils;

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use livestream::Livestream;
use tracing::{event, Level};
use tracing_subscriber::filter::{FilterExt, LevelFilter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};

fn main() -> Result<()> {
    // Parse CLI args
    let args = cli::Args::parse();

    // Init logging
    create_output_dir(&args.download_options.output)?;
    init_tracing(&args.download_options.output)?;

    // Run main program
    if let Err(e) = run(args) {
        event!(Level::ERROR, "{}", e);
        std::process::exit(1);
    }

    Ok(())
}

#[tokio::main]
async fn run(args: cli::Args) -> Result<()> {
    let (livestream, stopper) = Livestream::new(&args.m3u8_url, &args.network_options).await?;

    // Gracefully exit on ctrl-c
    {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut stream = {
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::interrupt()).unwrap()
        };
        #[cfg(target_os = "windows")]
        let mut stream = {
            use tokio::signal::windows::ctrl_c;
            ctrl_c().unwrap()
        };

        tokio::spawn(async move {
            stream.recv().await;
            event!(Level::INFO, "Stopping download");
            stopper.stop().await;
        });
    }

    // Download stream
    livestream.download(&args.download_options).await?;

    Ok(())
}

fn create_output_dir(output_dir: impl AsRef<Path>) -> Result<()> {
    if output_dir.as_ref().is_dir() {
        eprintln!(
            "Found existing output directory {:?}, existing files may be overwritten.",
            output_dir.as_ref()
        );
        eprint!("Is ths OK? [y/N] ");
        let mut response = String::new();
        std::io::stdin().read_line(&mut response)?;
        if response.trim().to_lowercase() != "y" {
            return Err(anyhow::anyhow!("Not downloading into existing directory"));
        }
    }
    std::fs::create_dir_all(output_dir)?;
    Ok(())
}

fn init_tracing(output_dir: impl AsRef<Path>) -> Result<()> {
    // Log DEBUG to file unless overridden
    let file = std::fs::File::create(output_dir.as_ref().join("log.txt"))?;
    let file_log = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(file)
        .with_filter(EnvFilter::from_env("LIVESTREAM_DL_LOG").or(LevelFilter::DEBUG));

    // Log INFO to stdout
    let stdout_log = tracing_subscriber::fmt::layer()
        .compact()
        .without_time()
        .with_filter(LevelFilter::INFO);

    // Start loggging
    let subscriber = tracing_subscriber::Registry::default()
        .with(stdout_log)
        .with(file_log);
    tracing::subscriber::set_global_default(subscriber)?;

    Ok(())
}
