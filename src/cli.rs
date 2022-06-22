use std::path::PathBuf;

use clap::Parser;
use reqwest::Url;

/// An m3u8 livestream downloader
#[derive(Parser, Clone, Debug)]
#[clap(version, about)]
pub struct Args {
    /// m3u8 playlist URL
    #[clap(value_parser, value_hint = clap::ValueHint::Url)]
    pub m3u8_url: Url,

    /// Log to file
    #[clap(short, long, value_parser, value_hint = clap::ValueHint::FilePath)]
    pub log_file: Option<PathBuf>,

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

    /// Remux video file to mp4 (requires ffmpeg in $PATH)
    #[clap(long, value_parser, default_value_t = false)]
    pub remux: bool,
}

#[derive(Parser, Clone, Debug)]
#[clap(help_heading = "NETWORK OPTIONS")]
pub struct NetworkOptions {
    /// Maximum number of times to retry network requests before giving up
    #[clap(long, value_parser, default_value_t = 10)]
    pub max_retries: u32,

    /// Network requests timeout in seconds
    #[clap(short, long, value_parser, value_name = "SECONDS", default_value_t = 30)]
    pub timeout: u64,

    /// Maximum number of concurrent downloads
    #[clap(short = 'j', long, value_parser, default_value_t = 20)]
    pub max_concurrent_downloads: usize,
}
