use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::config::TtsConfig;

pub async fn synthesize(client: &Client, config: &TtsConfig, text: &str) -> Result<Vec<u8>> {
    let base_url = config.engine_url.trim_end_matches('/');
    let audio_query_url = format!("{base_url}/audio_query");
    let synthesis_url = format!("{base_url}/synthesis");

    let audio_query: Value = client
        .post(audio_query_url)
        .query(&[("text", text), ("speaker", &config.speaker_id.to_string())])
        .send()
        .await
        .context("TTS の audio_query に接続できません")?
        .error_for_status()
        .context("TTS の audio_query がエラーを返しました")?
        .json()
        .await
        .context("TTS の audio_query 応答を解釈できません")?;

    let wav = client
        .post(synthesis_url)
        .query(&[("speaker", config.speaker_id)])
        .json(&audio_query)
        .send()
        .await
        .context("TTS の synthesis に接続できません")?
        .error_for_status()
        .context("TTS の synthesis がエラーを返しました")?
        .bytes()
        .await
        .context("TTS の WAV 音声を読み取れません")?;
    Ok(wav.to_vec())
}
