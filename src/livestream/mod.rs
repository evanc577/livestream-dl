mod cookies;
mod displayable_variant;
mod encryption;
mod hashable_byte_range;
mod http_client;
mod media_format;
mod playlist_fetcher;
mod remote_data;
mod segment;
mod stopper;
mod stream;
mod utils;

use std::collections::{BinaryHeap, HashMap};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::channel::mpsc;
use futures::StreamExt;
use itertools::Itertools;
use lru::LruCache;
use m3u8_rs::Playlist;
use reqwest::{Client, Url};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies, RetryTransientMiddleware};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::{event, Level};

use self::cookies::CookieJar;
use self::displayable_variant::DisplayableVariant;
pub use self::encryption::Encryption;
pub use self::hashable_byte_range::HashableByteRange;
use self::http_client::HttpClient;
pub use self::media_format::MediaFormat;
use self::playlist_fetcher::m3u8_fetcher;
use self::remote_data::RemoteData;
pub use self::segment::Segment;
pub use self::stopper::Stopper;
pub use self::stream::Stream;
use self::utils::make_absolute_url;
use crate::cli::Args;
use crate::error::LivestreamDLError;
use crate::mux::remux;

#[derive(Debug)]
pub struct Livestream {
    streams: HashMap<Stream, Url>,
    client: HttpClient,
    stopper: Stopper,
    options: Args,
}

type SegmentIdData = (Stream, Segment, Vec<u8>);

impl Stream {
    /// Name of stream if available
    pub fn name(&self) -> Option<String> {
        match self {
            Self::Main => None,
            Self::Video { name: n, .. } => Some(n.clone()),
            Self::Audio { name: n, .. } => Some(n.clone()),
            Self::Subtitle { name: n, .. } => Some(n.clone()),
        }
    }
}

impl Display for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Main => write!(f, "main"),
            Self::Video { name: n, .. } => write!(f, "video_{}", n),
            Self::Audio { name: n, .. } => write!(f, "audio_{}", n),
            Self::Subtitle { name: n, .. } => write!(f, "subtitle_{}", n),
        }
    }
}

impl Livestream {
    /// Create a new Livestream
    ///
    /// If a master playlist is given, choose the highest bitrate variant and download its stream
    /// and all of its alternative media streams
    pub async fn new(url: &Url, options: &Args) -> Result<(Self, Stopper)> {
        // Create reqwest client
        // and accept invalid certs
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(options.network_options.timeout));

        // Add cookie provider if needed
        let client = if let Some(cookies_path) = &options.network_options.cookies {
            let jar = CookieJar::parse_from_file(cookies_path)?;
            client.cookie_provider(Arc::new(jar))
        } else {
            client
        }
        .build()?;

        // Set client retry on failure
        let retry_policy = policies::ExponentialBackoff::builder()
            .retry_bounds(Duration::from_secs(1), Duration::from_secs(10))
            .backoff_exponent(2)
            .build_with_max_retries(options.network_options.max_retries);

        // Build client with middleware
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        // Build HttpClient
        let query_pairs = if options.network_options.copy_query {
            Some(url.query_pairs().collect::<Vec<_>>())
        } else {
            None
        };
        let client = HttpClient::new(client, query_pairs);

        // Get m3u8 playlist
        let resp = client.get(url.clone()).send().await?;
        if !resp.status().is_success() {
            return Err(LivestreamDLError::NetworkRequest(resp).into());
        }

        // Check if m3u8 is master or media
        let final_url = resp.url().clone();
        let bytes = resp.bytes().await?;

        // Parse m3u8 playlist and add streams
        let mut streams = HashMap::new();
        match m3u8_rs::parse_playlist(&bytes) {
            Ok((_, Playlist::MasterPlaylist(p))) => {
                let stream = if !options.download_options.choose_stream {
                    // Pick highest bitrate stream
                    p.variants
                        .iter()
                        .filter_map(|v| Some((v.bandwidth.parse::<u64>().ok()?, v)))
                        .max_by_key(|(x, _)| *x)
                        .ok_or_else(|| anyhow::anyhow!("No streams found"))?
                        .1
                } else {
                    // Show stream chooser
                    let options: Vec<_> = p
                        .variants
                        .iter()
                        .filter_map(|v| Some((v.bandwidth.parse::<u64>().ok()?, v)))
                        .sorted_by_key(|(b, _)| *b)
                        .map(|(_, v)| v)
                        .rev()
                        .map(DisplayableVariant::from)
                        .collect();
                    let response = inquire::Select::new("Choose stream", options).prompt()?;
                    response.into()
                };

                // Add main stream
                streams.insert(Stream::Main, make_absolute_url(url, &stream.uri)?);

                // Closure to find alternative media with matching group id and add them to streams
                let mut add_alternative =
                    |group, f: fn(String, Option<String>) -> Stream| -> Result<()> {
                        for a in p.alternatives.iter().filter(|a| &a.group_id == group) {
                            if let Some(a_url) = &a.uri {
                                streams.insert(
                                    f(a.name.clone(), a.language.clone()),
                                    make_absolute_url(url, a_url)?,
                                );
                            }
                        }
                        Ok(())
                    };

                // Add audio streams
                if let Some(group) = &stream.audio {
                    add_alternative(group, |n, l| Stream::Audio { name: n, lang: l })?;
                }

                // Add video streams
                if let Some(group) = &stream.video {
                    add_alternative(group, |n, l| Stream::Video { name: n, lang: l })?;
                }

                // Add subtitle streams
                if let Some(group) = &stream.subtitles {
                    add_alternative(group, |n, l| Stream::Subtitle { name: n, lang: l })?;
                }
            }
            Ok((_, Playlist::MediaPlaylist(_))) => {
                streams.insert(Stream::Main, final_url);
            }
            Err(_) => {
                return Err(LivestreamDLError::ParseM3u8(final_url.to_string()).into());
            }
        }

        let stopper = Stopper::new();

        Ok((
            Self {
                streams,
                client,
                stopper: stopper.clone(),
                options: options.clone(),
            },
            stopper,
        ))
    }

    /// Download the livestream to disk
    pub async fn download(&self, output: &Path) -> Result<()> {
        // m3u8 reader task handles
        let mut handles = Vec::new();

        let rx = {
            // Create channel for m3u8 fetcher <-> segment downloader tasks
            let (tx, rx) = mpsc::unbounded();

            // Spawn m3u8 reader task
            for (stream, url) in &self.streams {
                let client = self.client.clone();
                let stopper = self.stopper.clone();
                let tx = tx.clone();
                let stream = stream.clone();
                let url = url.clone();

                handles.push(tokio::spawn(async move {
                    m3u8_fetcher(client, stopper.clone(), tx, stream, url).await
                }));
            }

            rx
        };

        // Create segments directory if needed
        let segments_directory = output.join("segments");

        // Cache initializations for each stream
        let init_lrus: HashMap<_, _> = self
            .streams
            .keys()
            .map(|k| {
                (
                    k,
                    Arc::new(Mutex::new(LruCache::new(
                        self.options.network_options.max_concurrent_downloads,
                    ))),
                )
            })
            .collect();

        // Save paths for each downloaded segment
        let mut downloaded_segments = HashMap::new();

        // Download segments
        let mut buffered = rx
            .map(|(stream, seg, encryption)| {
                fetch_segment(
                    &self.client,
                    init_lrus[&stream].clone(),
                    stream,
                    seg,
                    encryption,
                )
            })
            .buffer_unordered(self.options.network_options.max_concurrent_downloads);

        // Save segments to disk in order, break if stopped
        while let Some(x) = tokio::select! {
            y = buffered.next() => { y },
            _ = self.stopper.wait() => { None }
        } {
            // Quit immediately if stopped
            if self.stopper.stopped().await {
                break;
            }

            // Save the segment
            match x {
                Ok(id_data) => {
                    let segment = id_data.1.clone();
                    let res =
                        save_segment(id_data, &mut downloaded_segments, &segments_directory).await;

                    // Log warning if segment failed to download
                    if let Err(e) = res {
                        event!(
                            Level::WARN,
                            "Failed to save {}, reason: {}",
                            segment.url(),
                            e
                        );
                    }
                }
                Err(e) => {
                    event!(Level::WARN, "{:?}", e);
                }
            }
        }

        // Remux if necessary
        if !self.options.download_options.no_remux {
            remux(downloaded_segments, output).await?;
        }

        // Check playlist fetcher task join handles
        for handle in handles {
            handle.await?.context("m3u8 fetcher failed")?;
        }

        Ok(())
    }
}

/// Download segment and save to disk if necessary
async fn fetch_segment(
    client: &HttpClient,
    lru: Arc<Mutex<LruCache<RemoteData, Vec<u8>>>>,
    stream: Stream,
    segment: Segment,
    encryption: Encryption,
) -> Result<SegmentIdData> {
    // Get initialization
    let init_bytes = if let Some(ref i) = segment.initialization {
        // Get cached initialization, otherwise fetch from network
        let mut guard = lru.lock().await;
        let data = guard.get(i).cloned();
        match data {
            Some(d) => d,
            None => {
                let d = i
                    .fetch(client)
                    .await
                    .context("error fetching segment initialization")?
                    .0;
                guard.put(i.clone(), d.clone());
                d
            }
        }
    } else {
        Vec::new()
    };

    // Fetch segment
    let (data_bytes, final_url) = segment
        .data
        .fetch(client)
        .await
        .context("error fetching segment")?;
    let decrypt_data_bytes = encryption.decrypt(client, &data_bytes).await?;

    // Concat initialization and segment
    let bytes = init_bytes
        .into_iter()
        .chain(decrypt_data_bytes.into_iter())
        .collect();

    event!(
        Level::INFO,
        "Downloaded {} {}",
        final_url,
        segment
            .data
            .byte_range_string()
            .unwrap_or_else(|| "".into())
    );

    Ok((stream, segment, bytes))
}

async fn save_segment<P>(
    (stream, mut segment, bytes): SegmentIdData,
    downloaded_segments: &mut HashMap<Stream, BinaryHeap<(Segment, PathBuf)>>,
    segments_directory: P,
) -> Result<()>
where
    P: AsRef<Path>,
{
    // Detect segment format
    segment.format = MediaFormat::detect(bytes.clone()).await?;

    // Create directory if neeeded
    fs::create_dir_all(segments_directory.as_ref()).await?;

    // Save segment to disk
    let file_path = segments_directory.as_ref().join(format!(
        "segment_{}_{}.{}",
        stream,
        segment.id(),
        segment.format.extension()
    ));
    event!(Level::TRACE, "saving to {:?}", &file_path);
    let mut file = fs::File::create(&file_path).await?;
    file.write_all(&bytes).await?;

    // Remember path
    downloaded_segments
        .entry(stream)
        .or_default()
        .push((segment, file_path));

    Ok(())
}
