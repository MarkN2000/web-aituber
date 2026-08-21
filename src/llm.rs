use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{FixedOffset, SecondsFormat, Utc};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::{
    config::LlmConfig,
    protocol::{ConversationTurn, SourceLink, Submission},
};

const MAX_SOURCES: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub struct LlmResponse {
    pub answer: String,
    pub sources: Vec<SourceLink>,
}

pub async fn generate(
    client: &Client,
    config: &LlmConfig,
    submission: &Submission,
    history: &[ConversationTurn],
    search_started: oneshot::Sender<()>,
) -> Result<LlmResponse> {
    let request = build_request(config, submission, history, &current_japan_time());
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

    read_stream(response, search_started).await
}

fn build_request(
    config: &LlmConfig,
    submission: &Submission,
    history: &[ConversationTurn],
    current_time: &str,
) -> Value {
    let is_food = submission.is_food();
    let instructions = if is_food {
        format!(
            "{}\n\n現在日時（日本時間）: {current_time}\n\n食事投稿への追加指示:\n{}",
            config.system_prompt.trim_end(),
            config.food_reaction_prompt.trim()
        )
    } else {
        format!(
            "{}\n\n現在日時（日本時間）: {current_time}\n検索した場合も、回答本文にはURLや出典一覧を含めないでください。出典は画面側で別に表示します。",
            config.system_prompt.trim_end()
        )
    };
    let mut input = Vec::with_capacity(history.len() * 2 + 1);

    if !is_food {
        for turn in history {
            input.push(input_message("user", &turn.question));
            input.push(input_message("assistant", &turn.answer));
        }
    }

    let mut content = vec![json!({ "type": "input_text", "text": submission.text })];
    if let Some(image) = submission.food_ai_image() {
        let data_url = format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(&image.data)
        );
        content.push(json!({
            "type": "input_image",
            "image_url": data_url,
            "detail": "low"
        }));
    }
    input.push(json!({ "role": "user", "content": content }));

    let mut request = json!({
        "model": config.model,
        "instructions": instructions,
        "input": input,
        "reasoning": { "effort": "low" },
        "text": { "verbosity": "low" },
        "store": false,
        "stream": true
    });
    if !is_food {
        request["tools"] = json!([{ "type": "web_search", "search_context_size": "low" }]);
        request["tool_choice"] = json!("auto");
        request["max_tool_calls"] = json!(1);
        request["include"] = json!(["web_search_call.action.sources"]);
    }
    request
}

fn input_message(role: &str, text: &str) -> Value {
    json!({ "role": role, "content": text })
}

async fn read_stream(
    response: reqwest::Response,
    search_started: oneshot::Sender<()>,
) -> Result<LlmResponse> {
    let mut chunks = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut fallback_text = String::new();
    let mut completed_response = None;
    let mut search_started = Some(search_started);

    while let Some(chunk) = chunks.next().await {
        buffer.extend_from_slice(&chunk.context("LLM API の応答を読み取れません")?);
        for data in take_sse_data(&mut buffer)? {
            if data == "[DONE]" {
                continue;
            }
            let event: Value =
                serde_json::from_str(&data).context("LLM API のストリーム応答を解釈できません")?;
            let event_type = event["type"].as_str().unwrap_or_default();

            if is_web_search_event(event_type, &event)
                && let Some(sender) = search_started.take()
            {
                let _ = sender.send(());
            }

            match event_type {
                "response.output_text.delta" => {
                    if let Some(delta) = event["delta"].as_str() {
                        fallback_text.push_str(delta);
                    }
                }
                "response.completed" => completed_response = event.get("response").cloned(),
                "response.failed" | "response.incomplete" | "error" => {
                    let message = event
                        .pointer("/response/error/message")
                        .or_else(|| event.pointer("/error/message"))
                        .or_else(|| event.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("詳細はありません");
                    bail!("LLM API の回答生成に失敗しました: {message}");
                }
                _ => {}
            }
        }
    }

    let mut result = completed_response
        .as_ref()
        .map(extract_response)
        .transpose()?
        .unwrap_or_else(|| LlmResponse {
            answer: fallback_text.trim().to_owned(),
            sources: Vec::new(),
        });
    if result.answer.is_empty() {
        result.answer = fallback_text.trim().to_owned();
    }
    if result.answer.is_empty() {
        bail!("LLM API の応答に回答テキストがありません");
    }
    Ok(result)
}

fn is_web_search_event(event_type: &str, event: &Value) -> bool {
    event_type.starts_with("response.web_search_call.")
        || (event_type == "response.output_item.added"
            && event.pointer("/item/type").and_then(Value::as_str) == Some("web_search_call"))
}

fn take_sse_data(buffer: &mut Vec<u8>) -> Result<Vec<String>> {
    let mut events = Vec::new();
    while let Some((boundary, delimiter_len)) = find_sse_boundary(buffer) {
        let frame = buffer.drain(..boundary).collect::<Vec<_>>();
        buffer.drain(..delimiter_len);
        let frame = String::from_utf8(frame).context("LLM API の応答がUTF-8ではありません")?;
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            events.push(data);
        }
    }
    Ok(events)
}

fn find_sse_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer
        .windows(2)
        .position(|bytes| bytes == b"\n\n")
        .map(|index| (index, 2));
    let crlf = buffer
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .map(|index| (index, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn extract_response(response: &Value) -> Result<LlmResponse> {
    let mut answer_parts = Vec::new();
    let mut sources = Vec::new();

    for item in response["output"].as_array().into_iter().flatten() {
        if item["type"].as_str() == Some("web_search_call") {
            for source in item
                .pointer("/action/sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                push_source(&mut sources, source);
            }
        }
        if item["type"].as_str() != Some("message") {
            continue;
        }
        for content in item["content"].as_array().into_iter().flatten() {
            if content["type"].as_str() != Some("output_text") {
                continue;
            }
            let text = content["text"].as_str().unwrap_or_default();
            let annotations = content["annotations"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let cleaned = strip_citations(text, annotations);
            if !cleaned.trim().is_empty() {
                answer_parts.push(cleaned.trim().to_owned());
            }

            for annotation in annotations {
                if sources.len() == MAX_SOURCES {
                    break;
                }
                if annotation["type"].as_str() != Some("url_citation") {
                    continue;
                }
                push_source(&mut sources, annotation);
            }
        }
    }

    Ok(LlmResponse {
        answer: answer_parts.join("\n"),
        sources,
    })
}

fn push_source(sources: &mut Vec<SourceLink>, source: &Value) {
    if sources.len() == MAX_SOURCES {
        return;
    }
    let Some(url) = source["url"].as_str().filter(|url| is_web_url(url)) else {
        return;
    };
    if sources.iter().any(|source| source.url == url) {
        return;
    }
    let title = source["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(url)
        .trim()
        .to_owned();
    sources.push(SourceLink {
        title,
        url: url.to_owned(),
    });
}

fn strip_citations(text: &str, annotations: &[Value]) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut removed = vec![false; chars.len()];

    for annotation in annotations {
        if annotation["type"].as_str() != Some("url_citation") {
            continue;
        }
        let Some(start) = annotation["start_index"]
            .as_u64()
            .map(|value| value as usize)
        else {
            continue;
        };
        let Some(end) = annotation["end_index"].as_u64().map(|value| value as usize) else {
            continue;
        };
        if start < end && end <= chars.len() {
            removed[start..end].fill(true);
        }
    }

    chars
        .into_iter()
        .zip(removed)
        .filter_map(|(character, removed)| (!removed).then_some(character))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn current_japan_time() -> String {
    let japan_offset = FixedOffset::east_opt(9 * 60 * 60).expect("日本時間のUTCオフセットは有効");
    Utc::now()
        .with_timezone(&japan_offset)
        .to_rfc3339_opts(SecondsFormat::Secs, false)
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
    use crate::{
        config::AppConfig,
        protocol::{InputImage, SubmissionKind},
    };

    #[test]
    fn question_request_contains_history_time_and_search_without_image() {
        let config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        let history = vec![ConversationTurn {
            turn_id: "turn-1".to_owned(),
            question: "前の質問".to_owned(),
            answer: "前の回答".to_owned(),
            sources: Vec::new(),
        }];
        let submission = Submission {
            id: "turn-2".to_owned(),
            kind: SubmissionKind::Question,
            text: "今回の質問".to_owned(),
        };

        let request = build_request(
            &config.llm,
            &submission,
            &history,
            "2026-08-07T03:00:00+09:00",
        );

        assert_eq!(request["model"], "gpt-5.6-luna");
        assert_eq!(request["tools"][0]["type"], "web_search");
        assert_eq!(request["tool_choice"], "auto");
        assert_eq!(request["max_tool_calls"], 1);
        assert_eq!(request["reasoning"]["effort"], "low");
        assert_eq!(request["text"]["verbosity"], "low");
        assert_eq!(request["include"][0], "web_search_call.action.sources");
        assert_eq!(request["stream"], true);
        assert_eq!(request["store"], false);
        assert!(
            request["instructions"]
                .as_str()
                .unwrap()
                .contains("現在日時（日本時間）: 2026-08-07T03:00:00+09:00")
        );
        assert_eq!(request["input"][0]["role"], "user");
        assert_eq!(request["input"][0]["content"], "前の質問");
        assert_eq!(request["input"][1]["role"], "assistant");
        assert_eq!(request["input"][1]["content"], "前の回答");
        assert_eq!(request["input"][2]["content"][0]["text"], "今回の質問");
        assert_eq!(request["input"][2]["content"].as_array().unwrap().len(), 1);
        assert!(
            !request["instructions"]
                .as_str()
                .unwrap()
                .contains("食事投稿への追加指示")
        );
    }

    #[test]
    fn food_request_uses_one_image_call_without_history_or_search() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.llm.food_reaction_prompt = "設定から変更した食事の感想指示".to_owned();
        let history = vec![ConversationTurn {
            turn_id: "turn-1".to_owned(),
            question: "前の質問".to_owned(),
            answer: "前の回答".to_owned(),
            sources: Vec::new(),
        }];
        let submission = Submission {
            id: "turn-2".to_owned(),
            kind: SubmissionKind::Food {
                vrm_image: InputImage {
                    mime_type: "image/webp".to_owned(),
                    data: vec![4, 5, 6],
                },
                ai_image: InputImage {
                    mime_type: "image/webp".to_owned(),
                    data: vec![1, 2, 3],
                },
            },
            text: "食べ物の絵を送りました".to_owned(),
        };

        let request = build_request(
            &config.llm,
            &submission,
            &history,
            "2026-08-22T12:00:00+09:00",
        );

        assert_eq!(request["input"].as_array().unwrap().len(), 1);
        assert_eq!(request["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(
            request["input"][0]["content"][1]["image_url"],
            "data:image/webp;base64,AQID"
        );
        assert!(request.get("tools").is_none());
        assert!(request.get("tool_choice").is_none());
        assert!(request.get("max_tool_calls").is_none());
        assert!(request.get("include").is_none());
        assert!(
            request["instructions"]
                .as_str()
                .unwrap()
                .contains("設定から変更した食事の感想指示")
        );
    }

    #[test]
    fn extracts_answer_and_unique_sources_without_inline_citation() {
        let response = json!({
            "output": [{
                "type": "web_search_call",
                "action": {
                    "sources": [{
                        "url": "https://example.com/consulted",
                        "title": "参照ページ"
                    }]
                }
            }, {
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "最新情報です。出典記号",
                    "annotations": [{
                        "type": "url_citation",
                        "start_index": 7,
                        "end_index": 11,
                        "url": "https://example.com/news",
                        "title": "ニュース"
                    }, {
                        "type": "url_citation",
                        "start_index": 7,
                        "end_index": 11,
                        "url": "https://example.com/news",
                        "title": "ニュース"
                    }]
                }]
            }]
        });

        let result = extract_response(&response).unwrap();
        assert_eq!(result.answer, "最新情報です。");
        assert_eq!(result.sources.len(), 2);
        assert_eq!(result.sources[0].title, "参照ページ");
        assert_eq!(result.sources[1].title, "ニュース");
    }

    #[test]
    fn decodes_sse_frames_split_across_chunks() {
        let mut buffer = b"event: message\r\ndata: {\"type\":\"response.output".to_vec();
        assert!(take_sse_data(&mut buffer).unwrap().is_empty());
        buffer.extend_from_slice(b"_text.delta\",\"delta\":\"ok\"}\r\n\r\n");
        let events = take_sse_data(&mut buffer).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("response.output_text.delta"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn current_time_uses_japan_offset() {
        assert!(current_japan_time().ends_with("+09:00"));
    }
}
