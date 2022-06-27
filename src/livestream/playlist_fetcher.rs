use std::time::Duration;

use anyhow::Result;
use futures::channel::mpsc;
use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;
use tokio::time;
use tracing::{event, Level, instrument};

use super::{Encryption, Segment, Stopper, Stream};
use crate::livestream::{HashableByteRange, MediaFormat};
use crate::utils::make_absolute_url;

/// Periodically fetch m3u8 media playlist and send new segments to download task
#[instrument(skip(client, notify_stop, tx))]
pub async fn m3u8_fetcher(
    client: ClientWithMiddleware,
    notify_stop: Stopper,
    tx: mpsc::UnboundedSender<(Stream, Segment, Encryption)>,
    stream: Stream,
    url: Url,
) -> Result<()> {
    let mut last_seg = None;
    let mut init_downloaded = false;

    loop {
        // Fetch playlist
        let now = time::Instant::now();
        let mut found_new_segments = false;
        event!(Level::TRACE, "Fetching {}", url.as_str());
        let bytes = client.get(url.clone()).send().await?.bytes().await?;
        let media_playlist = m3u8_rs::parse_media_playlist(&bytes)
            .map_err(|e| anyhow::anyhow!("{:?}", e))?
            .1;

        // Loop through media segments
        let mut discon_offset = 0;
        let mut encryption = Encryption::None;
        for (seq, segment) in (media_playlist.media_sequence..).zip(media_playlist.segments.iter())
        {
            // Calculate segment discontinuity
            if segment.discontinuity {
                discon_offset += 1;
            }
            let discon_seq = media_playlist.discontinuity_sequence + discon_offset;

            // Skip segment if already downloaded
            if let Some(s) = last_seg {
                if s >= (discon_seq, seq) {
                    continue;
                }
            }

            // Check encryption
            if let Some(key) = &segment.key {
                encryption = Encryption::new(&client, key, &url, seq).await?;
            }

            // Segment is new
            last_seg = Some((discon_seq, seq));
            found_new_segments = true;

            // Download initialization if needed
            if !init_downloaded {
                if let Some(map) = &segment.map {
                    let init_url = make_absolute_url(&url, &map.uri)?;
                    event!(Level::TRACE, "Found new initialization segment {}", init_url.as_str());
                    if tx
                        .unbounded_send((
                            stream.clone(),
                            Segment::Initialization {
                                url: init_url,
                                byte_range: map
                                    .byte_range
                                    .as_ref()
                                    .map(|b| HashableByteRange::new(b.clone())),
                            },
                            encryption.clone(),
                        ))
                        .is_err()
                    {
                        return Ok(());
                    }
                    init_downloaded = true;
                }
            }

            // Parse URL
            let seg_url = make_absolute_url(&url, &segment.uri)?;

            // Download segment
            event!(Level::TRACE, "Found new segment {}", seg_url.as_str());
            if tx
                .unbounded_send((
                    stream.clone(),
                    Segment::Sequence {
                        url: seg_url,
                        byte_range: segment
                            .byte_range
                            .as_ref()
                            .map(|b| HashableByteRange::new(b.clone())),
                        discon_seq,
                        seq,
                        format: MediaFormat::Unknown,
                    },
                    encryption.clone(),
                ))
                .is_err()
            {
                return Ok(());
            }
        }

        // Return if stream ended
        if media_playlist.end_list {
            event!(Level::TRACE, "Playlist ended");
            return Ok(());
        }

        let wait_duration = if found_new_segments {
            // Wait for target duration if new segments were found
            Duration::from_secs_f32(media_playlist.target_duration)
        } else {
            // Otherwise wait for half target duration
            Duration::from_secs_f32(media_playlist.target_duration / 2.0)
        };

        // Wait until next interval or if stopped
        tokio::select! {
            biased;

            // Not cancel safe, but this is ok because all stoppers are notified when stopped, so
            // fairness doesn't matter
            _ = notify_stop.wait() => {},

            _ = time::sleep_until(now + wait_duration) => {},
        };

        // Return if stopped
        if notify_stop.stopped().await {
            return Ok(());
        }
    }
}
