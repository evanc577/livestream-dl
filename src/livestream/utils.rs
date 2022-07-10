use anyhow::Result;
use reqwest::Url;

/// Create absolute url from a possibly relative url and a base url if needed
pub fn make_absolute_url(base: &Url, url: &str) -> Result<Url> {
    match Url::parse(url) {
        Ok(u) => Ok(u),
        Err(e) if e == url::ParseError::RelativeUrlWithoutBase => Ok(base.join(url)?),
        Err(e) => Err(e.into()),
    }
}
