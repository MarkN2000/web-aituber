use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct Submission {
    pub id: String,
    pub kind: SubmissionKind,
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum SubmissionKind {
    Question,
    Food {
        vrm_image: InputImage,
        ai_image: InputImage,
    },
}

impl Submission {
    pub fn is_food(&self) -> bool {
        matches!(&self.kind, SubmissionKind::Food { .. })
    }

    pub fn food_vrm_image(&self) -> Option<&InputImage> {
        match &self.kind {
            SubmissionKind::Food { vrm_image, .. } => Some(vrm_image),
            SubmissionKind::Question => None,
        }
    }

    pub fn food_ai_image(&self) -> Option<&InputImage> {
        match &self.kind {
            SubmissionKind::Food { ai_image, .. } => Some(ai_image),
            SubmissionKind::Question => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputImage {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConversationTurn {
    pub turn_id: String,
    pub question: String,
    pub answer: String,
    pub sources: Vec<SourceLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceLink {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    #[default]
    Answer,
    Filler,
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
    Eating,
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
        history: Vec<ConversationTurn>,
    },
    History {
        turns: Vec<ConversationTurn>,
    },
    State {
        turn: TurnState,
    },
    FoodAction {
        turn_id: String,
        image_url: String,
        consume_at_ms: u64,
        duration_ms: u64,
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
        kind: SegmentKind,
        sources: Vec<SourceLink>,
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

    #[test]
    fn filler_segment_is_distinguishable_from_an_answer() {
        let event = ServerEvent::Segment {
            turn_id: "turn-1".to_owned(),
            sequence: 0,
            text: String::new(),
            emotion: Emotion::Neutral,
            motion: None,
            audio_url: "/audio/filler.webm".to_owned(),
            duration_ms: 800,
            is_last: false,
            kind: SegmentKind::Filler,
            sources: Vec::new(),
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "segment");
        assert_eq!(value["kind"], "filler");
        assert_eq!(value["text"], "");
    }

    #[test]
    fn food_action_contains_presentation_timing() {
        let event = ServerEvent::FoodAction {
            turn_id: "turn-1".to_owned(),
            image_url: "/food-images/turn-1".to_owned(),
            consume_at_ms: 1_200,
            duration_ms: 1_600,
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "food_action");
        assert_eq!(value["image_url"], "/food-images/turn-1");
        assert_eq!(value["consume_at_ms"], 1_200);
        assert_eq!(value["duration_ms"], 1_600);
    }

    #[test]
    fn snapshot_contains_shared_history() {
        let event = ServerEvent::Snapshot {
            current: None,
            history: vec![ConversationTurn {
                turn_id: "turn-1".to_owned(),
                question: "質問".to_owned(),
                answer: "回答".to_owned(),
                sources: vec![SourceLink {
                    title: "出典".to_owned(),
                    url: "https://example.com".to_owned(),
                }],
            }],
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "snapshot");
        assert_eq!(value["history"][0]["question"], "質問");
        assert_eq!(value["history"][0]["answer"], "回答");
        assert_eq!(
            value["history"][0]["sources"][0]["url"],
            "https://example.com"
        );
    }
}
