use std::fs;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::Path;

use anyhow::Result;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::Url;
use tracing::{event, Level};

use crate::error::LivestreamDLError;

/// Cookie provider wrapping reqwest Jar
pub struct CookieJar(Jar);

impl CookieJar {
    /// Parse cookies from file in Netscape format
    pub fn parse_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let jar = Jar::default();

        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            // Skip empty lines and comments
            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }

            let (domain, cookie) = match parse_cookie(&line) {
                Ok(x) => x,
                Err(e) => {
                    event!(Level::WARN, "{}", e);
                    continue;
                }
            };
            jar.add_cookie_str(&cookie, &domain)
        }

        Ok(Self(jar))
    }
}

fn parse_cookie(line: &str) -> Result<(Url, String)> {
    if let [domain, _, _, _, _, name, value] = line.split('\t').collect::<Vec<_>>().as_slice() {
        let domain = Url::parse(&format!("https://{}", domain.trim_start_matches('.')))
            .map_err(|_| LivestreamDLError::ParseCookie(line.to_owned()))?;
        let cookie = format!("{}={}", name, value);
        Ok((domain, cookie))
    } else {
        Err(LivestreamDLError::ParseCookie(line.to_owned()).into())
    }
}

impl CookieStore for CookieJar {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        url: &url::Url,
    ) {
        self.0.set_cookies(cookie_headers, url)
    }

    fn cookies(&self, url: &url::Url) -> Option<reqwest::header::HeaderValue> {
        self.0.cookies(url)
    }
}
