use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
use tokio::{fs, time};

#[derive(Debug)]
pub struct Livestream {
    url: Url,
    client: ClientWithMiddleware,
    stopper: Stopper,
}

#[derive(Debug)]
pub struct LivestreamOptions {
    pub output: PathBuf,
    pub segments_dir: Option<PathBuf>,
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
    pub async fn new(url: &Url) -> Result<(Self, Stopper)> {
        let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
        let retry_policy = policies::ExponentialBackoff::builder().build_with_max_retries(3);
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        let bytes = client.get(url.clone()).send().await?.bytes().await?;
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
            Ok((_, Playlist::MediaPlaylist(_))) => url.clone(),
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
            },
            stopper,
        ))
    }

    pub async fn download(&self, options: LivestreamOptions) -> Result<()> {
        let (tx, rx) = mpsc::unbounded();

        // Spawn m3u8 reader task
        let handle = {
            let url = self.url.clone();
            let client = self.client.clone();
            let stopper = self.stopper.clone();
            tokio::spawn(async move { m3u8_fetcher(client, stopper, tx, url).await })
        };

        // Create segments directory if needed
        if let Some(ref p) = options.segments_dir {
            fs::create_dir_all(&p).await?;
        }

        // Generate output file names
        let (output_temp, output) = {
            let filename = options
                .output
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Output is not a file"))?
                .to_owned();
            let (mut filename_1, mut filename_2) = (filename.clone(), filename);
            filename_1.push(".ts.part");
            filename_2.push(".ts");
            let dir = options.output.parent().unwrap();
            (dir.join(filename_1), dir.join(filename_2))
        };

        // Download segments
        let mut file = fs::File::create(&output_temp).await?;
        let mut buffered = rx
            .map(|(s, u)| fetch_segment(&self.client, u, s, options.segments_dir.as_ref()))
            .buffered(20);
        while let Some(x) = buffered.next().await {
            let (bytes, url) = x?;
            file.write_all(&bytes).await?;
            println!("Downloaded {}", url.as_str());
        }

        // Check join handle
        handle.await??;

        // Rename output file
        fs::rename(output_temp, output).await?;

        Ok(())
    }
}

async fn m3u8_fetcher(
    client: ClientWithMiddleware,
    notify_stop: Stopper,
    tx: mpsc::UnboundedSender<(u64, Url)>,
    url: Url,
) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(5));
    let mut downloaded_segments = HashSet::new();

    loop {
        let bytes = client.get(url.clone()).send().await?.bytes().await?;
        let media_playlist = m3u8_rs::parse_media_playlist(&bytes)
            .map_err(|e| anyhow::anyhow!("{:?}", e))?
            .1;

        for (i, segment) in (media_playlist.media_sequence..).zip(media_playlist.segments.iter()) {
            if downloaded_segments.contains(&segment.uri) {
                continue;
            }
            downloaded_segments.insert(segment.uri.clone());
            if tx.unbounded_send((i, Url::parse(&segment.uri)?)).is_err() {
                return Ok(());
            }
        }

        if media_playlist.end_list {
            return Ok(());
        }

        tokio::select! {
            _ = interval.tick() => {}
            _ = notify_stop.notified() => {
                return Ok(());
            }
        };
    }
}

async fn fetch_segment(
    client: &ClientWithMiddleware,
    url: Url,
    segment: u64,
    segment_path: Option<impl AsRef<Path>>,
) -> Result<(Vec<u8>, Url)> {
    let bytes: Vec<u8> = client
        .get(url.clone())
        .send()
        .await?
        .bytes()
        .await?
        .into_iter()
        .collect();

    if let Some(p) = segment_path {
        let filename = p.as_ref().join(format!("segment_{:010}", segment));
        let mut file = fs::File::create(&filename).await?;
        file.write_all(&bytes).await?;
    }

    Ok((bytes, url))
}
