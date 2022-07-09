use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use isolang::Language;
use oxilangtag::LanguageTag;
use tokio::io::AsyncWriteExt;
use tokio::{fs, process};
use tracing::{event, instrument, Level};

use crate::livestream::{MediaFormat, Segment, Stream};

/// Remux media files into a single mp4 file with ffmpeg
#[instrument(level = "trace")]
pub async fn remux<P: AsRef<Path> + Debug>(
    downloaded_paths: HashMap<Stream, Vec<(Segment, PathBuf)>>,
    output_dir: P,
) -> Result<()> {
    // Map discon seq -> Vec<(stream, concatenated path)>
    let mut discons: HashMap<_, Vec<_>> = HashMap::new();

    // Loop through all streams and discontinuity sequences and concatenate them
    for (stream, segments) in downloaded_paths.iter() {
        let mut segments_to_process = Vec::new();
        let mut cur_discon_seq = None;

        for (segment, path) in segments {
            match segment {
                Segment::Initialization { .. } => {
                    panic!("Expected Segment::Sequence, got Segment::Initialization")
                }
                Segment::Sequence { discon_seq: d, .. } => {
                    if cur_discon_seq.is_none() {
                        cur_discon_seq = Some(d);
                    }

                    if cur_discon_seq.map(|x| x == d).unwrap() {
                        // Add current segment to be processed
                        segments_to_process.push((segment, path.as_path()));
                    } else {
                        // If discontinuity changed, concat all previous discontinuity segments
                        if !segments_to_process.is_empty() {
                            let file_path = gen_concat_path(
                                stream,
                                segments_to_process[0].0,
                                &output_dir,
                                *cur_discon_seq.unwrap(),
                            )?;
                            concat_segments(segments_to_process.as_slice(), &file_path).await?;
                            discons
                                .entry(cur_discon_seq.unwrap())
                                .or_default()
                                .push((stream, file_path));
                        }

                        // Reset segments to process, push current segment, and update current
                        // discontinuity sequence
                        segments_to_process.clear();
                        segments_to_process.push((segment, path.as_path()));
                        cur_discon_seq = Some(d);
                    }
                }
            }
        }

        // Concat last discontinuity
        if !segments_to_process.is_empty() {
            let d = cur_discon_seq.unwrap();
            let file_path = gen_concat_path(stream, segments_to_process[0].0, &output_dir, *d)?;
            concat_segments(segments_to_process.as_slice(), &file_path).await?;
            discons.entry(d).or_default().push((stream, file_path));
        }
    }

    for (discon_seq, concatted_streams) in &discons {
        // Call ffmpeg to remux video file
        let mut cmd = process::Command::new("ffmpeg");
        cmd.arg("-y").arg("-copyts");

        // Set ffmpeg input files
        for (_, path) in concatted_streams {
            cmd.arg("-i").arg(path);
        }

        // Map all streams
        for i in 0..concatted_streams.len() {
            cmd.arg("-map").arg(i.to_string());
        }

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
        for (stream, _) in concatted_streams {
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

        // Generate output name
        const FILE_NAME: &str = "video";
        let output_path = if discons.len() == 1 {
            output_dir.as_ref().join(FILE_NAME)
        } else {
            FILE_NAME
                .to_string()
                .push_str(&format!("_{:010}", discon_seq));
            output_dir.as_ref().parent().unwrap().join(FILE_NAME)
        }
        .with_extension("mp4");

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
            .arg(output_path)
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
    }

    // Delete original files
    for concatted_streams in discons.values() {
        for (_, path) in concatted_streams {
            event!(Level::TRACE, "Removing {}", path.to_string_lossy());
            fs::remove_file(path).await?;
        }
    }

    Ok(())
}

#[instrument(level = "trace")]
fn gen_concat_path(
    stream: &Stream,
    segment: &Segment,
    output_dir: impl AsRef<Path> + Debug,
    d: u64,
) -> Result<PathBuf> {
    let ext = match segment {
        Segment::Initialization { .. } => {
            return Err(anyhow::anyhow!(
                "gen_concat_path got initialization expected sequence"
            ))
        }
        Segment::Sequence { format, .. } => format.extension(),
    };
    let file_name = format!("{}_{:010}.{}", stream, d, ext);
    let file_path = output_dir.as_ref().join(file_name);
    Ok(file_path)
}

#[instrument(level = "trace")]
async fn concat_segments<P: AsRef<Path> + Debug>(
    inputs: &[(&Segment, P)],
    output: P,
) -> Result<()> {
    if should_use_ffmpeg_concat(inputs[0].0).await? {
        ffmpeg_concat(inputs.iter().map(|(_, p)| p), &output).await
    } else {
        file_concat(inputs.iter().map(|(_, p)| p), &output).await
    }
}

#[instrument(level = "trace", skip(input_paths))]
async fn file_concat<P: AsRef<Path> + Debug>(
    input_paths: impl IntoIterator<Item = P>,
    output: P,
) -> Result<()> {
    event!(
        Level::INFO,
        "File concat to temporary file {:?}",
        output.as_ref()
    );

    let mut file = fs::File::create(output.as_ref()).await?;
    for path in input_paths {
        file.write_all(&fs::read(path.as_ref()).await?).await?;
    }
    Ok(())
}

#[instrument(level = "trace", skip(input_paths))]
async fn ffmpeg_concat<P: AsRef<Path> + Debug>(
    input_paths: impl IntoIterator<Item = P>,
    output: P,
) -> Result<()> {
    event!(
        Level::INFO,
        "ffmpeg concat demux to temporary file {:?}",
        output.as_ref()
    );

    // Create concat text file
    let file = tempfile::NamedTempFile::new()?;
    let cwd = env::current_dir()?;
    for path in input_paths {
        let absolute_path = if path.as_ref().is_absolute() {
            Cow::from(path.as_ref())
        } else {
            Cow::Owned(cwd.join(path))
        };
        writeln!(
            file.as_file(),
            "file '{}'",
            absolute_path.as_ref().to_str().unwrap()
        )?;
    }

    // Call ffmpeg to concat segments
    let mut cmd = process::Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(file.path())
        .arg("-c")
        .arg("copy")
        .arg("-fflags")
        .arg("+genpts")
        .arg(output.as_ref())
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

/// Decide whether to use file or ffmpeg concat demuxer
#[instrument(level = "trace")]
async fn should_use_ffmpeg_concat(segment: &Segment) -> Result<bool> {
    let use_ffmpeg = match segment {
        Segment::Initialization { .. } => false,
        Segment::Sequence { format, .. } => match format {
            MediaFormat::Mp3 => true,
            _ => false,
        },
    };

    Ok(use_ffmpeg)
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
