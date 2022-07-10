mod concat;

use std::collections::HashMap;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

use anyhow::Result;
use isolang::Language;
use oxilangtag::LanguageTag;
use tokio::{fs, process};
use tracing::{event, instrument, Level};

use self::concat::concat_streams;
use crate::livestream::{Segment, Stream};

/// Remux media files into a single mp4 file with ffmpeg
#[instrument(level = "trace")]
pub async fn remux<P: AsRef<Path> + Debug>(
    downloaded_paths: HashMap<Stream, Vec<(Segment, PathBuf)>>,
    output_dir: P,
) -> Result<()> {
    // Get list of concatenated streams for each discontinuity
    let discons = concat_streams(&downloaded_paths, &output_dir).await?;

    // For each discontinuity, mux into a video file
    for (discon_seq, concatted_streams) in &discons {
        // Generate output name
        const FILE_NAME: &str = "video";
        let output_path = if discons.len() == 1 {
            output_dir.as_ref().join(FILE_NAME)
        } else {
            let file_name = FILE_NAME.to_string() + &format!("_{:010}", discon_seq);
            output_dir.as_ref().join(file_name)
        }
        .with_extension("mp4");

        // Mux streams
        mux_streams(concatted_streams, output_path).await?;
    }

    // Delete original concatenated files
    for concatted_streams in discons.values() {
        for (_, path) in concatted_streams {
            event!(Level::TRACE, "Removing {}", path.to_string_lossy());
            fs::remove_file(path).await?;
        }
    }

    Ok(())
}

/// Mux streams into a video file
async fn mux_streams<P: AsRef<Path> + Debug>(
    streams: &Vec<(&Stream, PathBuf)>,
    output_path: P,
) -> Result<()> {
    // Call ffmpeg to remux video file
    let mut cmd = process::Command::new("ffmpeg");
    cmd.arg("-y").arg("-copyts");

    // Set ffmpeg input files
    for (_, path) in streams {
        cmd.arg("-i").arg(path);
    }

    // Map all streams
    for i in 0..streams.len() {
        cmd.arg("-map").arg(i.to_string());
    }

    // Add metadata
    add_metadata(&mut cmd, streams);

    event!(Level::INFO, "ffmpeg mux to {:?}", &output_path);

    // Set remaining ffmpeg args and run ffmpeg
    cmd.arg("-muxpreload")
        .arg("0")
        .arg("-muxdelay")
        .arg("0")
        .arg("-avoid_negative_ts")
        .arg("make_zero")
        .arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg("copy")
        .arg("-c:s")
        .arg("mov_text")
        .arg("-dn")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path.as_ref())
        .kill_on_drop(true);

    event!(Level::TRACE, "{:?}", cmd);
    let output = cmd.output().await?;
    event!(
        Level::TRACE,
        "ffmpeg stdout: {:#?}",
        String::from_utf8_lossy(&output.stdout)
    );
    event!(
        Level::TRACE,
        "ffmpeg stderr: {:#?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check ffmpeg exit status
    if !output.status.success() {
        return Err(anyhow::anyhow!("ffmpeg command failed"));
    }

    Ok(())
}

/// Pass stream names and languages to ffmpeg command
fn add_metadata(cmd: &mut process::Command, streams: &Vec<(&Stream, PathBuf)>) {
    // Closure to add stream metadata if available
    let mut add_lang = |stream: &Stream, t, lang, count| {
        // Language
        if let Some(l) = lang {
            if let Ok(l) = to_iso639_2(l) {
                cmd.arg(format!("-metadata:s:{}:{}", t, count))
                    .arg(format!("language={}", l));
            }
        }

        // Name
        if let Some(n) = stream.name() {
            cmd.arg(format!("-metadata:s:{}:{}", t, count))
                .arg(format!("title={}", n))
                .arg(format!("-metadata:s:{}:{}", t, count))
                .arg(format!("handler={}", n));
        }

        count + 1
    };

    // Set stream metadata
    let mut video_count = 0;
    let mut audio_count = 0;
    let mut subtitle_count = 0;
    for (stream, _) in streams {
        match stream {
            Stream::Main => {
                video_count += 1;
            }
            Stream::Video { lang: l, .. } => {
                video_count = add_lang(stream, "v", l.as_ref(), video_count);
            }
            Stream::Audio { lang: l, .. } => {
                audio_count = add_lang(stream, "a", l.as_ref(), audio_count);
            }
            Stream::Subtitle { lang: l, .. } => {
                subtitle_count = add_lang(stream, "s", l.as_ref(), subtitle_count);
            }
        }
    }
}

/// Convert rfc5646 language tag to iso639-3 format readable by ffmpeg
#[instrument(level = "trace")]
fn to_iso639_2(lang: impl AsRef<str> + Debug) -> Result<String> {
    // Parse language tag string
    let tag = LanguageTag::parse(lang.as_ref())?;
    let mut code = tag.primary_language().to_owned();

    // If tag is 2 letter iso639-1, convert to 3 letter iso639-3
    if code.len() == 2 {
        code = Language::from_639_1(&code)
            .ok_or_else(|| anyhow::anyhow!("Unknown language: {}", &code))?
            .to_639_3()
            .to_owned();
    }

    // Append region code if necessary
    if let Some(r) = tag.region() {
        code.push_str(&format!("-{}", r));
    }

    Ok(code)
}
