use std::{future::Future, path::PathBuf, time::Duration};

use anyhow::{Result, anyhow};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{AppConfig, FoodMotionConfig, IdleSpeechConfig, IdleSpeechEntry},
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
    let mut config_changes = state.config.subscribe();
    loop {
        let config = config_changes.borrow_and_update().clone();
        let wait = async {
            if config.idle_speech.enabled && !config.character.preparation_mode {
                tokio::time::sleep(idle_delay(&config.idle_speech)).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let submission = tokio::select! {
            biased;
            submission = submissions.recv() => {
                let Some(submission) = submission else { break };
                Some(submission)
            },
            _ = config_changes.changed() => continue,
            _ = wait => {
                if state.events.receiver_count() == 0 { continue; }
                process_idle_speech(&state, &config, &mut submissions, &mut config_changes).await
            },
        };
        if let Some(submission) = submission
            && let Err(error) = process_submission(&state, submission).await
        {
            tracing::error!(error = ?error, "投稿の処理に失敗しました");
        }
    }
}

fn idle_delay(config: &IdleSpeechConfig) -> Duration {
    let span = u64::from(config.max_seconds - config.min_seconds) + 1;
    let random = uuid::Uuid::new_v4().as_u128() as u64;
    Duration::from_secs(u64::from(config.min_seconds) + random % span)
}

fn choose_idle_entry(config: &IdleSpeechConfig) -> &IdleSpeechEntry {
    let index = (uuid::Uuid::new_v4().as_u128() as u64) % config.entries.len() as u64;
    &config.entries[index as usize]
}

async fn process_idle_speech(
    state: &AppState,
    config: &AppConfig,
    submissions: &mut mpsc::Receiver<Submission>,
    config_changes: &mut tokio::sync::watch::Receiver<std::sync::Arc<AppConfig>>,
) -> Option<Submission> {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    *state.active.lock().await = Some(ActiveTurn {
        turn_id: turn_id.clone(),
        cancel: cancel.clone(),
    });
    publish_state(
        state,
        TurnState {
            turn_id: turn_id.clone(),
            question: String::new(),
            status: TurnStatus::IdleSpeaking,
        },
    )
    .await;
    let file_name = format!("{turn_id}-idle.m4a");
    let output_path = state.audio_dir.join(&file_name);
    let mut next_submission = None;
    let result = tokio::select! {
        biased;
        submission = submissions.recv() => {
            next_submission = submission;
            None
        },
        _ = config_changes.changed() => None,
        _ = cancel.cancelled() => None,
        result = async {
            let entry = choose_idle_entry(&config.idle_speech);
            let duration_ms = prepare_speech(state, config, &entry.text, &output_path).await?;
            present_idle_speech(state, config, entry, &turn_id, &file_name, duration_ms).await;
            Ok::<(), anyhow::Error>(())
        } => Some(result),
    };
    *state.active.lock().await = None;
    *state.current.write().await = None;
    send_event(
        state,
        match result {
            Some(Ok(())) => ServerEvent::Complete { turn_id },
            Some(Err(error)) => {
                tracing::warn!(error = ?error, "待機発話の生成に失敗しました");
                ServerEvent::Error {
                    turn_id,
                    message: "待機発話の音声を生成できませんでした。".to_owned(),
                }
            }
            None => ServerEvent::Cancelled { turn_id },
        },
    );
    schedule_audio_cleanup(vec![output_path]);
    send_event(state, ServerEvent::Idle);
    next_submission
}

async fn present_idle_speech(
    state: &AppState,
    config: &AppConfig,
    entry: &IdleSpeechEntry,
    turn_id: &str,
    file_name: &str,
    duration_ms: u64,
) {
    let motion = config
        .character
        .emotion_motions
        .get(entry.emotion.as_str())
        .filter(|motions| !motions.is_empty())
        .map(|_| entry.emotion);
    send_event(
        state,
        ServerEvent::Segment {
            turn_id: turn_id.to_owned(),
            sequence: 0,
            text: entry.text.clone(),
            emotion: entry.emotion,
            motion,
            audio_url: format!("/audio/{file_name}"),
            duration_ms,
            is_last: true,
            kind: SegmentKind::Idle,
            sources: Vec::new(),
        },
    );
    // ponytail: 終了は音声時間で推定する。端末ごとの遅延まで同期するなら再生完了通知へ拡張する。
    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
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
        let motion =
            config
                .character
                .configured_food_motion()
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
                let file_name = format!("{}-search.m4a", submission.id);
                let output_path = state.audio_dir.join(&file_name);
                let filler = state.next_search_filler(&config.llm.search_fillers);
                match cancellable(
                    cancel,
                    prepare_speech(state, config, filler, &output_path),
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
        let file_name = format!("{}-{index}.m4a", submission.id);
        let output_path = state.audio_dir.join(&file_name);
        audio_files.push(output_path.clone());
        let duration_ms = cancellable(
            cancel,
            prepare_speech(state, config, &segment.text, &output_path),
        )
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;

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
                .filter(|motions| !motions.is_empty())
                .map(|_| segment.emotion)
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

        cancellable(
            cancel,
            present_food(state, submission, food_motion, food_segments),
        )
        .await
        .map_err(|error| error.with_files(audio_files.clone()))?;
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

async fn present_food(
    state: &AppState,
    submission: &Submission,
    food_motion: &FoodMotionConfig,
    segments: Vec<(ServerEvent, u64)>,
) -> Result<()> {
    publish_state(
        state,
        TurnState {
            turn_id: submission.id.clone(),
            question: submission.text.clone(),
            status: TurnStatus::Eating,
        },
    )
    .await;
    let started_at = Instant::now();
    let motion_deadline = started_at + Duration::from_millis(food_motion.duration_ms);
    send_event(
        state,
        ServerEvent::FoodAction {
            turn_id: submission.id.clone(),
            image_url: format!("/food-images/{}", submission.id),
            consume_at_ms: food_motion.consume_at_ms,
            duration_ms: food_motion.duration_ms,
        },
    );
    tokio::time::sleep_until(started_at + Duration::from_millis(food_motion.speech_start_ms)).await;

    publish_state(
        state,
        TurnState {
            turn_id: submission.id.clone(),
            question: submission.text.clone(),
            status: TurnStatus::Speaking,
        },
    )
    .await;
    let mut playback_deadline = None;
    for (event, duration_ms) in segments {
        send_event(state, event);
        playback_deadline = append_playback_duration(playback_deadline, duration_ms);
    }
    // 感想が短くても食事モーションの終了まで次の投稿へ進まない。
    let due = playback_deadline.map_or(motion_deadline, |deadline| deadline.max(motion_deadline));
    tokio::time::sleep_until(due).await;
    Ok(())
}

async fn prepare_speech(
    state: &AppState,
    config: &AppConfig,
    text: &str,
    output_path: &std::path::Path,
) -> Result<u64> {
    let audio = tts::cached_speech(state, config, &config.tts, text, None).await?;
    tokio::fs::create_dir_all(&*state.audio_dir).await?;
    tokio::fs::write(output_path, &audio.bytes).await?;
    Ok(audio.duration_ms)
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

    fn test_state(config: AppConfig) -> AppState {
        let (submissions, _) = mpsc::channel(1);
        let (events, _) = broadcast::channel(16);
        AppState {
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
            motion_files_lock: Arc::new(Mutex::new(())),
            vrm_model_lock: Arc::new(Mutex::new(())),
            background_image_lock: Arc::new(Mutex::new(())),
            preparation_image_lock: Arc::new(Mutex::new(())),
            screen_overlay_lock: Arc::new(Mutex::new(())),
            background_music_lock: Arc::new(Mutex::new(())),
            update_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown: watch::channel(false).0,
            search_filler_rotation: Arc::new(SearchFillerRotation::default()),
        }
    }

    #[test]
    fn 待ち時間は設定範囲内で固定間隔にもできる() {
        let mut idle = crate::config::IdleSpeechConfig::default();
        for _ in 0..100 {
            assert!(
                (Duration::from_secs(30)..=Duration::from_secs(90)).contains(&idle_delay(&idle))
            );
        }
        idle.max_seconds = idle.min_seconds;
        assert_eq!(idle_delay(&idle), Duration::from_secs(30));
    }

    #[test]
    fn 待機発話の候補は毎回選び感情とセリフを同じ組で返す() {
        let mut config = IdleSpeechConfig::default();
        assert!(std::ptr::eq(choose_idle_entry(&config), &config.entries[0]));
        config.entries.push(IdleSpeechEntry {
            emotion: Emotion::Happy,
            text: "うれしい。".to_owned(),
        });
        config.entries.push(IdleSpeechEntry {
            emotion: Emotion::Sad,
            text: "静かだね。".to_owned(),
        });
        let mut seen = [false; 3];
        for _ in 0..256 {
            let selected = choose_idle_entry(&config);
            let index = config
                .entries
                .iter()
                .position(|entry| std::ptr::eq(entry, selected))
                .unwrap();
            seen[index] = true;
        }
        assert!(seen.into_iter().all(|selected| selected));
    }

    #[tokio::test(start_paused = true)]
    async fn 待機発話は指定感情と固定文を送り音声終了まで待つ() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.idle_speech.entries.push(IdleSpeechEntry {
            emotion: Emotion::Happy,
            text: "ひと休み。".to_owned(),
        });
        let state = test_state(config.clone());
        let mut events = state.events.subscribe();
        let running = state.clone();
        let task = tokio::spawn(async move {
            present_idle_speech(
                &running,
                &config,
                &config.idle_speech.entries[1],
                "idle-1",
                "idle-1.m4a",
                1500,
            )
            .await;
        });
        assert!(
            matches!(events.recv().await.unwrap(), ServerEvent::Segment {
            kind: SegmentKind::Idle, emotion: Emotion::Happy, motion: Some(Emotion::Happy),
            text, is_last: true, ..
        } if text == "ひと休み。")
        );
        tokio::time::advance(Duration::from_millis(1499)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        task.await.unwrap();
        assert!(state.history.lock().await.snapshot().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn 待機発話は失敗後も間隔を置き準備中と無効時は発動しない() {
        for (enabled, preparation, listening) in [
            (true, false, true),
            (false, false, true),
            (true, true, true),
            (true, false, false),
        ] {
            let mut config: AppConfig =
                serde_json::from_str(include_str!("../config.example.json")).unwrap();
            config.idle_speech.enabled = enabled;
            config.idle_speech.min_seconds = 2;
            config.idle_speech.max_seconds = 2;
            config.character.preparation_mode = preparation;
            // URL構築で即失敗させ、外部TTSへ接続しない。
            config.tts.engine_url = "invalid:".to_owned();
            let state = test_state(config);
            let mut events = listening.then(|| state.events.subscribe());
            let (_sender, receiver) = mpsc::channel(1);
            let task = tokio::spawn(run(state.clone(), receiver));
            tokio::task::yield_now().await;
            for _ in 0..2 {
                tokio::time::advance(Duration::from_millis(1999)).await;
                tokio::task::yield_now().await;
                assert!(state.current.read().await.is_none());
                if let Some(events) = events.as_mut() {
                    assert!(events.try_recv().is_err());
                }
                tokio::time::advance(Duration::from_millis(1)).await;
                tokio::task::yield_now().await;
                if enabled && !preparation && listening {
                    let events = events.as_mut().unwrap();
                    assert!(
                        matches!(events.recv().await.unwrap(), ServerEvent::State { turn } if matches!(turn.status, TurnStatus::IdleSpeaking))
                    );
                    assert!(matches!(
                        events.recv().await.unwrap(),
                        ServerEvent::Error { .. }
                    ));
                    assert!(matches!(events.recv().await.unwrap(), ServerEvent::Idle));
                } else if let Some(events) = events.as_mut() {
                    assert!(events.try_recv().is_err());
                }
            }
            assert!(state.history.lock().await.snapshot().is_empty());
            task.abort();
        }
    }

    #[tokio::test]
    async fn 待機音声の生成中でも投稿と管理中断と設定変更を優先する() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        for action in ["submission", "skip", "config"] {
            let path = std::env::temp_dir().join(format!("idle-{}.json", uuid::Uuid::new_v4()));
            let mut config: AppConfig =
                serde_json::from_str(include_str!("../config.example.json")).unwrap();
            // 接続を受け付けずに保持し、TTS生成が終わらない状況を再現する。
            config.tts.engine_url = format!("http://{}", listener.local_addr().unwrap());
            std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
            let mut state = test_state(config.clone());
            state.config = ConfigStore::new(&path, config.clone());
            let (sender, mut receiver) = mpsc::channel(1);
            let mut changes = state.config.subscribe();
            let mut events = state.events.subscribe();
            let running = state.clone();
            let task = tokio::spawn(async move {
                process_idle_speech(&running, &config, &mut receiver, &mut changes).await
            });
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerEvent::State { .. }
            ));
            match action {
                "submission" => sender
                    .send(Submission {
                        id: "next".to_owned(),
                        kind: SubmissionKind::Question,
                        text: "質問".to_owned(),
                    })
                    .await
                    .unwrap(),
                "skip" => state.active.lock().await.as_ref().unwrap().cancel.cancel(),
                _ => {
                    state
                        .config
                        .update_and_save(|config| config.idle_speech.enabled = false)
                        .unwrap();
                }
            }
            let next = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                next.map(|submission| submission.id),
                (action == "submission").then(|| "next".to_owned())
            );
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerEvent::Cancelled { .. }
            ));
            assert!(matches!(events.recv().await.unwrap(), ServerEvent::Idle));
            assert!(state.current.read().await.is_none());
            assert!(state.active.lock().await.is_none());
            assert!(state.history.lock().await.snapshot().is_empty());
            std::fs::remove_file(path).unwrap();
        }
    }

    fn start_food_presentation(
        consume_at_ms: u64,
        speech_start_ms: u64,
        duration_ms: u64,
        audio_durations: &[u64],
        cancel: CancellationToken,
    ) -> (
        tokio::task::JoinHandle<std::result::Result<(), CancellableError>>,
        broadcast::Receiver<ServerEvent>,
    ) {
        let config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        let state = test_state(config);
        let events = state.events.subscribe();
        let submission = Submission {
            id: "food-turn".to_owned(),
            kind: SubmissionKind::Food {
                vrm_image: crate::protocol::InputImage {
                    mime_type: "image/webp".to_owned(),
                    data: Vec::new(),
                },
                ai_image: crate::protocol::InputImage {
                    mime_type: "image/webp".to_owned(),
                    data: Vec::new(),
                },
            },
            text: "食べ物の絵を送りました".to_owned(),
        };
        let motion = FoodMotionConfig {
            url: "/assets/motions/eat2.vrma".to_owned(),
            consume_at_ms,
            speech_start_ms,
            duration_ms,
        };
        let segments = audio_durations
            .iter()
            .enumerate()
            .map(|(index, &duration_ms)| {
                (
                    ServerEvent::Segment {
                        turn_id: submission.id.clone(),
                        sequence: index as u32,
                        text: "おいしいです。".to_owned(),
                        emotion: Emotion::Happy,
                        motion: None,
                        audio_url: format!("/audio/food-{index}.m4a"),
                        duration_ms,
                        is_last: index + 1 == audio_durations.len(),
                        kind: SegmentKind::Answer,
                        sources: Vec::new(),
                    },
                    duration_ms,
                )
            })
            .collect();
        let task = tokio::spawn(async move {
            cancellable(
                &cancel,
                present_food(&state, &submission, &motion, segments),
            )
            .await
        });
        (task, events)
    }

    #[tokio::test(start_paused = true)]
    async fn 食事の発話開始は消去と独立しモーションと全音声の両方を待つ() {
        for (consume_at_ms, speech_start_ms, duration_ms, audio_durations, completion_ms) in [
            (500, 1000, 2000, vec![200], 2000),
            (500, 1000, 2000, vec![1000, 1500], 3500),
            (400, 0, 800, vec![400], 800),
            (500, 2500, 2000, vec![400], 2900),
        ] {
            let started_at = Instant::now();
            let (task, mut events) = start_food_presentation(
                consume_at_ms,
                speech_start_ms,
                duration_ms,
                &audio_durations,
                CancellationToken::new(),
            );
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerEvent::State { turn } if matches!(turn.status, TurnStatus::Eating)
            ));
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerEvent::FoodAction { consume_at_ms: consume, duration_ms: duration, .. }
                    if consume == consume_at_ms && duration == duration_ms
            ));
            if speech_start_ms > 0 {
                tokio::time::advance(Duration::from_millis(speech_start_ms - 1)).await;
                tokio::task::yield_now().await;
                assert!(events.try_recv().is_err());
                assert!(!task.is_finished());
                tokio::time::advance(Duration::from_millis(1)).await;
            }
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerEvent::State { turn } if matches!(turn.status, TurnStatus::Speaking)
            ));
            for (index, duration) in audio_durations.iter().enumerate() {
                assert!(matches!(
                    events.recv().await.unwrap(),
                    ServerEvent::Segment { sequence, duration_ms, motion: None, .. }
                        if sequence == index as u32 && duration_ms == *duration
                ));
            }
            assert_eq!(
                Instant::now() - started_at,
                Duration::from_millis(speech_start_ms)
            );

            tokio::time::advance(Duration::from_millis(completion_ms - speech_start_ms - 1)).await;
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            tokio::time::advance(Duration::from_millis(1)).await;
            assert!(matches!(task.await.unwrap(), Ok(())));
            assert_eq!(
                Instant::now() - started_at,
                Duration::from_millis(completion_ms)
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn 食事発話の前後どちらでも中断し後から発話イベントを送らない() {
        for cancel_at_ms in [200, 600] {
            let cancel = CancellationToken::new();
            let (task, mut events) =
                start_food_presentation(500, 500, 2000, &[1000], cancel.clone());
            events.recv().await.unwrap();
            events.recv().await.unwrap();
            tokio::time::advance(Duration::from_millis(cancel_at_ms)).await;
            tokio::task::yield_now().await;
            if cancel_at_ms > 500 {
                assert!(matches!(
                    events.recv().await.unwrap(),
                    ServerEvent::State { .. }
                ));
                assert!(matches!(
                    events.recv().await.unwrap(),
                    ServerEvent::Segment { .. }
                ));
            }
            cancel.cancel();
            assert!(matches!(
                task.await.unwrap(),
                Err(CancellableError::Cancelled)
            ));
            tokio::time::advance(Duration::from_secs(30)).await;
            assert!(events.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn 準備中は待機投稿を処理せずキャンセルする() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.character.preparation_mode = true;
        let state = test_state(config);
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
