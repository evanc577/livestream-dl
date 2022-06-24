use reqwest::Url;

use super::{HashableByteRange, MediaFormat};

/// Type of media segment
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Segment {
    Initialization {
        url: Url,
        byte_range: Option<HashableByteRange>,
    },
    Sequence {
        url: Url,
        byte_range: Option<HashableByteRange>,
        discon_seq: u64,
        seq: u64,
        format: MediaFormat,
    },
}

impl Segment {
    /// URL of segment
    pub fn url(&self) -> &Url {
        match self {
            Self::Initialization { url: u, .. } => u,
            Self::Sequence { url: u, .. } => u,
        }
    }

    /// String identifier of segment
    pub fn id(&self) -> String {
        match self {
            Self::Initialization { .. } => "init".into(),
            Self::Sequence {
                discon_seq: d,
                seq: s,
                ..
            } => format!("d{:010}s{:010}", d, s),
        }
    }

    pub fn byte_range(&self) -> Option<String> {
        let range = match self {
            Self::Initialization {
                byte_range: None, ..
            } => return None,
            Self::Sequence {
                byte_range: None, ..
            } => return None,
            Self::Initialization {
                byte_range: Some(b),
                ..
            } => b,
            Self::Sequence {
                byte_range: Some(b),
                ..
            } => b,
        };

        let start = range.offset.unwrap_or(0);
        let end = start + range.length.saturating_sub(1);

        Some(format!("bytes={}-{}", start, end))
    }
}
