use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Speaker {
    pub id: u32,
    pub speaker_name: String,
    pub style_name: String,
}

#[derive(Deserialize)]
struct EngineSpeaker {
    name: String,
    styles: Vec<EngineStyle>,
}

#[derive(Deserialize)]
struct EngineStyle {
    id: u32,
    name: String,
}

/// VOICEVOX/AivisSpeech互換の話者一覧を、選択用の平坦な一覧へ変換する。
pub async fn fetch_speakers(client: &Client, engine_url: &str) -> Result<Vec<Speaker>> {
    let speakers_url = format!("{}/speakers", engine_url.trim_end_matches('/'));
    let speakers: Vec<EngineSpeaker> = client
        .get(speakers_url)
        .send()
        .await
        .context("TTS の speakers に接続できません")?
        .error_for_status()
        .context("TTS の speakers がエラーを返しました")?
        .json()
        .await
        .context("TTS の speakers 応答を解釈できません")?;
    Ok(speakers
        .into_iter()
        .flat_map(|speaker| {
            speaker.styles.into_iter().map(move |style| Speaker {
                id: style.id,
                speaker_name: speaker.name.clone(),
                style_name: style.name,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_response_is_flattened() {
        let source =
            r#"[{"name":"話者A","styles":[{"id":1,"name":"通常"},{"id":2,"name":"喜び"}]}]"#;
        let speakers: Vec<EngineSpeaker> = serde_json::from_str(source).unwrap();
        let flattened = speakers
            .into_iter()
            .flat_map(|speaker| {
                speaker.styles.into_iter().map(move |style| Speaker {
                    id: style.id,
                    speaker_name: speaker.name.clone(),
                    style_name: style.name,
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            flattened,
            vec![
                Speaker {
                    id: 1,
                    speaker_name: "話者A".to_owned(),
                    style_name: "通常".to_owned(),
                },
                Speaker {
                    id: 2,
                    speaker_name: "話者A".to_owned(),
                    style_name: "喜び".to_owned(),
                },
            ]
        );
    }
}
