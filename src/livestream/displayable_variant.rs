use std::fmt::Display;

use m3u8_rs::VariantStream;

pub struct DisplayableVariant<'a>(&'a VariantStream);

impl<'a> From<&'a VariantStream> for DisplayableVariant<'a> {
    fn from(v: &'a VariantStream) -> Self {
        Self(v)
    }
}

impl<'a> Display for DisplayableVariant<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();

        // Bitrate
        {
            let bandwidth = self.0.bandwidth.trim();
            let mut chars = bandwidth.chars();
            let (trunc, prefix) = match bandwidth.len() {
                0..=4 => (0, ""),
                5..=7 => (3, "K"),
                _ => (6, "M"),
            };
            chars.nth_back(trunc);
            s.push_str(&format!("Bitrate: {:>4} {}b/s", chars.as_str(), prefix));
        }

        // Resolution
        if let Some(ref res) = self.0.resolution {
            s.push_str(&format!("  Resolution: {:>9}", res));
        }

        // Codec
        if let Some(ref codec) = self.0.codecs {
            s.push_str(&format!("  Codec: {}", codec));
        }

        write!(f, "{}", s)
    }
}

impl<'a> From<DisplayableVariant<'a>> for &'a VariantStream {
    fn from(v: DisplayableVariant<'a>) -> Self {
        v.0
    }
}
