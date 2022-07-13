mod cookies;
mod displayable_variant;
mod encryption;
mod hashable_byte_range;
mod media_format;
mod playlist_fetcher;
mod segment;
mod stopper;
mod stream;
mod utils;

use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::channel::mpsc;
use futures::StreamExt;
use itertools::Itertools;
use m3u8_rs::Playlist;
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies, RetryTransientMiddleware};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{event, instrument, Level};

use self::cookies::CookieJar;
use self::displayable_variant::DisplayableVariant;
pub use self::encryption::Encryption;
pub use self::hashable_byte_range::HashableByteRange;
pub use self::media_format::MediaFormat;
use self::playlist_fetcher::m3u8_fetcher;
pub use self::segment::Segment;
pub use self::stopper::Stopper;
pub use self::stream::Stream;
use self::utils::make_absolute_url;
use crate::cli::Args;
use crate::mux::remux;

#[derive(Debug)]
pub struct Livestream {
    streams: HashMap<Stream, Url>,
    client: ClientWithMiddleware,
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
    #[instrument(level = "trace")]
    pub async fn new(url: &Url, options: &Args) -> Result<(Self, Stopper)> {
        // Create reqwest client
        let client =
            Client::builder().timeout(Duration::from_secs(options.network_options.timeout));

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

        // Build client
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        // Get m3u8 playlist
        let resp = client.get(url.clone()).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to fetch m3u8 playlist. Status code: {}",
                resp.status().as_str(),
            ));
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
            Err(e) => {
                return Err(anyhow::anyhow!("Error parsing m3u8 playlist: {}", e));
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
    #[instrument(level = "trace")]
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
        fs::create_dir_all(&segments_directory).await?;

        // Save initializations for each stream
        let mut init_map = HashMap::new();

        // Save paths for each downloaded segment
        let mut downloaded_segments = HashMap::new();

        // Download segments
        let mut buffered = rx
            .map(|(stream, seg, encryption)| fetch_segment(&self.client, stream, seg, encryption))
            .buffered(self.options.network_options.max_concurrent_downloads);

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
            let id_data = x?;
            let segment = id_data.1.clone();
            let res = save_segment(
                id_data,
                &mut init_map,
                &mut downloaded_segments,
                &segments_directory,
            )
            .await;

            // Log warning if segment failed to download
            if let Err(e) = res {
                event!(
                    Level::WARN,
                    "Failed to download {}, reason: {}",
                    segment.url(),
                    e
                );
            }
        }

        // Remux if necessary
        if !self.options.download_options.no_remux {
            remux(downloaded_segments, output).await?;
        }

        // Check playlist fetcher task join handles
        for handle in handles {
            handle.await??;
        }

        Ok(())
    }
}

/// Download segment and save to disk if necessary
#[instrument(level = "trace")]
async fn fetch_segment(
    client: &ClientWithMiddleware,
    stream: Stream,
    segment: Segment,
    encryption: Encryption,
) -> Result<SegmentIdData> {
    let mut header_map = HeaderMap::new();
    let byte_range = segment.byte_range();
    if let Some(ref range) = byte_range {
        header_map.insert(header::RANGE, header::HeaderValue::from_str(range)?);
    }

    // Fetch segment
    let bytes: Vec<u8> = client
        .get(segment.url().clone())
        .headers(header_map)
        .send()
        .await?
        .bytes()
        .await?
        .into_iter()
        .collect();

    // Decrypt
    let bytes = encryption.decrypt(client, &bytes).await?;

    event!(
        Level::INFO,
        "Downloaded {} {}",
        segment.url().as_str(),
        byte_range.unwrap_or_else(|| "".into())
    );

    Ok((stream, segment, bytes))
}

#[instrument(level = "trace", skip(bytes, init_map))]
async fn save_segment<P>(
    (stream, mut segment, mut bytes): SegmentIdData,
    init_map: &mut HashMap<Stream, Vec<u8>>,
    downloaded_segments: &mut HashMap<Stream, Vec<(Segment, PathBuf)>>,
    segments_directory: P,
) -> Result<()>
where
    P: AsRef<Path> + Debug,
{
    // Get ID here before mutably borrowing segment's fields
    let id = segment.id();

    match segment {
        Segment::Initialization { .. } => {
            // If segment is initialization, save data for later use
            init_map.insert(stream, bytes);
        }
        Segment::Sequence { ref mut format, .. } => {
            // If initialization exists, prepend it first
            if let Some(init) = init_map.get(&stream) {
                bytes = init.iter().chain(bytes.iter()).copied().collect();
            }

            // Detect segment format
            *format = MediaFormat::detect(bytes.clone()).await?;

            // Save segment to disk
            let file_path = segments_directory.as_ref().join(format!(
                "segment_{}_{}.{}",
                stream,
                id,
                format.extension()
            ));
            event!(Level::TRACE, "saving to {:?}", &file_path);
            let mut file = fs::File::create(&file_path).await?;
            file.write_all(&bytes).await?;

            // Remember path
            downloaded_segments
                .entry(stream)
                .or_default()
                .push((segment, file_path));
        }
    }

    Ok(())
}
