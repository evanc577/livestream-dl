use anyhow::Result;
use m3u8_rs::ByteRange;
use reqwest::header::{self, HeaderMap};
use reqwest::Url;

use super::http_client::HttpClient;
use super::HashableByteRange;
use crate::error::LivestreamDLError;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RemoteData(Url, Option<HashableByteRange>);

impl RemoteData {
    pub fn new(url: Url, byte_range: Option<ByteRange>) -> Self {
        Self(url, byte_range.map(HashableByteRange::new))
    }

    pub fn url(&self) -> &Url {
        &self.0
    }

    pub fn byte_range_string(&self) -> Option<String> {
        let start = self.1.as_ref()?.offset.unwrap_or(0);
        let end = start + self.1.as_ref()?.length.saturating_sub(1);

        Some(format!("bytes={}-{}", start, end))
    }

    /// Fetch this segment and return (bytes, final url)
    pub async fn fetch(&self, client: &HttpClient) -> Result<(Vec<u8>, Url)> {
        // Add byte range headers if needed
        let mut header_map = HeaderMap::new();
        if let Some(ref range) = self.byte_range_string() {
            header_map.insert(header::RANGE, header::HeaderValue::from_str(range)?);
        }

        // Fetch data
        let resp = client
            .get(self.url().clone())
            .headers(header_map)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(LivestreamDLError::NetworkRequest(resp).into());
        }
        let final_url = resp.url().clone();
        let bytes = resp.bytes().await?.into_iter().collect();

        Ok((bytes, final_url))
    }
}
