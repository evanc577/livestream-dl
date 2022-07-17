use anyhow::Result;
use m3u8_rs::ByteRange;
use reqwest::header::{self, HeaderMap};
use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;

use super::HashableByteRange;

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

    pub async fn fetch(&self, client: &ClientWithMiddleware) -> Result<Vec<u8>> {
        // Add byte range headers if needed
        let mut header_map = HeaderMap::new();
        if let Some(ref range) = self.byte_range_string() {
            header_map.insert(header::RANGE, header::HeaderValue::from_str(range)?);
        }

        // Fetch data
        let bytes: Vec<u8> = client
            .get(self.url().clone())
            .headers(header_map)
            .send()
            .await?
            .bytes()
            .await?
            .into_iter()
            .collect();

        Ok(bytes)
    }
}
