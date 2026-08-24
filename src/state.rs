use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use tokio::sync::{Mutex, RwLock, broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    config::ConfigStore,
    protocol::{ConversationTurn, InputImage, ServerEvent, Submission, TurnState},
};

#[derive(Clone)]
pub struct AppState {
    pub config: ConfigStore,
    pub http: reqwest::Client,
    pub submissions: mpsc::Sender<Submission>,
    pub events: broadcast::Sender<ServerEvent>,
    pub current: Arc<RwLock<Option<TurnState>>>,
    pub active: Arc<Mutex<Option<ActiveTurn>>>,
    pub history: Arc<Mutex<ConversationHistory>>,
    pub food_images: Arc<RwLock<HashMap<String, InputImage>>>,
    pub audio_dir: Arc<PathBuf>,
    pub assets_dir: Arc<PathBuf>,
    pub vrm_model_lock: Arc<Mutex<()>>,
    pub background_image_lock: Arc<Mutex<()>>,
    pub preparation_image_lock: Arc<Mutex<()>>,
    pub screen_overlay_lock: Arc<Mutex<()>>,
    pub background_music_lock: Arc<Mutex<()>>,
    pub update_in_progress: Arc<AtomicBool>,
    pub shutdown: watch::Sender<bool>,
    pub search_filler_rotation: Arc<SearchFillerRotation>,
}

#[derive(Default)]
pub struct SearchFillerRotation {
    next: AtomicUsize,
}

impl SearchFillerRotation {
    fn select<'a>(&self, fillers: &'a [String]) -> &'a str {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % fillers.len();
        &fillers[index]
    }
}

impl AppState {
    pub fn next_search_filler<'a>(&self, fillers: &'a [String]) -> &'a str {
        self.search_filler_rotation.select(fillers)
    }
}

pub struct ActiveTurn {
    pub turn_id: String,
    pub cancel: CancellationToken,
}

const CONVERSATION_HISTORY_LIMIT: usize = 10;

#[derive(Default)]
pub struct ConversationHistory {
    turns: VecDeque<ConversationTurn>,
}

impl ConversationHistory {
    pub fn snapshot(&self) -> Vec<ConversationTurn> {
        self.turns.iter().cloned().collect()
    }

    pub fn record(&mut self, turn: ConversationTurn) {
        if self.turns.len() == CONVERSATION_HISTORY_LIMIT {
            self.turns.pop_front();
        }
        self.turns.push_back(turn);
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_history_keeps_latest_ten_turns() {
        let mut history = ConversationHistory::default();
        for index in 0..11 {
            history.record(ConversationTurn {
                turn_id: format!("turn-{index}"),
                question: format!("質問{index}"),
                answer: format!("回答{index}"),
                sources: Vec::new(),
            });
        }

        let turns = history.snapshot();
        assert_eq!(turns.len(), 10);
        assert_eq!(turns.first().unwrap().question, "質問1");
        assert_eq!(turns.last().unwrap().answer, "回答10");
    }

    #[test]
    fn search_fillers_are_selected_in_rotation() {
        let rotation = SearchFillerRotation::default();
        let fillers = vec!["一つ目".to_owned(), "二つ目".to_owned()];

        assert_eq!(rotation.select(&fillers), "一つ目");
        assert_eq!(rotation.select(&fillers), "二つ目");
        assert_eq!(rotation.select(&fillers), "一つ目");
    }
}
