use std::time::Duration;

use anyhow::Result;
use futures::channel::mpsc;
use reqwest::Url;
use tokio::time;
use tracing::{event, Level};

use super::http_client::HttpClient;
use super::remote_data::RemoteData;
use super::utils::make_absolute_url;
use super::{Encryption, Segment, Stopper, Stream};
use crate::error::LivestreamDLError;
use crate::livestream::MediaFormat;

/// Periodically fetch m3u8 media playlist and send new segments to download task
pub async fn m3u8_fetcher(
    client: HttpClient,
    notify_stop: Stopper,
    tx: mpsc::UnboundedSender<(Stream, Segment, Encryption)>,
    stream: Stream,
    url: Url,
) -> Result<()> {
    let mut last_seg = None;
    let mut cur_init = None;

    loop {
        // Fetch playlist
        let now = time::Instant::now();
        let mut found_new_segments = false;

        event!(Level::TRACE, "Fetching {}", url.as_str());
        let resp = client.get(url.clone()).send().await?;
        let final_url = resp.url().to_string();
        if !resp.status().is_success() {
            return Err(LivestreamDLError::NetworkRequest(resp).into());
        }
        let bytes = resp.bytes().await?;

        let media_playlist = m3u8_rs::parse_media_playlist(&bytes)
            .map_err(|_| LivestreamDLError::ParseM3u8(final_url))?
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
                encryption = Encryption::new(key, &url, seq).await?;
            }

            // Segment is new
            last_seg = Some((discon_seq, seq));
            found_new_segments = true;

            // Parse URL
            let seg_url = make_absolute_url(&url, &segment.uri)?;

            // Make Initialization
            let init = if let Some(map) = &segment.map {
                let init =
                    RemoteData::new(make_absolute_url(&url, &map.uri)?, map.byte_range.clone());
                cur_init = Some(init.clone());
                Some(init)
            } else {
                cur_init.clone()
            };

            // Download segment
            event!(Level::TRACE, "Found new segment {}", seg_url.as_str());
            if tx
                .unbounded_send((
                    stream.clone(),
                    Segment {
                        data: RemoteData::new(seg_url, segment.byte_range.clone()),
                        discon_seq,
                        seq,
                        format: MediaFormat::Unknown,
                        initialization: init,
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
