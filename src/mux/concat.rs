use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::{fs, process};
use tracing::{event, Level};

use crate::livestream::{MediaFormat, Segment, Stream};

/// For each discontinuity, concatenate all streams
pub async fn concat_streams<P: AsRef<Path> + Debug>(
    downloaded_paths: &HashMap<Stream, Vec<(Segment, PathBuf)>>,
    output_dir: P,
) -> Result<HashMap<u64, Vec<(&Stream, PathBuf)>>> {
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
                                .entry(*cur_discon_seq.unwrap())
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
            discons.entry(*d).or_default().push((stream, file_path));
        }
    }

    Ok(discons)
}

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
async fn should_use_ffmpeg_concat(segment: &Segment) -> Result<bool> {
    #[allow(clippy::match_like_matches_macro)]
    let use_ffmpeg = match segment {
        Segment::Initialization { .. } => false,
        Segment::Sequence { format, .. } => match format {
            MediaFormat::Mp3 => true,
            _ => false,
        },
    };

    Ok(use_ffmpeg)
}
