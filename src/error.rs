use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum LivestreamDLError {
    #[error("http request returned status code {0}, url: {1}")]
    NetworkRequest(u16, String),
    #[error("failed to parse cookie: {0}")]
    ParseCookie(String),
    #[error("failed to parse m3u8 playlist from url: {0}")]
    ParseM3u8(String),
}
