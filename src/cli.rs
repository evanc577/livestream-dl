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

    #[clap(flatten)]
    pub download_options: DownloadOptions,

    #[clap(flatten)]
    pub network_options: NetworkOptions,
}

#[derive(Parser, Clone, Debug)]
#[clap(help_heading = "DOWNLOAD OPTIONS")]
pub struct DownloadOptions {
    /// Output directory
    #[clap(short, long, value_parser, value_hint = clap::ValueHint::DirPath)]
    pub output: PathBuf,

    /// Remux streams to mp4 file with this name.
    /// Video file will be placed relative to --output directory.
    /// mp4 extension will be automatically added.
    /// Requires ffmpeg in $PATH.
    #[clap(long, value_parser, value_hint = clap::ValueHint::FilePath)]
    pub remux: Option<PathBuf>,
}

#[derive(Parser, Clone, Debug)]
#[clap(help_heading = "NETWORK OPTIONS")]
pub struct NetworkOptions {
    /// Maximum number of times to retry network requests before giving up
    #[clap(long, value_parser, default_value_t = 10)]
    pub max_retries: u32,

    /// Network requests timeout in seconds
    #[clap(
        short,
        long,
        value_parser,
        value_name = "SECONDS",
        default_value_t = 30
    )]
    pub timeout: u64,

    /// Maximum number of concurrent downloads
    #[clap(short = 'j', long, value_parser, default_value_t = 20)]
    pub max_concurrent_downloads: usize,
}
