mod cli;
mod livestream;

use anyhow::Result;
use clap::Parser;
use livestream::Livestream;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    let (livestream, stopper) = Livestream::new(&args.m3u8_url, &args.network_options).await?;

    // Gracefully exit on ctrl-c
    {
        #[cfg(target_os = "linux")]
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
            println!("Stopping download");
            stopper.stop();
        });
    }

    // Download stream
    livestream.download(&args.download_options).await?;

    Ok(())
}
