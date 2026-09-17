//! 実FFmpegで音声形式と旧BGM移行を確認する。
//! 実行: cargo test --test audio-format -- --ignored
use std::{fs, io::Cursor, path::Path, process::Command};

use web_aituber::{audio, background_music};

fn assert_aac(path: &Path, channels: u64) {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let probe: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(probe["streams"][0]["codec_name"], "aac");
    assert_eq!(probe["streams"][0]["profile"], "LC");
    assert_eq!(probe["streams"][0]["channels"], channels);
    assert!(
        probe["format"]["format_name"]
            .as_str()
            .unwrap()
            .contains("m4a")
    );
    let duration = probe["format"]["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!((duration - 1.0).abs() < 0.1);
    let bytes = fs::read(path).unwrap();
    let moov = bytes.windows(4).position(|value| value == b"moov").unwrap();
    let mdat = bytes.windows(4).position(|value| value == b"mdat").unwrap();
    assert!(moov < mdat, "再生情報は音声データより前に配置する");
}

#[tokio::test]
#[ignore = "FFmpegとffprobeがPATHに必要"]
async fn aac変換と旧bgmの安全な移行() {
    let directory = std::env::temp_dir().join(format!("audio-format-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut buffer,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in 0..24000 {
        writer
            .write_sample(
                ((sample as f64 * 440.0 * std::f64::consts::TAU / 24000.0).sin() * 8000.0) as i16,
            )
            .unwrap();
    }
    writer.finalize().unwrap();
    let tts = directory.join("tts.m4a");
    assert_eq!(
        audio::transcode_to_aac("ffmpeg", buffer.get_ref(), &tts)
            .await
            .unwrap(),
        1000
    );
    assert_aac(&tts, 1);

    let wav = directory.join("input.wav");
    fs::write(&wav, buffer.get_ref()).unwrap();
    // 旧版が生成していた形式を移行用の入力として再現する。
    let legacy = directory.join("background-music.webm");
    assert!(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&wav)
            .args(["-c:a", "libopus"])
            .arg(&legacy)
            .status()
            .unwrap()
            .success()
    );
    let original = fs::read(&legacy).unwrap();
    assert!(
        background_music::migrate_legacy("存在しないffmpeg", &directory)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&legacy).unwrap(), original);
    let music = directory.join(background_music::FILE_NAME);
    assert!(!music.exists());
    background_music::migrate_legacy("ffmpeg", &directory)
        .await
        .unwrap();
    assert_aac(&music, 2);
    assert!(!legacy.exists());
    let backups: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "bak"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original);
    // 既存M4Aを優先し、旧ファイルは再変換せず退避する。
    fs::write(&legacy, &original).unwrap();
    let converted = fs::read(&music).unwrap();
    background_music::migrate_legacy("存在しないffmpeg", &directory)
        .await
        .unwrap();
    assert_eq!(fs::read(&music).unwrap(), converted);
    fs::remove_file(&music).unwrap();
    background_music::migrate_legacy("存在しないffmpeg", &directory)
        .await
        .unwrap();
    assert!(!music.exists(), "削除したBGMをバックアップから復活させない");
    fs::remove_dir_all(&directory).unwrap();
}
