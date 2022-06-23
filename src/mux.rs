use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use isolang::Language;
use log::{info, trace};
use oxilangtag::LanguageTag;
use tokio::{fs, process};

use crate::livestream::Stream;

/// Remux media files into a single mp4 file with ffmpeg
pub async fn remux(
    inputs: HashMap<Stream, impl AsRef<Path>>,
    output: impl AsRef<Path>,
) -> Result<()> {
    info!("Remuxing to mp4");

    let inputs: Vec<_> = inputs.into_iter().collect();

    // Call ffmpeg to remux video file
    let mut cmd = process::Command::new("ffmpeg");

    // Set ffmpeg input files
    for (_, path) in &inputs {
        cmd.arg("-i").arg(path.as_ref());
    }

    // Map all streams
    for i in 0..inputs.len() {
        cmd.arg("-map").arg(i.to_string());
    }

    // Add stream language if available
    let mut add_lang = |t, lang, count| {
        if let Some(l) = lang {
            if let Ok(l) = to_iso639_2(l) {
                cmd.arg(format!("-metadata:s:{}:{}", t, count))
                    .arg(format!("language={}", l));
            }
        }
        count + 1
    };

    // Set stream languages
    let mut video_count = 0;
    let mut audio_count = 0;
    let mut subtitle_count = 0;
    for (stream, _) in &inputs {
        match stream {
            Stream::Main => {
                video_count += 1;
            }
            Stream::Video { lang: l, .. } => {
                video_count = add_lang("v", l.as_ref(), video_count);
            }
            Stream::Audio { lang: l, .. } => {
                audio_count = add_lang("a", l.as_ref(), audio_count);
            }
            Stream::Subtitle { lang: l, .. } => {
                subtitle_count = add_lang("s", l.as_ref(), subtitle_count);
            }
        }
    }

    // Set remaining ffmpeg args and run ffmpeg
    cmd.arg("-c:v")
        .arg("copy")
        .arg("-c:a")
        .arg("copy")
        .arg("-c:s")
        .arg("mov_text")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output.as_ref().with_extension("mp4"));

    dbg!(&cmd);
    let exit_status = cmd.status().await?;

    // Check ffmpeg exit status
    if !exit_status.success() {
        return Err(anyhow::anyhow!("ffmpeg command failed"));
    }

    // Delete original files
    for (_, path) in &inputs {
        trace!("Removing {}", path.as_ref().to_string_lossy());
        fs::remove_file(path.as_ref()).await?;
    }

    Ok(())
}

/// Convert rfc5646 language tag to iso639-3 format readable by ffmpeg
fn to_iso639_2(lang: impl AsRef<str>) -> Result<String> {
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
