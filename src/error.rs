use std::fmt::Display;

use reqwest::Response;

#[allow(dead_code)]
#[derive(Debug)]
pub enum LivestreamDLError {
    NetworkRequest(Response),
    ParseCookie(String),
    ParseM3u8(String),
}

impl Display for LivestreamDLError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NetworkRequest(r) => {
                write!(
                    f,
                    "http request returned status code {} for url: {}",
                    r.status().as_u16(),
                    r.url()
                )
            }
            Self::ParseCookie(s) => {
                write!(f, "failed to parse cookie: {}", s)
            }
            Self::ParseM3u8(s) => {
                write!(f, "failed to parse m3u8 playlist from url: {}", s)
            }
        }
    }
}

impl std::error::Error for LivestreamDLError {}
