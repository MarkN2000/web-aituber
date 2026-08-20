use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{config::LlmConfig, protocol::Submission};

pub async fn generate(
    client: &Client,
    config: &LlmConfig,
    submission: &Submission,
) -> Result<String> {
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

    let request = json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": config.system_prompt },
            { "role": "user", "content": content }
        ]
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
