#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, ErrorKind, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, watch};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use web_aituber::{
    config::ConfigStore,
    pipeline, routes,
    state::{AppState, ConversationHistory, SearchFillerRotation},
};

const SUBMISSION_QUEUE_SIZE: usize = 100;
const DISPLAY_EVENT_BUFFER_SIZE: usize = 128;
const UPDATE_LOG_ENV: &str = "WEB_AITUBER_UPDATE_LOG";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let message = format!("サーバーを起動できませんでした: {error:#}");
        append_update_error(&message);
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ConfigStore::load()?;
    let bind: SocketAddr = config
        .current()
        .bind
        .parse()
        .context("bindの形式が不正です")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("{bind}で待ち受けできません"))?;
    web_aituber::background_music::migrate_legacy(
        &config.current().ffmpeg_path,
        Path::new("assets"),
    )
    .await
    .context("BGMをM4Aへ移行できませんでした")?;
    let audio_session =
        AudioSession::create(&audio_root()).context("一時音声の保存先を初期化できません")?;
    let (submissions, submission_receiver) = mpsc::channel(SUBMISSION_QUEUE_SIZE);
    let (events, _) = broadcast::channel(DISPLAY_EVENT_BUFFER_SIZE);
    let (shutdown, shutdown_receiver) = watch::channel(false);

    let state = AppState {
        config,
        http: http_client()?,
        submissions,
        events,
        current: Arc::new(RwLock::new(None)),
        active: Arc::new(Mutex::new(None)),
        history: Arc::new(Mutex::new(ConversationHistory::default())),
        food_images: Arc::new(RwLock::new(HashMap::new())),
        audio_dir: Arc::new(audio_session.directory.clone()),
        assets_dir: Arc::new(PathBuf::from("assets")),
        motion_files_lock: Arc::new(Mutex::new(())),
        vrm_model_lock: Arc::new(Mutex::new(())),
        background_image_lock: Arc::new(Mutex::new(())),
        preparation_image_lock: Arc::new(Mutex::new(())),
        screen_overlay_lock: Arc::new(Mutex::new(())),
        background_music_lock: Arc::new(Mutex::new(())),
        update_in_progress: Arc::new(AtomicBool::new(false)),
        shutdown,
        search_filler_rotation: Arc::new(SearchFillerRotation::default()),
    };

    let worker = tokio::spawn(pipeline::run(state.clone(), submission_receiver));

    tracing::info!(address = %bind, "サーバーを開始しました");

    let result = axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal(shutdown_receiver))
        .await
        .context("HTTPサーバーが終了しました");
    worker.abort();
    let _ = worker.await;
    if let Err(error) = audio_session.remove() {
        tracing::warn!(error = ?error, "一時音声ディレクトリを削除できませんでした");
    }
    result
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(90))
        .timeout(Duration::from_secs(300))
        .build()
        .context("HTTPクライアントを初期化できませんでした")
}

fn append_update_error(message: &str) {
    let Some(path) = std::env::var_os(UPDATE_LOG_ENV) else {
        return;
    };
    append_update_error_to(Path::new(&path), message);
}

fn append_update_error_to(path: &Path, message: &str) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{message}");
    let _ = file.flush();
}

fn audio_root() -> PathBuf {
    #[cfg(unix)]
    let name = format!("web-aituber-audio-{}", unsafe { libc::geteuid() });
    #[cfg(not(unix))]
    let name = "web-aituber-audio";
    std::env::temp_dir().join(name)
}

struct AudioSession {
    directory: PathBuf,
    owner: File,
}

impl AudioSession {
    fn create(root: &Path) -> Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(root)
            .with_context(|| format!("一時音声の保存先を作成できません: {}", root.display()))?;
        #[cfg(unix)]
        {
            let metadata = fs::symlink_metadata(root)?;
            anyhow::ensure!(
                metadata.is_dir()
                    && metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o077 == 0,
                "一時音声の保存先には実行ユーザーが所有する専用ディレクトリが必要です: {}",
                root.display()
            );
        }
        // ponytail: 起動時の回収と所有ロックの作成を直列化する。回収が重くなれば削除対象を先に退避する。
        let startup = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("startup.lock"))?;
        startup.lock()?;
        for entry in fs::read_dir(root)?.flatten() {
            let path = entry.path();
            if entry
                .file_name()
                .to_str()
                .is_none_or(|name| Uuid::try_parse(name).is_err())
                || !entry.file_type().is_ok_and(|ty| ty.is_file())
            {
                continue;
            }
            if let Err(error) = Self::remove_if_inactive(&path)
                && error.kind() != ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), error = ?error, "過去の一時音声を削除できませんでした");
            }
        }

        let marker = root.join(Uuid::new_v4().to_string());
        let owner = File::create_new(&marker)?;
        owner.lock()?;
        let directory = marker.with_extension("audio");
        fs::create_dir(&directory).with_context(|| {
            format!(
                "一時音声ディレクトリを作成できません: {}",
                directory.display()
            )
        })?;
        Ok(Self { directory, owner })
    }

    fn remove_if_inactive(marker: &Path) -> io::Result<()> {
        let owner = OpenOptions::new().read(true).write(true).open(marker)?;
        match owner.try_lock() {
            Ok(()) => Self {
                directory: marker.with_extension("audio"),
                owner,
            }
            .remove(),
            Err(TryLockError::WouldBlock) => Ok(()),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    fn remove(self) -> io::Result<()> {
        if let Err(error) = fs::remove_dir_all(&self.directory)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error);
        }
        drop(self.owner);
        let marker = self.directory.with_extension("");
        match fs::remove_file(marker) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

#[cfg(windows)]
async fn shutdown_signal(shutdown: watch::Receiver<bool>) {
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(error = ?error, "終了シグナルを待機できませんでした");
            }
        }
        _ = wait_for_requested_shutdown(shutdown) => {}
    }
}

#[cfg(unix)]
async fn shutdown_signal(shutdown: watch::Receiver<bool>) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(error = ?error, "SIGTERMを待機できませんでした");
            return wait_for_requested_shutdown(shutdown).await;
        }
    };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(error = ?error, "終了シグナルを待機できませんでした");
            }
        }
        _ = terminate.recv() => {}
        _ = wait_for_requested_shutdown(shutdown) => {}
    }
}

async fn wait_for_requested_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow_and_update() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stalled_tts_request_expires() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let voice = web_aituber::config::TtsConfig {
            engine_url: format!("http://{}", listener.local_addr().unwrap()),
            speaker_id: 0,
        };
        let client = http_client().unwrap();
        let request = tokio::spawn(async move {
            web_aituber::tts::synthesize(&client, &voice, "テスト。").await
        });
        let socket = tokio::time::timeout(Duration::from_secs(5), async {
            let (socket, _) = listener.accept().await.unwrap();
            socket.readable().await.unwrap();
            socket
        })
        .await
        .unwrap();

        tokio::time::pause();
        let error = tokio::time::timeout(Duration::from_secs(95), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_timeout());
        drop(socket);
    }

    #[test]
    fn startup_reclaims_crashed_sessions_and_keeps_live_audio() {
        let root = std::env::temp_dir().join(format!("web-aituber-test-{}", Uuid::new_v4()));
        let live = AudioSession::create(&root).unwrap();
        #[cfg(unix)]
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o077, 0);
        let live_audio = live.directory.join("live.webm");
        fs::write(&live_audio, b"live").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::exit_with_an_audio_session", "--ignored"])
            .env("WEB_AITUBER_TEST_AUDIO_ROOT", &root)
            .status()
            .unwrap();
        assert!(status.success());

        let crashed = root.join(fs::read_to_string(root.join("child-session")).unwrap());
        assert!(crashed.join("child.webm").is_file());
        let next = AudioSession::create(&root).unwrap();
        assert_eq!(fs::read(live_audio).unwrap(), b"live");
        assert!(!crashed.exists());
        assert!(!crashed.with_extension("").exists());
        let next_dir = next.directory.clone();
        next.remove().unwrap();
        assert!(!next_dir.exists());
        assert!(!next_dir.with_extension("").exists());
        live.remove().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn audio_session_rejects_a_symlink_root() {
        let parent = std::env::temp_dir().join(format!("web-aituber-test-{}", Uuid::new_v4()));
        let other = parent.join("other");
        let link = parent.join("link");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&other)
            .unwrap();
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(AudioSession::create(&link).is_err());
        assert_eq!(fs::read_dir(other).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    #[ignore = "異常終了した音声セッションを作る別プロセス用"]
    fn exit_with_an_audio_session() {
        let Some(root) = std::env::var_os("WEB_AITUBER_TEST_AUDIO_ROOT") else {
            return;
        };
        let root = Path::new(&root);
        let session = AudioSession::create(root).unwrap();
        fs::write(session.directory.join("child.webm"), b"child").unwrap();
        fs::write(
            root.join("child-session"),
            session.directory.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        std::process::exit(0);
    }

    #[test]
    fn startup_error_is_appended_to_update_log() {
        let root = std::env::temp_dir().join(format!("web-aituber-log-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("update.log");

        append_update_error_to(&path, "起動エラー");

        assert_eq!(std::fs::read_to_string(path).unwrap(), "起動エラー\n");
        std::fs::remove_dir_all(root).unwrap();
    }
}
