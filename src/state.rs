use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    config::AppConfig,
    protocol::{ConversationTurn, ServerEvent, Submission, TurnState},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub http: reqwest::Client,
    pub submissions: mpsc::Sender<Submission>,
    pub events: broadcast::Sender<ServerEvent>,
    pub current: Arc<RwLock<Option<TurnState>>>,
    pub active: Arc<Mutex<Option<ActiveTurn>>>,
    pub history: Arc<Mutex<ConversationHistory>>,
    pub audio_dir: Arc<PathBuf>,
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
    pub fn next_search_filler(&self) -> &str {
        self.search_filler_rotation
            .select(&self.config.llm.search_fillers)
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
                has_image: index == 10,
                sources: Vec::new(),
            });
        }

        let turns = history.snapshot();
        assert_eq!(turns.len(), 10);
        assert_eq!(turns.first().unwrap().question, "質問1");
        assert_eq!(turns.last().unwrap().answer, "回答10");
        assert!(turns.last().unwrap().has_image);
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
