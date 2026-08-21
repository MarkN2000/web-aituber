use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{FixedOffset, SecondsFormat, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    config::LlmConfig,
    protocol::{ConversationTurn, Submission},
};

pub async fn generate(
    client: &Client,
    config: &LlmConfig,
    submission: &Submission,
    history: &[ConversationTurn],
) -> Result<String> {
    let current_time = current_japan_time();
    let messages = build_messages(config, submission, history, &current_time);
    let request = json!({
        "model": config.model,
        "messages": messages
    });
    let response = client
        .post(&config.api_url)
        .bearer_auth(&config.api_key)
        .json(&request)
        .send()
        .await
        .context("LLM API への接続に失敗しました")?;
    let status = response.status();
    if !status.is_success() {
        let error = response
            .json::<ApiErrorResponse>()
            .await
            .ok()
            .and_then(|body| body.error)
            .and_then(|error| error.message)
            .unwrap_or_else(|| "詳細はありません".to_owned());
        bail!("LLM API がエラーを返しました ({status}): {error}");
    }
    let body: ChatCompletionResponse = response
        .json()
        .await
        .context("LLM API の応答を解釈できません")?;
    let answer = body
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .map(|content| content.trim().to_owned())
        .filter(|content| !content.is_empty());
    match answer {
        Some(answer) => Ok(answer),
        None => bail!("LLM API の応答に回答テキストがありません"),
    }
}

fn build_messages(
    config: &LlmConfig,
    submission: &Submission,
    history: &[ConversationTurn],
    current_time: &str,
) -> Vec<Value> {
    let mut messages = Vec::with_capacity(history.len() * 2 + 2);
    let system_prompt = format!(
        "{}\n\n現在日時（日本時間）: {current_time}",
        config.system_prompt.trim_end()
    );
    messages.push(json!({ "role": "system", "content": system_prompt }));

    for turn in history {
        messages.push(json!({ "role": "user", "content": turn.question }));
        messages.push(json!({ "role": "assistant", "content": turn.answer }));
    }

    let mut content = vec![json!({ "type": "text", "text": submission.text })];
    if let Some(image) = &submission.image {
        let data_url = format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(&image.data)
        );
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": data_url }
        }));
    }
    messages.push(json!({ "role": "user", "content": content }));
    messages
}

fn current_japan_time() -> String {
    let japan_offset = FixedOffset::east_opt(9 * 60 * 60).expect("日本時間のUTCオフセットは有効");
    Utc::now()
        .with_timezone(&japan_offset)
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorResponse {
    error: Option<ApiErrorDetail>,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, protocol::InputImage};

    #[test]
    fn conversation_history_is_added_before_current_submission() {
        let config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        let history = vec![ConversationTurn {
            turn_id: "turn-1".to_owned(),
            question: "前の質問".to_owned(),
            answer: "前の回答".to_owned(),
            has_image: false,
        }];
        let submission = Submission {
            id: "turn-2".to_owned(),
            text: "今回の質問".to_owned(),
            image: Some(InputImage {
                mime_type: "image/png".to_owned(),
                data: vec![1, 2, 3],
            }),
        };

        let messages = build_messages(
            &config.llm,
            &submission,
            &history,
            "2026-08-07T03:00:00+09:00",
        );

        assert_eq!(messages.len(), 4);
        assert!(
            messages[0]["content"]
                .as_str()
                .unwrap()
                .ends_with("現在日時（日本時間）: 2026-08-07T03:00:00+09:00")
        );
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "前の質問");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "前の回答");
        assert_eq!(messages[3]["content"][0]["text"], "今回の質問");
        assert!(
            messages[3]["content"][1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn current_time_uses_japan_offset() {
        assert!(current_japan_time().ends_with("+09:00"));
    }
}
