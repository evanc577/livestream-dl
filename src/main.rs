mod cli;
mod livestream;
mod mux;
mod error;

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
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
    let output = create_output_dir(&args.download_options.output)?;
    init_tracing(&output)?;

    // Run main program
    if let Err(e) = run(args, output) {
        event!(Level::ERROR, "{:?}", e);
        std::process::exit(1);
    }

    Ok(())
}

#[tokio::main]
async fn run(args: cli::Args, output: impl AsRef<Path>) -> Result<()> {
    let (livestream, stopper) = Livestream::new(&args.m3u8_url, &args)
        .await
        .context("error initializing livestream downloader")?;

    // Gracefully exit on ctrl-c
    {
        #[cfg(target_family = "unix")]
        let mut stream = {
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::interrupt()).unwrap()
        };
        #[cfg(target_family = "windows")]
        let mut stream = {
            use tokio::signal::windows::ctrl_c;
            ctrl_c().unwrap()
        };

        tokio::spawn(async move {
            stream.recv().await;
            event!(
                Level::WARN,
                "Stopping download... Press Ctrl-C again to force stop"
            );
            stopper.stop().await;

            tokio::spawn(async move {
                stream.recv().await;
                event!(Level::WARN, "Force stopping process");
                std::process::exit(1);
            });
        });
    }

    // Download stream
    event!(Level::INFO, "Downloading stream to {:?}", output.as_ref());
    livestream.download(output.as_ref()).await?;

    Ok(())
}

fn create_output_dir(output_dir: &Option<impl AsRef<Path> + Debug>) -> Result<PathBuf> {
    let final_output_dir = if let Some(output_dir) = output_dir {
        // If output directory already exists, prompt user to overwrite, otherwise exit
        if output_dir.as_ref().is_dir() {
            let response = inquire::Confirm::new(&format!(
                    "Found existing output directory {:?}, existing files may be overwritten.\nIs this OK?",
                    output_dir.as_ref()
                    ))
                .with_default(false)
                .prompt()?;

            if !response {
                return Err(anyhow::anyhow!("Not downloading into existing directory"));
            }
        }

        output_dir.as_ref().to_path_buf()
    } else {
        // Generate a path
        let now = time::OffsetDateTime::now_local()?;
        let format = time::format_description::parse("[year][month][day]")?;
        let base_file_name = format!("{}-stream-download", now.format(&format)?);
        let mut candidate_path = std::env::current_dir()?.join(&base_file_name);

        // Try different paths until a non-existing one is found
        let mut counter = 1;
        while candidate_path.exists() {
            candidate_path =
                candidate_path.with_file_name(base_file_name.clone() + &format!(".{}", counter));
            counter += 1;
        }

        candidate_path
    };

    // Create directory
    std::fs::create_dir_all(&final_output_dir)?;

    Ok(final_output_dir)
}

fn init_tracing(output_dir: impl AsRef<Path>) -> Result<()> {
    // Enable ANSI support on Windows for colors
    #[cfg(target_family = "windows")]
    let _ = ansi_term::enable_ansi_support();

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

    // Start logging
    let subscriber = tracing_subscriber::Registry::default()
        .with(stdout_log)
        .with(file_log);
    tracing::subscriber::set_global_default(subscriber)?;

    Ok(())
}
