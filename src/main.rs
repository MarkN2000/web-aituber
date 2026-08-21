use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
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
    let audio_dir = create_audio_directory().await?;
    let (submissions, submission_receiver) = mpsc::channel(SUBMISSION_QUEUE_SIZE);
    let (events, _) = broadcast::channel(DISPLAY_EVENT_BUFFER_SIZE);

    let state = AppState {
        config,
        http: reqwest::Client::new(),
        submissions,
        events,
        current: Arc::new(RwLock::new(None)),
        active: Arc::new(Mutex::new(None)),
        history: Arc::new(Mutex::new(ConversationHistory::default())),
        audio_dir: Arc::new(audio_dir.clone()),
        search_filler_rotation: Arc::new(SearchFillerRotation::default()),
    };

    tokio::spawn(pipeline::run(state.clone(), submission_receiver));

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("{bind}で待ち受けできません"))?;
    tracing::info!(address = %bind, "サーバーを開始しました");

    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTPサーバーが終了しました")?;

    if let Err(error) = tokio::fs::remove_dir_all(&audio_dir).await {
        tracing::warn!(path = %audio_dir.display(), error = ?error, "一時音声ディレクトリを削除できませんでした");
    }
    Ok(())
}

async fn create_audio_directory() -> Result<PathBuf> {
    let directory = std::env::temp_dir()
        .join("web-aituber")
        .join(Uuid::new_v4().to_string());
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

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(error = ?error, "終了シグナルを待機できませんでした");
    }
}
