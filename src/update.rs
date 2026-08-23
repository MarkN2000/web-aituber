use std::{
    env,
    fs::File,
    io::{self},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const RELEASE_API_URL: &str = "https://api.github.com/repos/MarkN2000/web-aituber/releases/latest";
const USER_AGENT: &str = "web-aituber-updater";
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 4 * 1024;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const RELEASE_ASSET_NAME: Option<&str> = Some("web-aituber-windows-x64.zip");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const RELEASE_ASSET_NAME: Option<&str> = Some("web-aituber-linux-x64.tar.gz");
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
const RELEASE_ASSET_NAME: Option<&str> = None;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub self_update_supported: bool,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug)]
pub struct PreparedUpdate {
    pub version: String,
}

pub fn unsupported_reason() -> Option<String> {
    install_layout().err().map(|error| error.to_string())
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug)]
struct ReleaseInfo {
    version: Version,
    archive_url: Option<String>,
    checksum_url: Option<String>,
}

#[derive(Debug)]
struct InstallLayout {
    root: PathBuf,
    executable_name: String,
    updater: PathBuf,
}

pub async fn check(http: &reqwest::Client) -> Result<UpdateCheck> {
    let current = current_version()?;
    let release = fetch_latest_release(http).await?;
    let (self_update_supported, unsupported_reason) = match install_layout() {
        Ok(_) => (true, None),
        Err(error) => (false, Some(error.to_string())),
    };

    Ok(UpdateCheck {
        current_version: current.to_string(),
        latest_version: release.version.to_string(),
        update_available: release.version > current,
        self_update_supported,
        unsupported_reason,
    })
}

pub async fn prepare_and_launch(http: &reqwest::Client) -> Result<PreparedUpdate> {
    let layout = install_layout()?;
    let current = current_version()?;
    let release = fetch_latest_release(http).await?;
    if release.version <= current {
        bail!("利用できるアップデートはありません");
    }

    let asset_name = RELEASE_ASSET_NAME.context("このOS・CPUは自己更新に対応していません")?;
    let work_root = layout
        .root
        .join(format!(".web-aituber-update-{}", Uuid::new_v4().simple()));
    let archive_path = work_root.join(asset_name);
    let extracted_root = work_root.join("extracted");

    let result = async {
        tokio::fs::create_dir(&work_root)
            .await
            .context("更新用の作業フォルダを作成できません")?;
        let checksum_url = release
            .checksum_url
            .as_deref()
            .context("最新リリースにチェックサムがありません")?;
        let archive_url = release
            .archive_url
            .as_deref()
            .context("最新リリースにこの環境用の更新ファイルがありません")?;
        let checksum_text = download_text(http, checksum_url, MAX_CHECKSUM_BYTES).await?;
        download_file(http, archive_url, &archive_path, MAX_ARCHIVE_BYTES).await?;
        verify_checksum(&archive_path, &checksum_text, asset_name).await?;
        tokio::fs::create_dir(&extracted_root)
            .await
            .context("更新ファイルの展開先を作成できません")?;

        let archive_for_extract = archive_path.clone();
        let destination_for_extract = extracted_root.clone();
        tokio::task::spawn_blocking(move || {
            extract_archive(&archive_for_extract, &destination_for_extract)
        })
        .await
        .context("更新ファイルの展開処理に失敗しました")??;

        let package_root = extracted_root.join(package_root_name());
        validate_package(&package_root)?;
        launch_updater(&layout, &package_root, &work_root)?;
        Ok::<_, anyhow::Error>(())
    }
    .await;

    if let Err(error) = result {
        if let Err(cleanup_error) = tokio::fs::remove_dir_all(&work_root).await
            && cleanup_error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(error = ?cleanup_error, "失敗した更新用フォルダを削除できませんでした");
        }
        return Err(error);
    }

    Ok(PreparedUpdate {
        version: release.version.to_string(),
    })
}

fn current_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).context("現在のバージョンを解析できません")
}

async fn fetch_latest_release(http: &reqwest::Client) -> Result<ReleaseInfo> {
    let response = http
        .get(RELEASE_API_URL)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("GitHub Releasesへ接続できません")?
        .error_for_status()
        .context("最新リリースを取得できません")?;
    let release: GitHubRelease = response
        .json()
        .await
        .context("最新リリースの応答を読み取れません")?;
    release_info(release)
}

fn release_info(release: GitHubRelease) -> Result<ReleaseInfo> {
    let asset_name = RELEASE_ASSET_NAME.context("このOS・CPUは自己更新に対応していません")?;
    let version_text = release
        .tag_name
        .strip_prefix('v')
        .context("最新リリースのタグがvから始まっていません")?;
    let version = Version::parse(version_text).context("最新リリースのバージョンが不正です")?;
    let archive_url = asset_url(&release.assets, asset_name);
    let checksum_url = asset_url(&release.assets, &format!("{asset_name}.sha256"));
    Ok(ReleaseInfo {
        version,
        archive_url,
        checksum_url,
    })
}

fn asset_url(assets: &[GitHubAsset], name: &str) -> Option<String> {
    assets
        .iter()
        .find(|asset| asset.name == name)
        .map(|asset| asset.browser_download_url.clone())
}

fn install_layout() -> Result<InstallLayout> {
    if cfg!(target_os = "linux") && env::var_os("INVOCATION_ID").is_some() {
        bail!("systemdで管理されているため、管理画面からは更新できません");
    }
    RELEASE_ASSET_NAME.context("このOS・CPUは自己更新に対応していません")?;

    let executable = env::current_exe()
        .context("実行ファイルの場所を取得できません")?
        .canonicalize()
        .context("実行ファイルの場所を確認できません")?;
    let root = executable
        .parent()
        .context("実行ファイルのフォルダを取得できません")?
        .to_path_buf();
    let current_dir = env::current_dir()
        .context("作業フォルダを取得できません")?
        .canonicalize()
        .context("作業フォルダを確認できません")?;
    if current_dir != root {
        bail!("正式な配布フォルダからの起動ではないため、管理画面からは更新できません");
    }
    if !root.join("web").is_dir() || !root.join("config.example.json").is_file() {
        bail!("正式な配布フォルダではないため、管理画面からは更新できません");
    }

    let executable_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .context("実行ファイル名を取得できません")?
        .to_owned();
    let updater = root.join(updater_file_name());
    if !updater.is_file() {
        bail!("外部アップデーターがないため、管理画面からは更新できません");
    }

    Ok(InstallLayout {
        root,
        executable_name,
        updater,
    })
}

async fn download_text(http: &reqwest::Client, url: &str, limit: u64) -> Result<String> {
    let bytes = download_bytes(http, url, limit).await?;
    String::from_utf8(bytes).context("チェックサムがUTF-8ではありません")
}

async fn download_file(
    http: &reqwest::Client,
    url: &str,
    destination: &Path,
    limit: u64,
) -> Result<()> {
    let response = send_download_request(http, url).await?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        bail!("更新ファイルが大きすぎます");
    }
    let mut file = tokio::fs::File::create(destination)
        .await
        .context("更新ファイルを作成できません")?;
    let mut received = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("更新ファイルを受信できません")?;
        received = received
            .checked_add(chunk.len() as u64)
            .context("更新ファイルのサイズが不正です")?;
        if received > limit {
            bail!("更新ファイルが大きすぎます");
        }
        file.write_all(&chunk)
            .await
            .context("更新ファイルを保存できません")?;
    }
    file.flush().await.context("更新ファイルを保存できません")?;
    Ok(())
}

async fn download_bytes(http: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = send_download_request(http, url).await?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        bail!("ダウンロードした内容が大きすぎます");
    }
    let mut result = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("ダウンロードを完了できません")?;
        if result.len().saturating_add(chunk.len()) > limit as usize {
            bail!("ダウンロードした内容が大きすぎます");
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

async fn send_download_request(http: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    http.get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .context("更新ファイルをダウンロードできません")?
        .error_for_status()
        .context("更新ファイルをダウンロードできません")
}

async fn verify_checksum(path: &Path, checksum_text: &str, asset_name: &str) -> Result<()> {
    let expected = parse_checksum(checksum_text, asset_name)?;
    let mut file = tokio::fs::File::open(path)
        .await
        .context("更新ファイルを検証できません")?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .context("更新ファイルを検証できません")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        bail!("更新ファイルのチェックサムが一致しません");
    }
    Ok(())
}

fn parse_checksum(text: &str, asset_name: &str) -> Result<String> {
    let mut fields = text.split_whitespace();
    let checksum = fields.next().context("チェックサムが空です")?;
    let file_name = fields
        .next()
        .context("チェックサムにファイル名がありません")?
        .trim_start_matches('*');
    if fields.next().is_some() || file_name != asset_name {
        bail!("チェックサムのファイル名が一致しません");
    }
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("チェックサムの形式が不正です");
    }
    Ok(checksum.to_ascii_lowercase())
}

#[cfg(target_os = "windows")]
fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    extract_zip(archive_path, destination)
}

#[cfg(target_os = "linux")]
fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    extract_tar_gz(archive_path, destination)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn extract_archive(_archive_path: &Path, _destination: &Path) -> Result<()> {
    bail!("このOSのアーカイブ形式には対応していません")
}

#[cfg(target_os = "windows")]
fn extract_zip(archive_path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive_path).context("更新ファイルを開けません")?;
    let mut archive = zip::ZipArchive::new(file).context("ZIPファイルを読み取れません")?;
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).context("ZIP項目を読み取れません")?;
        let relative = entry
            .enclosed_name()
            .context("ZIPに不正なパスが含まれています")?;
        validate_relative_path(&relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("ZIPにシンボリックリンクが含まれています");
        }
        extracted = extracted
            .checked_add(entry.size())
            .context("展開後のサイズが不正です")?;
        if extracted > MAX_EXTRACTED_BYTES {
            bail!("展開後の更新ファイルが大きすぎます");
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output).context("更新フォルダを展開できません")?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).context("更新フォルダを展開できません")?;
        }
        let mut output_file = File::create(&output).context("更新ファイルを展開できません")?;
        io::copy(&mut entry, &mut output_file).context("更新ファイルを展開できません")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_tar_gz(archive_path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive_path).context("更新ファイルを開けません")?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut extracted = 0_u64;
    for entry in archive.entries().context("tar.gzを読み取れません")? {
        let mut entry = entry.context("tar項目を読み取れません")?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            bail!("tar.gzに通常ファイル以外の項目が含まれています");
        }
        extracted = extracted
            .checked_add(entry.header().size().context("tar項目のサイズが不正です")?)
            .context("展開後のサイズが不正です")?;
        if extracted > MAX_EXTRACTED_BYTES {
            bail!("展開後の更新ファイルが大きすぎます");
        }
        let relative = entry.path().context("tar項目のパスが不正です")?;
        validate_relative_path(&relative)?;
        let output = destination.join(relative.as_ref());
        entry
            .unpack(&output)
            .context("更新ファイルを展開できません")?;
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("アーカイブに不正なパスが含まれています");
    }
    Ok(())
}

fn validate_package(package_root: &Path) -> Result<()> {
    let required_files = [
        executable_file_name(),
        updater_file_name(),
        "config.example.json",
        "README.md",
    ];
    if required_files
        .iter()
        .any(|name| !package_root.join(name).is_file())
        || !package_root.join("web").is_dir()
        || !package_root.join("web/admin.html").is_file()
    {
        bail!("更新ファイルの構成が不正です");
    }
    Ok(())
}

fn launch_updater(layout: &InstallLayout, package_root: &Path, work_root: &Path) -> Result<()> {
    let temporary_updater = env::temp_dir().join(format!(
        "web-aituber-updater-{}{}",
        Uuid::new_v4().simple(),
        env::consts::EXE_SUFFIX
    ));
    std::fs::copy(&layout.updater, &temporary_updater)
        .context("外部アップデーターを準備できません")?;

    let mut command = Command::new(&temporary_updater);
    command
        .arg(std::process::id().to_string())
        .arg(&layout.root)
        .arg(package_root)
        .arg(work_root)
        .arg(&layout.executable_name)
        .current_dir(&layout.root)
        .stdin(Stdio::null());
    configure_hidden_process(&mut command);
    command
        .spawn()
        .context("外部アップデーターを起動できません")?;
    Ok(())
}

#[cfg(windows)]
fn configure_hidden_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_process(_command: &mut Command) {}

fn executable_file_name() -> &'static str {
    if cfg!(windows) {
        "web-aituber.exe"
    } else {
        "web-aituber"
    }
}

fn updater_file_name() -> &'static str {
    if cfg!(windows) {
        "web-aituber-updater.exe"
    } else {
        "web-aituber-updater"
    }
}

fn package_root_name() -> &'static str {
    if cfg!(windows) {
        "web-aituber-windows-x64"
    } else {
        "web-aituber-linux-x64"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_requires_expected_file_name() {
        let hash = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{hash}  package.zip"), "package.zip").unwrap(),
            hash
        );
        assert!(parse_checksum(&format!("{hash}  other.zip"), "package.zip").is_err());
        assert!(parse_checksum("short  package.zip", "package.zip").is_err());
    }

    #[test]
    fn archive_paths_must_stay_relative() {
        assert!(validate_relative_path(Path::new("package/web/admin.html")).is_ok());
        assert!(validate_relative_path(Path::new("../config.json")).is_err());
        assert!(validate_relative_path(Path::new("C:\\config.json")).is_err());
    }

    #[test]
    fn release_reads_platform_archive_and_checksum() {
        let asset_name = RELEASE_ASSET_NAME.unwrap();
        let release = GitHubRelease {
            tag_name: "v1.2.3".to_owned(),
            assets: vec![
                GitHubAsset {
                    name: asset_name.to_owned(),
                    browser_download_url: "https://example.com/archive".to_owned(),
                },
                GitHubAsset {
                    name: format!("{asset_name}.sha256"),
                    browser_download_url: "https://example.com/checksum".to_owned(),
                },
            ],
        };
        let info = release_info(release).unwrap();
        assert_eq!(info.version, Version::new(1, 2, 3));
        assert_eq!(
            info.archive_url.as_deref(),
            Some("https://example.com/archive")
        );
        assert_eq!(
            info.checksum_url.as_deref(),
            Some("https://example.com/checksum")
        );
    }
}
