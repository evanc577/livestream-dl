use std::path::PathBuf;

use clap::Parser;
use reqwest::Url;

#[derive(Parser, Debug)]
pub struct Args {
    /// URL to m3u8 playlist
    #[clap(value_parser, value_name = "URL", value_hint = clap::ValueHint::Url)]
    pub m3u8_url: Url,

    /// Output file
    #[clap(short, long, value_parser, value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    pub output: PathBuf,

    /// Segments directory
    #[clap(short, long, value_parser, value_name = "PATH", value_hint = clap::ValueHint::DirPath)]
    pub segments_dir: Option<PathBuf>,
}
