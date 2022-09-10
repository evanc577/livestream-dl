use std::path::PathBuf;

use clap::Parser;
use reqwest::Url;

/// A HLS (m3u8) livestream downloader
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
    /// Output directory, should be non-existent. If not specified, automatically generate
    #[clap(short, long, value_parser)]
    pub output: Option<PathBuf>,

    /// Don't remux streams to mp4 after download
    #[clap(long, value_parser)]
    pub no_remux: bool,

    /// Show interactive stream picker. If not specified, automatically choose highest bitrate
    /// stream
    #[clap(long, value_parser)]
    pub choose_stream: bool,

    /// (TLS) By default, every SSL connection curl makes is verified to be secure.
    /// This option allows request to proceed and operate even for server connections otherwise considered insecure.
    #[clap(long, value_parser, short='k')]
    pub insecure: bool,
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
        default_value_t = 300
    )]
    pub timeout: u64,

    /// Maximum number of concurrent downloads
    #[clap(short = 'j', long, value_parser, default_value_t = 20)]
    pub max_concurrent_downloads: usize,

    /// Use cookies, path to cookies file in Netscape format
    #[clap(short, long, value_parser, value_hint = clap::ValueHint::FilePath)]
    pub cookies: Option<PathBuf>,

    /// Copy GET query parameters from m3u8_url to all subsequent network requests
    #[clap(short = 'q', long, value_parser)]
    pub copy_query: bool,
}
