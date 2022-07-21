use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum LivestreamDLError {
    #[error("http request returned status code {0}, url: {1}")]
    NetworkRequest(u16, String),
}
