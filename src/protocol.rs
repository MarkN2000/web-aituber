use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct Submission {
    pub id: String,
    pub text: String,
    pub image: Option<InputImage>,
}

#[derive(Debug)]
pub struct InputImage {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Emotion {
    #[default]
    Neutral,
    Happy,
    Sad,
    Angry,
    Surprised,
}

impl Emotion {
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "neutral" => Some(Self::Neutral),
            "happy" => Some(Self::Happy),
            "sad" => Some(Self::Sad),
            "angry" => Some(Self::Angry),
            "surprised" => Some(Self::Surprised),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Happy => "happy",
            Self::Sad => "sad",
            Self::Angry => "angry",
            Self::Surprised => "surprised",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Generating,
    Speaking,
}

#[derive(Clone, Debug, Serialize)]
pub struct TurnState {
    pub turn_id: String,
    pub question: String,
    pub status: TurnStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Snapshot {
        current: Option<TurnState>,
    },
    State {
        turn: TurnState,
    },
    Segment {
        turn_id: String,
        sequence: u32,
        text: String,
        emotion: Emotion,
        motion: Option<String>,
        audio_url: String,
        duration_ms: u64,
        is_last: bool,
    },
    Complete {
        turn_id: String,
    },
    Cancelled {
        turn_id: String,
    },
    Error {
        turn_id: String,
        message: String,
    },
    Idle,
}

#[derive(Debug, Deserialize)]
pub struct AdminSkipRequest {
    pub turn_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_event_uses_expected_names() {
        let event = ServerEvent::State {
            turn: TurnState {
                turn_id: "turn-1".to_owned(),
                question: "質問".to_owned(),
                status: TurnStatus::Generating,
            },
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "state");
        assert_eq!(value["turn"]["status"], "generating");
    }
}
