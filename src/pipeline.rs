use std::{future::Future, path::PathBuf, time::Duration};

use anyhow::{Result, anyhow};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    audio,
    config::AppConfig,
    protocol::{
        ConversationTurn, Emotion, SegmentKind, ServerEvent, SourceLink, Submission, TurnState,
        TurnStatus,
    },
    state::{ActiveTurn, AppState},
    tts,
};

const AUDIO_RETENTION: Duration = Duration::from_secs(300);
const FOOD_IMAGE_RETENTION: Duration = Duration::from_secs(300);
const MAX_ANSWER_CHARACTERS: usize = 300;
const MAX_ANSWER_SENTENCES: usize = 4;

pub async fn run(state: AppState, mut submissions: mpsc::Receiver<Submission>) {
    while let Some(submission) = submissions.recv().await {
        if let Err(error) = process_submission(&state, submission).await {
            tracing::error!(error = ?error, "投稿の処理に失敗しました");
        }
    }
}

async fn process_submission(state: &AppState, submission: Submission) -> Result<()> {
    let cancel = CancellationToken::new();
    let config = {
        let mut active = state.active.lock().await;
        let config = state.config.current();
        if config.character.preparation_mode {
            send_event(
                state,
                ServerEvent::Cancelled {
                    turn_id: submission.id,
                },
            );
            send_event(state, ServerEvent::Idle);
            return Ok(());
        }
        *active = Some(ActiveTurn {
            turn_id: submission.id.clone(),
            cancel: cancel.clone(),
        });
        config
    };

    publish_state(
        state,
        TurnState {
            turn_id: submission.id.clone(),
            question: submission.text.clone(),
            status: TurnStatus::Generating,
        },
    )
    .await;

    let result = process_active_submission(state, &config, &submission, &cancel).await;

    {
        let mut active = state.active.lock().await;
        if active
            .as_ref()
            .is_some_and(|turn| turn.turn_id == submission.id)
        {
            *active = None;
        }
    }
    *state.current.write().await = None;

    match result {
        Ok(completed) => {
            let history = {
                let mut history = state.history.lock().await;
                history.record(ConversationTurn {
                    turn_id: submission.id.clone(),
                    question: submission.text.clone(),
                    answer: completed.answer,
                    sources: completed.sources,
                });
                history.snapshot()
            };
            send_event(state, ServerEvent::History { turns: history });
            send_event(
                state,
                ServerEvent::Complete {
                    turn_id: submission.id.clone(),
                },
            );
            schedule_audio_cleanup(completed.audio_files);
        }
        Err(ProcessError::Cancelled(audio_files)) => {
            send_event(
                state,
                ServerEvent::Cancelled {
                    turn_id: submission.id.clone(),
                },
            );
            schedule_audio_cleanup(audio_files);
        }
        Err(ProcessError::Failed { error, audio_files }) => {
            tracing::error!(turn_id = %submission.id, error = ?error, "回答処理に失敗しました");
            send_event(
                state,
                ServerEvent::Error {
                    turn_id: submission.id.clone(),
                    message: "回答または音声の生成に失敗しました".to_owned(),
                },
            );
            schedule_audio_cleanup(audio_files);
        }
    }

    send_event(state, ServerEvent::Idle);
    Ok(())
}

async fn process_active_submission(
    state: &AppState,
    config: &AppConfig,
    submission: &Submission,
    cancel: &CancellationToken,
) -> std::result::Result<CompletedSubmission, ProcessError> {
    let is_food = submission.is_food();
    let food_presentation = if let Some(image) = submission.food_vrm_image() {
        let motion = config
            .character
            .food_motion
            .as_ref()
            .ok_or_else(|| ProcessError::Failed {
                error: anyhow!("食事モーションが設定されていません"),
                audio_files: Vec::new(),
            })?;
        Some((image, motion))
    } else {
        None
    };
    let mut audio_files = Vec::new();
    let mut playback_deadline: Option<Instant> = None;
    let mut sequence_offset = 0_u32;
    let mut food_segments = Vec::new();

    let history = state.history.lock().await.snapshot();
    let (search_sender, mut search_started) = tokio::sync::oneshot::channel();
    let llm = crate::llm::generate(
        &state.http,
        &config.llm,
        submission,
        &history,
        search_sender,
    );
    tokio::pin!(llm);

    let generated = tokio::select! {
        _ = cancel.cancelled() => return Err(ProcessError::Cancelled(audio_files)),
        result = &mut llm => result.map_err(|error| ProcessError::Failed {
            error,
            audio_files: audio_files.clone(),
        })?,
        search = &mut search_started => {
            if search.is_ok() {
                let file_name = format!("{}-search.webm", submission.id);
                let output_path = state.audio_dir.join(&file_name);
                let filler = state.next_search_filler(&config.llm.search_fillers);
                match cancellable(
                    cancel,
                    prepare_search_filler(state, config, filler, &output_path),
                ).await {
                    Ok(duration_ms) => {
                        audio_files.push(output_path);
                        send_event(
                            state,
                            ServerEvent::Segment {
                                turn_id: submission.id.clone(),
                                sequence: 0,
                                text: String::new(),
                                emotion: Emotion::Neutral,
                                motion: None,
                                audio_url: format!("/audio/{file_name}"),
                                duration_ms,
                                is_last: false,
                                kind: SegmentKind::Filler,
                                sources: Vec::new(),
                            },
                        );
                        playback_deadline = Some(Instant::now() + Duration::from_millis(duration_ms));
                        sequence_offset = 1;
                    }
                    Err(CancellableError::Cancelled) => {
                        return Err(ProcessError::Cancelled(vec![output_path]));
                    }
                    Err(CancellableError::Failed(error)) => {
                        tracing::warn!(error = ?error, "検索中フィラーの生成に失敗しました");
                        if let Err(remove_error) = tokio::fs::remove_file(&output_path).await
                            && remove_error.kind() != std::io::ErrorKind::NotFound
                        {
                            tracing::warn!(error = ?remove_error, "未完成のフィラー音声を削除できませんでした");
                        }
                    }
                }
            }
            cancellable(cancel, llm.as_mut())
                .await
                .map_err(|error| error.with_files(audio_files.clone()))?
        }
    };

    let segments = limited_answer_segments(&generated.answer, generated.output_limit_reached);
    if segments.is_empty() {
        return Err(ProcessError::Failed {
            error: anyhow!("LLMの回答が空です"),
            audio_files,
        });
    }

    let mut motion_sent = false;

    for (index, segment) in segments.iter().enumerate() {
        let wav = cancellable(
            cancel,
            tts::synthesize(&state.http, &config.tts, &segment.text),
        )
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;

        let file_name = format!("{}-{index}.webm", submission.id);
        let output_path = state.audio_dir.join(&file_name);
        let duration_ms = cancellable(
            cancel,
            audio::transcode_to_opus(&config.ffmpeg_path, &wav, &output_path),
        )
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;
        audio_files.push(output_path);

        if index == 0 && !is_food {
            publish_state(
                state,
                TurnState {
                    turn_id: submission.id.clone(),
                    question: submission.text.clone(),
                    status: TurnStatus::Speaking,
                },
            )
            .await;
        }

        let motion = if is_food || motion_sent {
            None
        } else {
            config
                .character
                .emotion_motions
                .get(segment.emotion.as_str())
                .cloned()
                .inspect(|_| motion_sent = true)
        };

        let event = ServerEvent::Segment {
            turn_id: submission.id.clone(),
            sequence: sequence_offset + index as u32,
            text: segment.text.clone(),
            emotion: segment.emotion,
            motion,
            audio_url: format!("/audio/{file_name}"),
            duration_ms,
            is_last: index + 1 == segments.len(),
            kind: SegmentKind::Answer,
            sources: if index + 1 == segments.len() {
                generated.sources.clone()
            } else {
                Vec::new()
            },
        };

        if is_food {
            food_segments.push((event, duration_ms));
        } else {
            send_event(state, event);
            playback_deadline = append_playback_duration(playback_deadline, duration_ms);
        }
    }

    if let Some((image, food_motion)) = food_presentation {
        state
            .food_images
            .write()
            .await
            .insert(submission.id.clone(), image.clone());
        schedule_food_image_cleanup(state, submission.id.clone());

        publish_state(
            state,
            TurnState {
                turn_id: submission.id.clone(),
                question: submission.text.clone(),
                status: TurnStatus::Eating,
            },
        )
        .await;
        send_event(
            state,
            ServerEvent::FoodAction {
                turn_id: submission.id.clone(),
                image_url: format!("/food-images/{}", submission.id),
                consume_at_ms: food_motion.consume_at_ms,
                duration_ms: food_motion.duration_ms,
            },
        );
        cancellable(cancel, async {
            tokio::time::sleep(Duration::from_millis(food_motion.duration_ms)).await;
            Ok(())
        })
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;

        publish_state(
            state,
            TurnState {
                turn_id: submission.id.clone(),
                question: submission.text.clone(),
                status: TurnStatus::Speaking,
            },
        )
        .await;
        for (event, duration_ms) in food_segments {
            send_event(state, event);
            playback_deadline = append_playback_duration(playback_deadline, duration_ms);
        }
    }

    if let Some(due) = playback_deadline {
        cancellable(cancel, async {
            tokio::time::sleep_until(due).await;
            Ok(())
        })
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;
    }

    Ok(CompletedSubmission {
        audio_files,
        answer: display_answer(&segments),
        sources: generated.sources,
    })
}

async fn prepare_search_filler(
    state: &AppState,
    config: &AppConfig,
    filler: &str,
    output_path: &std::path::Path,
) -> Result<u64> {
    let wav = tts::synthesize(&state.http, &config.tts, filler).await?;
    audio::transcode_to_opus(&config.ffmpeg_path, &wav, output_path).await
}

async fn publish_state(state: &AppState, turn: TurnState) {
    *state.current.write().await = Some(turn.clone());
    send_event(state, ServerEvent::State { turn });
}

fn send_event(state: &AppState, event: ServerEvent) {
    let _ = state.events.send(event);
}

fn schedule_audio_cleanup(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(AUDIO_RETENTION).await;
        for path in paths {
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), error = ?error, "一時音声を削除できませんでした");
            }
        }
    });
}

fn schedule_food_image_cleanup(state: &AppState, turn_id: String) {
    let food_images = state.food_images.clone();
    tokio::spawn(async move {
        tokio::time::sleep(FOOD_IMAGE_RETENTION).await;
        food_images.write().await.remove(&turn_id);
    });
}

fn append_playback_duration(deadline: Option<Instant>, duration_ms: u64) -> Option<Instant> {
    let now = Instant::now();
    let playback_start = deadline.map_or(now, |deadline| deadline.max(now));
    Some(playback_start + Duration::from_millis(duration_ms))
}

async fn cancellable<T, F>(
    cancel: &CancellationToken,
    future: F,
) -> std::result::Result<T, CancellableError>
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        _ = cancel.cancelled() => Err(CancellableError::Cancelled),
        result = future => result.map_err(CancellableError::Failed),
    }
}

enum CancellableError {
    Cancelled,
    Failed(anyhow::Error),
}

impl CancellableError {
    fn with_files(self, audio_files: Vec<PathBuf>) -> ProcessError {
        match self {
            Self::Cancelled => ProcessError::Cancelled(audio_files),
            Self::Failed(error) => ProcessError::Failed { error, audio_files },
        }
    }
}

enum ProcessError {
    Cancelled(Vec<PathBuf>),
    Failed {
        error: anyhow::Error,
        audio_files: Vec<PathBuf>,
    },
}

struct CompletedSubmission {
    audio_files: Vec<PathBuf>,
    answer: String,
    sources: Vec<SourceLink>,
}

#[derive(Debug, PartialEq, Eq)]
struct AnswerSegment {
    text: String,
    emotion: Emotion,
}

fn display_answer(segments: &[AnswerSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

fn limited_answer_segments(answer: &str, require_complete_last: bool) -> Vec<AnswerSegment> {
    let mut segments = split_answer(answer);
    if require_complete_last
        && segments
            .last()
            .is_some_and(|segment| !segment.text.chars().last().is_some_and(is_sentence_end))
    {
        segments.pop();
    }

    let mut characters = 0;
    segments
        .into_iter()
        .take(MAX_ANSWER_SENTENCES)
        .take_while(|segment| {
            let next_characters = characters + segment.text.chars().count();
            if next_characters > MAX_ANSWER_CHARACTERS {
                return false;
            }
            characters = next_characters;
            true
        })
        .collect()
}

fn split_answer(answer: &str) -> Vec<AnswerSegment> {
    let mut raw_segments = Vec::new();
    let mut current = String::new();

    for character in answer.chars() {
        if character == '\r' {
            continue;
        }
        if character == '\n' {
            if !current.chars().last().is_some_and(char::is_whitespace) {
                current.push(' ');
            }
            continue;
        }
        current.push(character);
        if is_sentence_end(character) {
            if !current.trim().is_empty() {
                raw_segments.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if !current.trim().is_empty() {
        raw_segments.push(current);
    }

    raw_segments
        .into_iter()
        .filter_map(|raw| parse_segment(&raw))
        .collect()
}

fn is_sentence_end(character: char) -> bool {
    matches!(character, '。' | '！' | '？' | '!' | '?')
}

fn parse_segment(raw: &str) -> Option<AnswerSegment> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (emotion, text) = if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let tag = &rest[..end];
            (
                Emotion::from_tag(tag).unwrap_or_default(),
                rest[end + 1..].trim(),
            )
        } else {
            (Emotion::Neutral, trimmed)
        }
    } else {
        (Emotion::Neutral, trimmed)
    };

    (!text.is_empty()).then(|| AnswerSegment {
        text: text.to_owned(),
        emotion,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use tokio::sync::{Mutex, RwLock, broadcast, mpsc, watch};

    use super::*;
    use crate::{
        config::{AppConfig, ConfigStore},
        protocol::{ServerEvent, SubmissionKind},
        state::{ConversationHistory, SearchFillerRotation},
    };

    #[test]
    fn 文と感情タグを分割する() {
        let result = split_answer("[happy]こんにちは！\n[sad]今日は雨です。タグなしです");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].text, "こんにちは！");
        assert_eq!(result[0].emotion, Emotion::Happy);
        assert_eq!(result[1].text, "今日は雨です。");
        assert_eq!(result[1].emotion, Emotion::Sad);
        assert_eq!(result[2].emotion, Emotion::Neutral);
    }

    #[test]
    fn 不正なタグを読み上げない() {
        let result = split_answer("[joy]こんにちは。");
        assert_eq!(result[0].text, "こんにちは。");
        assert_eq!(result[0].emotion, Emotion::Neutral);
    }

    #[test]
    fn 履歴用回答から感情タグを除去する() {
        let segments = split_answer("[happy]こんにちは！[sad]また明日。");
        assert_eq!(display_answer(&segments), "こんにちは！また明日。");
    }

    #[test]
    fn 回答は感情タグを除いた本文を最大300文字かつ4文に制限する() {
        let first = format!("[happy]{}。", "あ".repeat(299));
        let answer = format!("{first}[sad]追加です。さらに追加です。まだ追加です。最後です。");

        let segments = limited_answer_segments(&answer, false);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text.chars().count(), MAX_ANSWER_CHARACTERS);

        let segments = limited_answer_segments("一文目。二文目。三文目。四文目。五文目。", false);
        assert_eq!(segments.len(), MAX_ANSWER_SENTENCES);
        assert_eq!(
            display_answer(&segments),
            "一文目。二文目。三文目。四文目。"
        );
    }

    #[test]
    fn 先頭の一文だけで300文字を超える回答は使用しない() {
        let answer = format!("{}。", "あ".repeat(300));

        assert!(limited_answer_segments(&answer, false).is_empty());
    }

    #[test]
    fn 出力上限に達した回答は文末まで完成した文だけを使用する() {
        let answer = "[neutral]完成した文です。[happy]途中の文";

        let limited = limited_answer_segments(answer, true);
        assert_eq!(display_answer(&limited), "完成した文です。");

        let completed = limited_answer_segments(answer, false);
        assert_eq!(display_answer(&completed), "完成した文です。途中の文");
    }

    #[test]
    fn 改行は文数に含めない() {
        let segments = limited_answer_segments("改行を\n含む一文です。", false);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "改行を 含む一文です。");
    }

    #[tokio::test]
    async fn 準備中は待機投稿を処理せずキャンセルする() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.character.preparation_mode = true;
        let (submissions, _) = mpsc::channel(1);
        let (events, _) = broadcast::channel(4);
        let state = AppState {
            config: ConfigStore::new("config.example.json", config),
            http: reqwest::Client::new(),
            submissions,
            events,
            current: Arc::new(RwLock::new(None)),
            active: Arc::new(Mutex::new(None)),
            history: Arc::new(Mutex::new(ConversationHistory::default())),
            food_images: Arc::new(RwLock::new(HashMap::new())),
            audio_dir: Arc::new(PathBuf::from("target/test-audio")),
            assets_dir: Arc::new(PathBuf::from("target/test-assets")),
            vrm_model_lock: Arc::new(Mutex::new(())),
            background_image_lock: Arc::new(Mutex::new(())),
            preparation_image_lock: Arc::new(Mutex::new(())),
            screen_overlay_lock: Arc::new(Mutex::new(())),
            background_music_lock: Arc::new(Mutex::new(())),
            update_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown: watch::channel(false).0,
            search_filler_rotation: Arc::new(SearchFillerRotation::default()),
        };
        let mut events = state.events.subscribe();

        process_submission(
            &state,
            Submission {
                id: "queued-turn".to_owned(),
                kind: SubmissionKind::Question,
                text: "処理しない質問".to_owned(),
            },
        )
        .await
        .unwrap();

        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::Cancelled { turn_id } if turn_id == "queued-turn"
        ));
        assert!(matches!(events.recv().await.unwrap(), ServerEvent::Idle));
        assert!(state.active.lock().await.is_none());
        assert!(state.current.read().await.is_none());
    }
}
