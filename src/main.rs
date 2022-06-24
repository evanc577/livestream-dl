mod cli;
mod livestream;
mod mux;

use std::path::Path;
use std::io;

use anyhow::Result;
use clap::Parser;
use fern::colors::{Color, ColoredLevelConfig};
use livestream::Livestream;
use log::{info, LevelFilter};
use tokio::fs;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    create_output_dir(&args.download_options.output).await?;
    setup_logger(&args.download_options.output)?;

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
            info!("Stopping download");
            stopper.stop().await;
        });
    }

    // Download stream
    livestream.download(&args.download_options).await?;

    Ok(())
}

async fn create_output_dir(output_dir: impl AsRef<Path>) -> Result<()> {
    if output_dir.as_ref().is_dir() {
        eprintln!("Using existing output directory {:?}. This may overwrite any existing files.", output_dir.as_ref());
        eprint!("Is ths OK? [Y/n] ");
        let mut response = String::new();
        let stdin = io::stdin(); // We get `Stdin` here.
        stdin.read_line(&mut response)?;
        if response.trim().to_lowercase() != "y" {
            return Err(anyhow::anyhow!("Not downloading into existing directory"));
        }
    }
    fs::create_dir_all(output_dir).await?;
    Ok(())
}

fn setup_logger(output_dir: impl AsRef<Path>) -> Result<()> {
    // Set up colors
    let colors = ColoredLevelConfig::new().info(Color::Green);

    // Log INFO to stdout
    let stdout_dispatch = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}] {}",
                colors.color(record.level()),
                message
            ))
        })
        .level(LevelFilter::Info)
        .chain(std::io::stdout());

    // Log TRACE to file
    let file_dispatch = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}][{}] {}",
                record.target(),
                record.level(),
                message
            ))
        })
        .level(LevelFilter::Trace)
        .chain(fern::log_file(output_dir.as_ref().join("log.txt"))?);

    // Apply logger
    fern::Dispatch::new()
        .chain(stdout_dispatch)
        .chain(file_dispatch)
        .apply()?;

    Ok(())
}
