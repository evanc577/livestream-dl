use std::path::PathBuf;

use clap::Parser;
use reqwest::Url;

/// An m3u8 livestream downloader
#[derive(Parser, Clone, Debug)]
#[clap(version, about)]
pub struct Args {
    /// URL to m3u8 playlist
    #[clap(value_parser, value_hint = clap::ValueHint::Url)]
    pub m3u8_url: Url,

    #[clap(flatten)]
    pub download_options: DownloadOptions,

    #[clap(flatten)]
    pub network_options: NetworkOptions,
}

#[derive(Parser, Clone, Debug)]
#[clap(help_heading = "DOWNLOAD OPTIONS")]
pub struct DownloadOptions {
    /// Output file (without extension)
    #[clap(short, long, value_parser, value_hint = clap::ValueHint::FilePath)]
    pub output: PathBuf,

    /// Save segments to this directory
    #[clap(short, long, value_parser, value_hint = clap::ValueHint::DirPath)]
    pub segments_directory: Option<PathBuf>,
}

#[derive(Parser, Clone, Debug)]
#[clap(help_heading = "NETWORK OPTIONS")]
pub struct NetworkOptions {
    /// Maximum number of times to retry network requests before giving up
    #[clap(long, value_parser, default_value = "10")]
    pub max_retries: u32,

    /// Network requests timeout in seconds
    #[clap(long, value_parser, default_value = "10")]
    pub timeout: u64,

    /// Maximum number of simultaneous downloads
    #[clap(long, value_parser, default_value = "20")]
    pub max_simultaneous_downloads: usize,
}
