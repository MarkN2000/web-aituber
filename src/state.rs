use std::{path::PathBuf, sync::Arc};

use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    config::AppConfig,
    protocol::{ServerEvent, Submission, TurnState},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub http: reqwest::Client,
    pub submissions: mpsc::Sender<Submission>,
    pub events: broadcast::Sender<ServerEvent>,
    pub current: Arc<RwLock<Option<TurnState>>>,
    pub active: Arc<Mutex<Option<ActiveTurn>>>,
    pub audio_dir: Arc<PathBuf>,
}

pub struct ActiveTurn {
    pub turn_id: String,
    pub cancel: CancellationToken,
}
