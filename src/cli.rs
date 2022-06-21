use std::path::PathBuf;

use clap::Parser;
use reqwest::Url;

#[derive(Parser, Debug)]
pub struct Args {
    /// URL to m3u8 playlist
    #[clap(value_parser, value_name = "URL", value_hint = clap::ValueHint::Url)]
    pub m3u8_url: Url,

    #[clap(flatten)]
    pub download_options: DownloadOptions,

    #[clap(flatten)]
    pub network_options: NetworkOptions,
}

#[derive(Parser, Debug)]
pub struct DownloadOptions {
    /// Output file (without extension)
    #[clap(short, long, value_parser, value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    pub output: PathBuf,

    /// Segments directory
    #[clap(short, long, value_parser, value_name = "PATH", value_hint = clap::ValueHint::DirPath)]
    pub segments_dir: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct NetworkOptions {
    /// Maximum number of times to retry network requests before giving up
    #[clap(long, value_parser, default_value = "10")]
    pub max_retries: u32,

    /// Network requests timeout in seconds
    #[clap(long, value_parser, default_value = "10")]
    pub timeout: u64,
}
