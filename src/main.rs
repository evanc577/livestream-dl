mod cli;
mod livestream;

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use fern::colors::{Color, ColoredLevelConfig};
use livestream::Livestream;
use log::{info, LevelFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    setup_logger(args.log_file)?;

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

fn setup_logger(log_file: Option<impl AsRef<Path>>) -> Result<()> {
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

    let dispatch = fern::Dispatch::new().chain(stdout_dispatch);

    let dispatch = if let Some(p) = log_file.as_ref() {
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
            .chain(fern::log_file(p.as_ref())?);
        dispatch.chain(file_dispatch)
    } else {
        dispatch
    };

    dispatch.apply()?;

    Ok(())
}
