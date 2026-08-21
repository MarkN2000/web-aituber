use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail};
use tokio::process::Command;
use uuid::Uuid;

pub const FILE_NAME: &str = "background-music.webm";
pub const MAX_SOURCE_BYTES: usize = 100 * 1024 * 1024;

/// リクエストのキャンセルや途中エラーでも変換用ファイルを残さない。
pub struct TemporaryFiles {
    input: PathBuf,
    output: PathBuf,
}

impl TemporaryFiles {
    pub fn new(assets_dir: &Path, extension: &str) -> Self {
        let id = Uuid::new_v4();
        Self {
            input: assets_dir.join(format!(".{FILE_NAME}.{id}.input.{extension}")),
            output: assets_dir.join(format!(".{FILE_NAME}.{id}.output.tmp")),
        }
    }

    pub fn input(&self) -> &Path {
        &self.input
    }

    pub fn output(&self) -> &Path {
        &self.output
    }
}

impl Drop for TemporaryFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.input);
        let _ = fs::remove_file(&self.output);
    }
}

pub fn accepted_extension(file_name: &str) -> Option<&'static str> {
    match Path::new(file_name)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3") => Some("mp3"),
        Some("ogg") => Some("ogg"),
        Some("wav") => Some("wav"),
        _ => None,
    }
}

pub async fn convert(ffmpeg_path: &str, input: &Path, output: &Path) -> Result<()> {
    let status = Command::new(ffmpeg_path)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(input)
        .args([
            "-map", "0:a:0", "-vn", "-sn", "-dn", "-ar", "48000", "-ac", "2", "-c:a", "libopus",
            "-b:a", "128k", "-f", "webm", "-n",
        ])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .with_context(|| format!("FFmpegを起動できません: {ffmpeg_path}"))?;

    if !status.success() {
        bail!("FFmpegのBGM変換に失敗しました: {status}");
    }

    let metadata = tokio::fs::metadata(output)
        .await
        .context("変換後のBGMを確認できません")?;
    if metadata.len() == 0 {
        bail!("変換後のBGMが空です");
    }

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(output)
        .await
        .context("変換後のBGMを開けません")?;
    file.sync_all()
        .await
        .context("変換後のBGMを同期できません")?;
    Ok(())
}

pub fn install_atomically(temporary: &Path, destination: &Path) -> Result<()> {
    replace_or_create_file(temporary, destination)
}

#[cfg(not(windows))]
fn replace_or_create_file(temporary: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary, destination)
        .with_context(|| format!("BGMを原子的に置き換えられません: {}", destination.display()))
}

#[cfg(windows)]
fn replace_or_create_file(temporary: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        return fs::rename(temporary, destination)
            .with_context(|| format!("BGMを原子的に作成できません: {}", destination.display()));
    }

    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    let destination_wide = wide(destination.as_os_str());
    let replacement_wide = wide(temporary.as_os_str());
    let result = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            replacement_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        bail!("BGMを原子的に置き換えられません: {}", destination.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert_eq!(accepted_extension("music.mp3"), Some("mp3"));
        assert_eq!(accepted_extension("music.OGG"), Some("ogg"));
        assert_eq!(accepted_extension("music.WaV"), Some("wav"));
        assert_eq!(accepted_extension("music.webm"), None);
        assert_eq!(accepted_extension("music"), None);
    }

    #[test]
    fn temporary_files_are_removed_on_drop() {
        let directory = std::env::temp_dir().join(format!("web-aituber-bgm-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let files = TemporaryFiles::new(&directory, "wav");
        fs::write(files.input(), b"input").unwrap();
        fs::write(files.output(), b"output").unwrap();
        let input = files.input().to_owned();
        let output = files.output().to_owned();
        drop(files);
        assert!(!input.exists());
        assert!(!output.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
