use std::{collections::HashMap, env, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    pub bind: String,
    pub admin_token: String,
    pub llm: LlmConfig,
    pub tts: TtsConfig,
    pub ffmpeg_path: String,
    pub character: CharacterConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LlmConfig {
    /// OpenAI Responses API 互換エンドポイントの完全な URL。
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    #[serde(default = "default_search_fillers")]
    pub search_fillers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TtsConfig {
    /// VOICEVOX または AivisSpeech Engine のベース URL。
    pub engine_url: String,
    pub speaker_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CharacterConfig {
    #[serde(default = "default_vrm_url")]
    pub vrm_url: String,
    #[serde(default)]
    pub idle_motions: Vec<String>,
    #[serde(default)]
    pub emotion_motions: HashMap<String, String>,
    #[serde(default)]
    pub camera: CameraConfig,
    #[serde(default)]
    pub background_color: String,
    #[serde(default)]
    pub light: LightConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CameraConfig {
    pub fov: f32,
    pub position: [f32; 3],
    pub target: [f32; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LightConfig {
    pub color: String,
    pub intensity: f32,
    pub position: [f32; 3],
    pub ambient_intensity: f32,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            vrm_url: default_vrm_url(),
            idle_motions: Vec::new(),
            emotion_motions: HashMap::new(),
            camera: CameraConfig::default(),
            background_color: "#1b1b22".to_owned(),
            light: LightConfig::default(),
        }
    }
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            fov: 30.0,
            position: [0.0, 1.4, 2.5],
            target: [0.0, 1.2, 0.0],
        }
    }
}

impl Default for LightConfig {
    fn default() -> Self {
        Self {
            color: "#ffffff".to_owned(),
            intensity: 1.5,
            position: [2.0, 3.0, 2.0],
            ambient_intensity: 0.8,
        }
    }
}

fn default_vrm_url() -> String {
    "/assets/model.vrm".to_owned()
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = env::var("APP_CONFIG_FILE").unwrap_or_else(|_| "config.json".to_owned());
        Self::load_from_path(path)
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let source = fs::read_to_string(path)
            .with_context(|| format!("設定ファイルを読み込めません: {}", path.display()))?;
        let config: Self = serde_json::from_str(&source)
            .with_context(|| format!("設定ファイルの JSON が不正です: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        required("bind", &self.bind)?;
        required("admin_token", &self.admin_token)?;
        required("llm.api_url", &self.llm.api_url)?;
        required("llm.api_key", &self.llm.api_key)?;
        required("llm.model", &self.llm.model)?;
        if self.llm.search_fillers.is_empty() {
            bail!("設定項目 llm.search_fillers は1件以上必要です");
        }
        for (index, filler) in self.llm.search_fillers.iter().enumerate() {
            required(&format!("llm.search_fillers[{index}]"), filler)?;
        }
        required("tts.engine_url", &self.tts.engine_url)?;
        required("ffmpeg_path", &self.ffmpeg_path)?;
        required("character.vrm_url", &self.character.vrm_url)?;
        Ok(())
    }
}

fn default_search_fillers() -> Vec<String> {
    vec!["少し調べてみますね。".to_owned()]
}

fn required(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("設定項目 {name} は空にできません");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_defaults_are_available() {
        let character: CharacterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(character.vrm_url, "/assets/model.vrm");
        assert_eq!(character.camera.fov, 30.0);
        assert_eq!(character.light.ambient_intensity, 0.8);
    }

    #[test]
    fn example_config_is_valid() {
        let config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn search_fillers_require_at_least_one_non_empty_sentence() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();

        config.llm.search_fillers.clear();
        assert!(config.validate().is_err());

        config.llm.search_fillers = vec!["   ".to_owned()];
        assert!(config.validate().is_err());
    }
}
