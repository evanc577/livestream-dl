use std::process::Stdio;

use anyhow::Result;
use log::trace;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process;

#[non_exhaustive]
#[allow(dead_code)]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MediaFormat {
    // Containers
    MpegTs, // MPEG-2 transport stream
    FMp4,   // Fragmented MPEG-4

    // Audio
    Aac,  // Advanced audio coding
    Adts, // Audio data transport stream
    Mp3,  // MP3
    Ac3,  // AC-3
    EAc3, // Enhanced AC-3

    // Subtitle
    WebVtt, // WebVTT

    // Unknown
    Unknown,
}

impl MediaFormat {
    pub async fn detect(data: Vec<u8>) -> Result<Self> {
        #[derive(Deserialize)]
        struct FFProbeOuput {
            format: FFProbeFormat,
        }
        #[derive(Deserialize)]
        struct FFProbeFormat {
            format_name: String,
        }

        // Call ffprobe to check format
        let mut cmd = process::Command::new("ffprobe");
        cmd.arg("-loglevel")
            .arg("quiet")
            .arg("-show_entries")
            .arg("format=format_name")
            .arg("-print_format")
            .arg("json")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        trace!("{:?}", cmd);
        let mut child = cmd.spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Can't open ffprobe stdin"))?;

        // Write to ffprobe stdin
        tokio::spawn(async move {
            stdin.write_all(&data).await
        });

        // Run ffprobe
        let output = child.wait_with_output().await?;
        let utf8_output = String::from_utf8(output.stdout)?;

        // Check ffprobe exit status
        if !output.status.success() {
            trace!("ffprobe detect format failed, output: {:?}", utf8_output);
            return Ok(Self::Unknown);
        }

        // Parse ffprobe output
        let parsed: Result<FFProbeOuput, _> = serde_json::from_str(&utf8_output);
        match parsed {
            Err(e) => {
                trace!("Unable to parse ffprobe output: {:?}, {:?}", utf8_output, e);
                Ok(Self::Unknown)
            }
            Ok(o) => {
                let format = match o.format.format_name.as_str().trim() {
                    "mpegts" => Self::MpegTs,
                    "mp3" => Self::Mp3,
                    "mov,mp4,m4a,3gp,3g2,mj2" => Self::FMp4,
                    "webvtt" => Self::WebVtt,
                    _ => Self::Unknown,
                };

                Ok(format)
            }
        }
    }

    pub fn extension(&self) -> String {
        match self {
            Self::MpegTs => "ts",
            Self::FMp4 => "mp4",
            Self::Aac => "m4a",
            Self::Adts => "aac",
            Self::Mp3 => "mp3",
            Self::Ac3 => "ac3",
            Self::EAc3 => "eac3",
            Self::WebVtt => "vtt",

            // Use ".ts" if unknown
            _ => "ts",
        }
        .into()
    }
}