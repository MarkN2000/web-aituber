use std::{io::Cursor, path::Path, process::Stdio};

use anyhow::{Context, Result, bail};
use tokio::{io::AsyncWriteExt, process::Command};

pub fn wav_duration_ms(wav: &[u8]) -> Result<u64> {
    let reader = hound::WavReader::new(Cursor::new(wav)).context("WAV を読み取れません")?;
    let sample_rate = u64::from(reader.spec().sample_rate);
    if sample_rate == 0 {
        bail!("WAV のサンプルレートが不正です");
    }
    let samples = u64::from(reader.duration());
    Ok(samples.saturating_mul(1_000).div_ceil(sample_rate))
}

pub async fn transcode_to_opus(ffmpeg_path: &str, wav: &[u8], destination: &Path) -> Result<u64> {
    let duration_ms = wav_duration_ms(wav)?;
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("音声保存先を作成できません: {}", parent.display()))?;
    }

    let mut child = Command::new(ffmpeg_path)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-vn",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-f",
            "webm",
            "-y",
        ])
        .arg(destination)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("FFmpeg を起動できません: {ffmpeg_path}"))?;
    let mut stdin = child
        .stdin
        .take()
        .context("FFmpeg の標準入力を開けません")?;
    stdin
        .write_all(wav)
        .await
        .context("FFmpeg に WAV を渡せません")?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .context("FFmpeg の実行を待機できません")?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!("FFmpeg の音声変換に失敗しました: {detail}");
    }
    Ok(duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wav_duration() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 1_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
            for _ in 0..1_500 {
                writer.write_sample(0_i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        assert_eq!(wav_duration_ms(buffer.get_ref()).unwrap(), 1_500);
    }
}
