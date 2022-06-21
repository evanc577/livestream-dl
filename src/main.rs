mod cli;
mod livestream;

use anyhow::Result;
use clap::Parser;
use livestream::{Livestream, LivestreamOptions};

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();

    let livestream = Livestream::new(&args.m3u8_url).await?;
    let livestream_options = LivestreamOptions {
        output: args.output,
        segments_dir: args.segments_dir,
    };
    livestream.download(livestream_options).await?;

    Ok(())
}
