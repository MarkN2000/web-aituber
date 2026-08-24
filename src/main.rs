use std::{
    collections::HashMap,
    io::ErrorKind,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
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

#[tokio::main]
async fn main() -> Result<()> {
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
    let audio_dir = create_audio_directory().await?;
    let (submissions, submission_receiver) = mpsc::channel(SUBMISSION_QUEUE_SIZE);
    let (events, _) = broadcast::channel(DISPLAY_EVENT_BUFFER_SIZE);
    let (shutdown, shutdown_receiver) = watch::channel(false);

    let state = AppState {
        config,
        http: reqwest::Client::new(),
        submissions,
        events,
        current: Arc::new(RwLock::new(None)),
        active: Arc::new(Mutex::new(None)),
        history: Arc::new(Mutex::new(ConversationHistory::default())),
        food_images: Arc::new(RwLock::new(HashMap::new())),
        audio_dir: Arc::new(audio_dir.clone()),
        assets_dir: Arc::new(PathBuf::from("assets")),
        vrm_model_lock: Arc::new(Mutex::new(())),
        background_image_lock: Arc::new(Mutex::new(())),
        screen_overlay_lock: Arc::new(Mutex::new(())),
        background_music_lock: Arc::new(Mutex::new(())),
        update_in_progress: Arc::new(AtomicBool::new(false)),
        shutdown,
        search_filler_rotation: Arc::new(SearchFillerRotation::default()),
    };

    tokio::spawn(pipeline::run(state.clone(), submission_receiver));

    tracing::info!(address = %bind, "サーバーを開始しました");

    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal(shutdown_receiver))
        .await
        .context("HTTPサーバーが終了しました")?;

    if let Err(error) = tokio::fs::remove_dir_all(&audio_dir).await {
        tracing::warn!(path = %audio_dir.display(), error = ?error, "一時音声ディレクトリを削除できませんでした");
    }
    Ok(())
}

async fn create_audio_directory() -> Result<PathBuf> {
    create_audio_directory_in(std::env::temp_dir().join("web-aituber")).await
}

async fn create_audio_directory_in(root: PathBuf) -> Result<PathBuf> {
    remove_previous_audio_sessions(&root).await;
    let directory = root.join(Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&directory)
        .await
        .with_context(|| {
            format!(
                "一時音声ディレクトリを作成できません: {}",
                directory.display()
            )
        })?;
    Ok(directory)
}

async fn remove_previous_audio_sessions(root: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(root).await
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(path = %root.display(), error = ?error, "過去の一時音声を削除できませんでした");
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
    async fn startup_removes_previous_audio_sessions() {
        let root = std::env::temp_dir().join(format!("web-aituber-test-{}", Uuid::new_v4()));
        let previous = root.join("previous");
        tokio::fs::create_dir_all(&previous).await.unwrap();
        tokio::fs::write(previous.join("audio.webm"), b"audio")
            .await
            .unwrap();

        let current = create_audio_directory_in(root.clone()).await.unwrap();

        assert!(!previous.exists());
        assert!(current.exists());
        assert_eq!(current.parent(), Some(root.as_path()));

        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
