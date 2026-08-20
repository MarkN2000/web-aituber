use std::{collections::VecDeque, path::PathBuf, sync::Arc};

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
    pub history: Arc<Mutex<ConversationHistory>>,
    pub audio_dir: Arc<PathBuf>,
}

pub struct ActiveTurn {
    pub turn_id: String,
    pub cancel: CancellationToken,
}

const CONVERSATION_HISTORY_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Default)]
pub struct ConversationHistory {
    turns: VecDeque<ConversationTurn>,
}

impl ConversationHistory {
    pub fn snapshot(&self) -> Vec<ConversationTurn> {
        self.turns.iter().cloned().collect()
    }

    pub fn record(&mut self, question: String, answer: String) {
        if self.turns.len() == CONVERSATION_HISTORY_LIMIT {
            self.turns.pop_front();
        }
        self.turns.push_back(ConversationTurn { question, answer });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_history_keeps_latest_ten_turns() {
        let mut history = ConversationHistory::default();
        for index in 0..11 {
            history.record(format!("質問{index}"), format!("回答{index}"));
        }

        let turns = history.snapshot();
        assert_eq!(turns.len(), 10);
        assert_eq!(turns.first().unwrap().question, "質問1");
        assert_eq!(turns.last().unwrap().answer, "回答10");
    }
}
