use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::channel::mpsc;
use futures::StreamExt;
use m3u8_rs::Playlist;
use reqwest::{Client, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies, RetryTransientMiddleware};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tokio::{fs, process, time};

use crate::cli::{DownloadOptions, NetworkOptions};

#[derive(Debug)]
pub struct Livestream {
    url: Url,
    client: ClientWithMiddleware,
    stopper: Stopper,
    network_options: NetworkOptions,
}

#[derive(Clone, Debug)]
pub struct Stopper(Arc<Notify>);

impl Stopper {
    fn new() -> Self {
        Self(Arc::new(Notify::new()))
    }

    async fn notified(&self) {
        self.0.notified().await;
    }

    pub fn stop(&self) {
        self.0.notify_waiters();
    }
}

impl Livestream {
    pub async fn new(url: &Url, network_options: &NetworkOptions) -> Result<(Self, Stopper)> {
        // Create reqwest client
        let client = Client::builder()
            .timeout(Duration::from_secs(network_options.timeout))
            .build()?;
        let retry_policy = policies::ExponentialBackoff::builder()
            .retry_bounds(Duration::from_secs(1), Duration::from_secs(10))
            .backoff_exponent(2)
            .build_with_max_retries(network_options.max_retries);
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        // Check if m3u8 is master or media
        let resp = client.get(url.clone()).send().await?;
        let final_url = resp.url().clone();
        let bytes = resp.bytes().await?;

        // Get media playlist url
        let media_url = match m3u8_rs::parse_playlist(&bytes) {
            Ok((_, Playlist::MasterPlaylist(p))) => {
                let max_stream = p
                    .variants
                    .into_iter()
                    .filter_map(|v| Some((v.bandwidth.parse::<u64>().ok()?, v)))
                    .max_by_key(|(x, _)| *x)
                    .ok_or_else(|| anyhow::anyhow!("No streams found"))?
                    .1;
                reqwest::Url::parse(&max_stream.uri)?
            }
            Ok((_, Playlist::MediaPlaylist(_))) => final_url,
            Err(e) => {
                return Err(anyhow::anyhow!("Error parsing m3u8 playlist: {}", e));
            }
        };

        let stopper = Stopper::new();

        Ok((
            Self {
                url: media_url,
                client,
                stopper: stopper.clone(),
                network_options: network_options.clone(),
            },
            stopper,
        ))
    }

    pub async fn download(&self, options: &DownloadOptions) -> Result<()> {
        let (tx, rx) = mpsc::unbounded();

        // Spawn m3u8 reader task
        let handle = {
            let url = self.url.clone();
            let client = self.client.clone();
            let stopper = self.stopper.clone();
            tokio::spawn(async move { m3u8_fetcher(client, stopper, tx, url).await })
        };

        // Create segments directory if needed
        if let Some(ref p) = options.segments_directory {
            fs::create_dir_all(&p).await?;
        }

        // Generate output file names
        let output = options.output.with_extension("ts");
        let output_temp = options.output.with_extension("part");

        // Download segments
        let mut file = fs::File::create(&output_temp).await?;
        let mut buffered = rx
            .map(|(s, u)| fetch_segment(&self.client, u, s, options.segments_directory.as_ref()))
            .buffered(self.network_options.max_simultaneous_downloads);
        while let Some(x) = buffered.next().await {
            let (bytes, url) = x?;
            // Append segment to output file
            file.write_all(&bytes).await?;
            println!("Downloaded {}", url.as_str());
        }

        // Rename output file
        fs::rename(output_temp, &output).await?;

        // Remux if necessary
        if options.remux {
            remux(output).await?;
        }

        // Check join handle
        handle.await??;

        Ok(())
    }
}

/// Periodically fetch m3u8 media playlist and send new segments to download task
async fn m3u8_fetcher(
    client: ClientWithMiddleware,
    notify_stop: Stopper,
    tx: mpsc::UnboundedSender<(u64, Url)>,
    url: Url,
) -> Result<()> {
    let mut last_seq = None;

    loop {
        // Fetch playlist
        let now = time::Instant::now();
        let mut found_new_segments = false;
        let bytes = client.get(url.clone()).send().await?.bytes().await?;
        let media_playlist = m3u8_rs::parse_media_playlist(&bytes)
            .map_err(|e| anyhow::anyhow!("{:?}", e))?
            .1;

        // Loop through media segments
        for (i, segment) in (media_playlist.media_sequence..).zip(media_playlist.segments.iter()) {
            // Skip segment if already downloaded
            if let Some(s) = last_seq {
                if s >= i {
                    continue;
                }
            }

            // Segment is new
            last_seq = Some(i);
            found_new_segments = true;

            // Parse URL
            let url = match Url::parse(&segment.uri) {
                Ok(u) => u,
                Err(e) if e == url::ParseError::RelativeUrlWithoutBase => url.join(&segment.uri)?,
                Err(e) => return Err(e.into()),
            };

            // Download segment
            if tx.unbounded_send((i, url)).is_err() {
                return Ok(());
            }
        }

        // Return if stream ended
        if media_playlist.end_list {
            return Ok(());
        }

        if found_new_segments {
            // Wait for target duration or return immediately if manually stopped
            tokio::select! {
                _ = time::sleep_until(now + Duration::from_secs_f32(media_playlist.target_duration)) => (),
                _ = notify_stop.notified() => return Ok(()),
            };
        } else {
            // Wait for half target duration or return immediately if manually stopped
            tokio::select! {
                _ = time::sleep_until(now + Duration::from_secs_f32(media_playlist.target_duration / 2.0)) => (),
                _ = notify_stop.notified() => return Ok(()),
            };
        }
    }
}

/// Download segment and save to disk if necessary
async fn fetch_segment(
    client: &ClientWithMiddleware,
    url: Url,
    segment: u64,
    segment_path: Option<impl AsRef<Path>>,
) -> Result<(Vec<u8>, Url)> {
    // Fetch segment
    let bytes: Vec<u8> = client
        .get(url.clone())
        .send()
        .await?
        .bytes()
        .await?
        .into_iter()
        .collect();

    // Save segment to disk if needed
    if let Some(p) = segment_path {
        let filename = p.as_ref().join(format!("segment_{:010}.ts", segment));
        let mut file = fs::File::create(&filename).await?;
        file.write_all(&bytes).await?;
    }

    Ok((bytes, url))
}

async fn remux(input: impl AsRef<Path>) -> Result<()> {
    println!("Remuxing to mp4");
    let output = input.as_ref().with_extension("mp4");

    // Call ffmpeg to remux video file
    process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input.as_ref())
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .spawn()?
        .wait()
        .await?;

    // Delete original
    fs::remove_file(input.as_ref()).await?;

    Ok(())
}
