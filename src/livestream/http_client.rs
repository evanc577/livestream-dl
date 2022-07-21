use std::fmt::Display;

use reqwest::IntoUrl;
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};

/// Wrapper around ClientWithMiddleware to optionally add additional GET query parameters to every
/// GET request
#[derive(Clone, Debug)]
pub struct HttpClient {
    client: ClientWithMiddleware,
    query_pairs: Option<Vec<(String, String)>>,
}

impl HttpClient {
    pub fn new<T, U, Q>(client: ClientWithMiddleware, query_pairs: Option<Q>) -> Self
    where
        T: Display,
        U: Display,
        Q: IntoIterator<Item = (T, U)>,
    {
        Self {
            client,
            query_pairs: query_pairs.map(|q| {
                q.into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            }),
        }
    }

    pub fn get<T: IntoUrl>(&self, url: T) -> RequestBuilder {
        match &self.query_pairs {
            Some(q) => self.client.get(url).query(q),
            None => self.client.get(url),
        }
    }
}
