mod cli;
mod livestream;

use anyhow::Result;
use clap::Parser;
use livestream::{Livestream, LivestreamOptions};

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    let (livestream, stopper) = Livestream::new(&args.m3u8_url).await?;
    let livestream_options = LivestreamOptions {
        output: args.output,
        segments_dir: args.segments_dir,
    };

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

    livestream.download(livestream_options).await?;

    Ok(())
}
