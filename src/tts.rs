use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::config::TtsConfig;

pub async fn synthesize(client: &Client, config: &TtsConfig, text: &str) -> Result<Vec<u8>> {
    let audio_query = create_audio_query(client, config, text).await?;
    synthesize_audio_query(client, config, &audio_query).await
}

async fn create_audio_query(client: &Client, config: &TtsConfig, text: &str) -> Result<Value> {
    let base_url = config.engine_url.trim_end_matches('/');
    let audio_query_url = format!("{base_url}/audio_query");
    client
        .post(audio_query_url)
        .query(&[("text", text), ("speaker", &config.speaker_id.to_string())])
        .send()
        .await
        .context("TTS の audio_query に接続できません")?
        .error_for_status()
        .context("TTS の audio_query がエラーを返しました")?
        .json()
        .await
        .context("TTS の audio_query 応答を解釈できません")
}

async fn synthesize_audio_query(
    client: &Client,
    config: &TtsConfig,
    audio_query: &Value,
) -> Result<Vec<u8>> {
    let base_url = config.engine_url.trim_end_matches('/');
    let synthesis_url = format!("{base_url}/synthesis");
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

#[derive(Debug)]
pub enum UserDictPreviewError {
    InvalidInput,
    Engine(anyhow::Error),
}

/// 入力中の読みとアクセント位置で、辞書を変更せずに試聴用WAVを生成する。
pub async fn synthesize_user_dict_preview(
    client: &Client,
    config: &TtsConfig,
    pronunciation: &str,
    accent_type: u32,
) -> std::result::Result<Vec<u8>, UserDictPreviewError> {
    let mut audio_query = create_audio_query(client, config, pronunciation)
        .await
        .map_err(UserDictPreviewError::Engine)?;
    apply_user_dict_preview_accent(&mut audio_query, pronunciation, accent_type)?;
    synthesize_audio_query(client, config, &audio_query)
        .await
        .map_err(UserDictPreviewError::Engine)
}

fn apply_user_dict_preview_accent(
    audio_query: &mut Value,
    pronunciation: &str,
    accent_type: u32,
) -> std::result::Result<(), UserDictPreviewError> {
    let accent_phrases = audio_query
        .get_mut("accent_phrases")
        .and_then(Value::as_array_mut)
        .ok_or(UserDictPreviewError::InvalidInput)?;
    if accent_phrases.len() != 1 {
        return Err(UserDictPreviewError::InvalidInput);
    }
    let accent_phrase = &mut accent_phrases[0];
    let mora_count = accent_phrase
        .get("moras")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or(UserDictPreviewError::InvalidInput)?;
    if mora_count == 0 {
        return Err(UserDictPreviewError::InvalidInput);
    }
    let generated_pronunciation = accent_phrase["moras"]
        .as_array()
        .and_then(|moras| {
            moras
                .iter()
                .map(|mora| mora.get("text")?.as_str())
                .collect::<Option<String>>()
        })
        .ok_or(UserDictPreviewError::InvalidInput)?;
    if generated_pronunciation != pronunciation {
        return Err(UserDictPreviewError::InvalidInput);
    }
    let accent = if accent_type == 0 {
        mora_count
    } else {
        usize::try_from(accent_type).map_err(|_| UserDictPreviewError::InvalidInput)?
    };
    if !(1..=mora_count).contains(&accent) {
        return Err(UserDictPreviewError::InvalidInput);
    }
    accent_phrase["accent"] = Value::from(accent);
    Ok(())
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UserDictWordType {
    ProperNoun,
    CommonNoun,
    Verb,
    Adjective,
    Suffix,
}

impl UserDictWordType {
    fn as_engine_value(self) -> &'static str {
        match self {
            Self::ProperNoun => "PROPER_NOUN",
            Self::CommonNoun => "COMMON_NOUN",
            Self::Verb => "VERB",
            Self::Adjective => "ADJECTIVE",
            Self::Suffix => "SUFFIX",
        }
    }

    fn from_context_id(context_id: u64) -> Option<Self> {
        match context_id {
            1348 => Some(Self::ProperNoun),
            1345 => Some(Self::CommonNoun),
            642 => Some(Self::Verb),
            20 => Some(Self::Adjective),
            1358 => Some(Self::Suffix),
            _ => None,
        }
    }

    fn from_engine_value(value: &str) -> Option<Self> {
        match value {
            "PROPER_NOUN" => Some(Self::ProperNoun),
            "COMMON_NOUN" => Some(Self::CommonNoun),
            "VERB" => Some(Self::Verb),
            "ADJECTIVE" => Some(Self::Adjective),
            "SUFFIX" => Some(Self::Suffix),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct UserDictWordInput {
    pub surface: String,
    pub pronunciation: String,
    pub accent_type: u32,
    pub word_type: UserDictWordType,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UserDictWord {
    pub uuid: String,
    #[serde(flatten)]
    pub input: UserDictWordInput,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UserDict {
    pub words: Vec<UserDictWord>,
    pub has_excluded_words: bool,
}

/// VOICEVOX/AivisSpeechのユーザー辞書から、両方で編集できる単一語だけを取得する。
pub async fn fetch_user_dict(client: &Client, engine_url: &str) -> Result<UserDict> {
    let user_dict_url = format!("{}/user_dict", engine_url.trim_end_matches('/'));
    let response: Value = client
        .get(user_dict_url)
        .query(&[("enable_compound_accent", true)])
        .send()
        .await
        .context("TTS の user_dict に接続できません")?
        .error_for_status()
        .context("TTS の user_dict がエラーを返しました")?
        .json()
        .await
        .context("TTS の user_dict 応答を解釈できません")?;
    parse_user_dict(response)
}

pub async fn add_user_dict_word(
    client: &Client,
    engine_url: &str,
    word: &UserDictWordInput,
) -> Result<()> {
    let url = format!("{}/user_dict_word", engine_url.trim_end_matches('/'));
    client
        .post(url)
        .query(&user_dict_word_query(word))
        .send()
        .await
        .context("TTS の user_dict_word に接続できません")?
        .error_for_status()
        .context("TTS の user_dict_word がエラーを返しました")?;
    Ok(())
}

pub async fn update_user_dict_word(
    client: &Client,
    engine_url: &str,
    word_uuid: Uuid,
    word: &UserDictWordInput,
) -> Result<()> {
    let url = format!(
        "{}/user_dict_word/{word_uuid}",
        engine_url.trim_end_matches('/')
    );
    client
        .put(url)
        .query(&user_dict_word_query(word))
        .send()
        .await
        .context("TTS の user_dict_word に接続できません")?
        .error_for_status()
        .context("TTS の user_dict_word がエラーを返しました")?;
    Ok(())
}

pub async fn delete_user_dict_word(
    client: &Client,
    engine_url: &str,
    word_uuid: Uuid,
) -> Result<()> {
    let url = format!(
        "{}/user_dict_word/{word_uuid}",
        engine_url.trim_end_matches('/')
    );
    client
        .delete(url)
        .send()
        .await
        .context("TTS の user_dict_word に接続できません")?
        .error_for_status()
        .context("TTS の user_dict_word がエラーを返しました")?;
    Ok(())
}

pub fn is_user_dict_input_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::status)
            == Some(reqwest::StatusCode::UNPROCESSABLE_ENTITY)
    })
}

fn user_dict_word_query(word: &UserDictWordInput) -> Vec<(&'static str, String)> {
    vec![
        ("surface", word.surface.clone()),
        ("pronunciation", word.pronunciation.clone()),
        ("accent_type", word.accent_type.to_string()),
        ("word_type", word.word_type.as_engine_value().to_owned()),
        ("priority", word.priority.to_string()),
    ]
}

fn parse_user_dict(response: Value) -> Result<UserDict> {
    let entries = response
        .as_object()
        .ok_or_else(|| anyhow!("TTS の user_dict 応答がオブジェクトではありません"))?;
    let mut words = entries
        .iter()
        .filter_map(|(uuid, value)| parse_user_dict_word(uuid, value))
        .collect::<Vec<_>>();
    words.sort_by(|left, right| {
        left.input
            .surface
            .cmp(&right.input.surface)
            .then_with(|| left.uuid.cmp(&right.uuid))
    });
    Ok(UserDict {
        has_excluded_words: words.len() != entries.len(),
        words,
    })
}

fn parse_user_dict_word(uuid: &str, value: &Value) -> Option<UserDictWord> {
    let object = value.as_object()?;
    let surface = single_string(object.get("surface")?)?;
    let pronunciation = single_string(object.get("pronunciation")?)?;
    let accent_type = u32::try_from(single_u64(object.get("accent_type")?)?).ok()?;
    let priority = u8::try_from(object.get("priority")?.as_u64()?).ok()?;
    let word_type = match object.get("word_type") {
        Some(Value::String(value)) => UserDictWordType::from_engine_value(value)?,
        Some(_) => return None,
        None => UserDictWordType::from_context_id(object.get("context_id")?.as_u64()?)?,
    };
    Some(UserDictWord {
        uuid: uuid.to_owned(),
        input: UserDictWordInput {
            surface,
            pronunciation,
            accent_type,
            word_type,
            priority,
        },
    })
}

fn single_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(values) if values.len() == 1 => values[0].as_str().map(ToOwned::to_owned),
        _ => None,
    }
}

fn single_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(value) => value.as_u64(),
        Value::Array(values) if values.len() == 1 => values[0].as_u64(),
        _ => None,
    }
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

    #[test]
    fn user_dict_response_keeps_only_common_single_words() {
        let response = serde_json::json!({
            "00000000-0000-0000-0000-000000000002": {
                "surface": ["Aivis"],
                "pronunciation": ["アイビス"],
                "accent_type": [1],
                "priority": 6,
                "context_id": 1348
            },
            "00000000-0000-0000-0000-000000000001": {
                "surface": "AITuber",
                "pronunciation": "エーアイチューバー",
                "accent_type": 0,
                "priority": 5,
                "context_id": 1345
            },
            "00000000-0000-0000-0000-000000000003": {
                "surface": ["新田", "真剣佑"],
                "pronunciation": ["アラタ", "マッケンユウ"],
                "accent_type": [1, 3],
                "priority": 5,
                "context_id": 1348
            },
            "00000000-0000-0000-0000-000000000004": {
                "surface": "東京",
                "pronunciation": "トーキョー",
                "accent_type": 0,
                "priority": 5,
                "context_id": 9999
            }
        });

        let dictionary = parse_user_dict(response).unwrap();
        let words = dictionary.words;

        assert_eq!(words.len(), 2);
        assert!(dictionary.has_excluded_words);
        assert_eq!(words[0].input.surface, "AITuber");
        assert_eq!(words[0].input.word_type, UserDictWordType::CommonNoun);
        assert_eq!(words[1].input.surface, "Aivis");
        assert_eq!(words[1].input.word_type, UserDictWordType::ProperNoun);
    }

    #[test]
    fn aivis_specific_word_type_is_excluded_even_if_context_id_is_common() {
        let response = serde_json::json!({
            "00000000-0000-0000-0000-000000000001": {
                "surface": "東京",
                "pronunciation": ["トーキョー"],
                "accent_type": [0],
                "priority": 5,
                "context_id": 1348,
                "word_type": "LOCATION_NAME"
            }
        });

        let dictionary = parse_user_dict(response).unwrap();

        assert!(dictionary.words.is_empty());
        assert!(dictionary.has_excluded_words);
    }

    #[test]
    fn preview_accent_converts_flat_accent_and_preserves_engine_fields() {
        let mut query = serde_json::json!({
            "accent_phrases": [{
                "moras": [{ "text": "キャ" }, { "text": "ラ" }],
                "accent": 1,
                "is_interrogative": false
            }],
            "kana": "キャラ'",
            "tempoDynamicsScale": 1.25
        });
        let mut expected = query.clone();
        expected["accent_phrases"][0]["accent"] = Value::from(2);

        apply_user_dict_preview_accent(&mut query, "キャラ", 0).unwrap();

        assert_eq!(query, expected);
    }

    #[test]
    fn preview_accent_accepts_nonzero_position() {
        let mut query = serde_json::json!({
            "accent_phrases": [{
                "moras": [{ "text": "テ" }, { "text": "ス" }, { "text": "ト" }],
                "accent": 1
            }]
        });

        apply_user_dict_preview_accent(&mut query, "テスト", 2).unwrap();

        assert_eq!(query["accent_phrases"][0]["accent"], 2);
    }

    #[test]
    fn preview_accent_rejects_ambiguous_or_out_of_range_query() {
        for mut query in [
            serde_json::json!({ "accent_phrases": [] }),
            serde_json::json!({
                "accent_phrases": [
                    { "moras": [{ "text": "テ" }], "accent": 1 },
                    { "moras": [{ "text": "スト" }], "accent": 1 }
                ]
            }),
            serde_json::json!({ "accent_phrases": [{ "moras": [], "accent": 1 }] }),
            serde_json::json!({
                "accent_phrases": [{
                    "moras": [{ "text": "テ" }, { "text": "キ" }, { "text": "スト" }],
                    "accent": 1
                }]
            }),
            serde_json::json!({
                "accent_phrases": [{
                    "moras": [{ "text": "テ" }, { "text": "ス" }, { "text": "ト" }],
                    "accent": 1
                }]
            }),
        ] {
            assert!(matches!(
                apply_user_dict_preview_accent(&mut query, "テスト", 4),
                Err(UserDictPreviewError::InvalidInput)
            ));
        }
    }
}
