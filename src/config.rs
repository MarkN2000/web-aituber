use std::{
    collections::HashMap,
    env, fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Clone)]
pub struct ConfigStore {
    current: watch::Sender<Arc<AppConfig>>,
    path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

pub struct ConfigReloadResult {
    pub restart_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub bind: String,
    pub admin_token: String,
    pub public_base_url: String,
    pub event_identifier: String,
    pub llm: LlmConfig,
    pub tts: TtsConfig,
    pub ffmpeg_path: String,
    pub character: CharacterConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LlmConfig {
    /// OpenAI Responses API 互換エンドポイントの完全な URL。
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub food_reaction_prompt: String,
    #[serde(default = "default_search_fillers")]
    pub search_fillers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TtsConfig {
    /// VOICEVOX または AivisSpeech Engine のベース URL。
    pub engine_url: String,
    pub speaker_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CharacterConfig {
    #[serde(default = "default_vrm_url")]
    pub vrm_url: String,
    #[serde(default = "default_antialias")]
    pub antialias: bool,
    #[serde(default)]
    pub idle_motions: Vec<String>,
    #[serde(default)]
    pub emotion_motions: HashMap<String, String>,
    #[serde(default)]
    pub food_prop: FoodPropConfig,
    #[serde(default)]
    pub camera: CameraConfig,
    #[serde(default)]
    pub background_color: String,
    #[serde(default = "default_background_music_volume")]
    pub background_music_volume: f32,
    #[serde(default = "default_background_music_duck_ratio")]
    pub background_music_duck_ratio: f32,
    #[serde(default)]
    pub screen_overlays: ScreenOverlaysConfig,
    #[serde(default)]
    pub light: LightConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScreenOverlaysConfig {
    #[serde(default)]
    pub top_left: ScreenOverlayConfig,
    #[serde(default)]
    pub top_right: ScreenOverlayConfig,
    #[serde(default)]
    pub bottom_left: ScreenOverlayConfig,
    #[serde(default)]
    pub bottom_right: ScreenOverlayConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScreenOverlayConfig {
    #[serde(default = "default_screen_overlay_scale")]
    pub scale: u8,
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
    #[serde(default = "default_light_brightness")]
    pub brightness: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FoodPropConfig {
    pub position: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub size: f32,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            vrm_url: default_vrm_url(),
            antialias: default_antialias(),
            idle_motions: Vec::new(),
            emotion_motions: HashMap::new(),
            food_prop: FoodPropConfig::default(),
            camera: CameraConfig::default(),
            background_color: "#1b1b22".to_owned(),
            background_music_volume: default_background_music_volume(),
            background_music_duck_ratio: default_background_music_duck_ratio(),
            screen_overlays: ScreenOverlaysConfig::default(),
            light: LightConfig::default(),
        }
    }
}

impl Default for ScreenOverlayConfig {
    fn default() -> Self {
        Self {
            scale: default_screen_overlay_scale(),
        }
    }
}

impl Default for FoodPropConfig {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            size: 0.2,
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
            brightness: default_light_brightness(),
        }
    }
}

fn default_vrm_url() -> String {
    "/assets/model.vrm".to_owned()
}

fn default_antialias() -> bool {
    true
}

fn default_background_music_volume() -> f32 {
    0.3
}

fn default_background_music_duck_ratio() -> f32 {
    0.4
}

fn default_screen_overlay_scale() -> u8 {
    100
}

fn default_light_brightness() -> f32 {
    1.0
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
        validate_public_base_url(&self.public_base_url)?;
        validate_event_identifier(&self.event_identifier)?;
        required("llm.api_url", &self.llm.api_url)?;
        validate_http_url("llm.api_url", &self.llm.api_url)?;
        required("llm.api_key", &self.llm.api_key)?;
        required("llm.model", &self.llm.model)?;
        required("llm.system_prompt", &self.llm.system_prompt)?;
        required("llm.food_reaction_prompt", &self.llm.food_reaction_prompt)?;
        if self.llm.search_fillers.is_empty() {
            bail!("設定項目 llm.search_fillers は1件以上必要です");
        }
        for (index, filler) in self.llm.search_fillers.iter().enumerate() {
            required(&format!("llm.search_fillers[{index}]"), filler)?;
        }
        required("tts.engine_url", &self.tts.engine_url)?;
        validate_http_url("tts.engine_url", &self.tts.engine_url)?;
        required("ffmpeg_path", &self.ffmpeg_path)?;
        required("character.vrm_url", &self.character.vrm_url)?;
        if !self.character.light.brightness.is_finite()
            || !(0.0..=2.0).contains(&self.character.light.brightness)
        {
            bail!("設定項目 character.light.brightness は0.0から2.0の有限値にしてください");
        }
        if !self.character.background_music_volume.is_finite()
            || !(0.0..=1.0).contains(&self.character.background_music_volume)
        {
            bail!("設定項目 character.background_music_volume は0.0から1.0の有限値にしてください");
        }
        if !self.character.background_music_duck_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.character.background_music_duck_ratio)
        {
            bail!(
                "設定項目 character.background_music_duck_ratio は0.0から1.0の有限値にしてください"
            );
        }
        for (slot, overlay) in [
            ("top_left", &self.character.screen_overlays.top_left),
            ("top_right", &self.character.screen_overlays.top_right),
            ("bottom_left", &self.character.screen_overlays.bottom_left),
            ("bottom_right", &self.character.screen_overlays.bottom_right),
        ] {
            if !(1..=100).contains(&overlay.scale) {
                bail!(
                    "設定項目 character.screen_overlays.{slot}.scale は1から100の整数にしてください"
                );
            }
        }
        if !self.character.food_prop.size.is_finite() || self.character.food_prop.size <= 0.0 {
            bail!("設定項目 character.food_prop.size は0より大きい有限値にしてください");
        }
        if self
            .character
            .food_prop
            .position
            .iter()
            .chain(self.character.food_prop.rotation_degrees.iter())
            .any(|value| !value.is_finite())
        {
            bail!("設定項目 character.food_prop の位置と回転は有限値にしてください");
        }
        Ok(())
    }
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        if let Some(path) = env::var_os("APP_CONFIG_FILE") {
            let path = PathBuf::from(path);
            let config = AppConfig::load_from_path(&path)?;
            return Ok(Self::new(path, config));
        }

        let path = PathBuf::from("config.json");
        let generated = create_default_config(&path, Path::new("config.example.json"))?;
        let config = AppConfig::load_from_path(&path)?;
        if generated {
            tracing::warn!(
                path = %path.display(),
                "初回設定ファイルを生成しました。llm.api_keyなどを編集して再起動してください"
            );
        }
        Ok(Self::new(path, config))
    }

    pub fn new(path: impl Into<PathBuf>, config: AppConfig) -> Self {
        let (current, _) = watch::channel(Arc::new(config));
        Self {
            current,
            path: Arc::new(path.into()),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn current(&self) -> Arc<AppConfig> {
        self.current.borrow().clone()
    }

    pub fn reload(&self) -> Result<ConfigReloadResult> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("設定の保存ロックを取得できません"))?;
        let mut replacement = AppConfig::load_from_path(self.path.as_ref())?;
        let current = self.current();
        let restart_required = replacement.bind != current.bind;
        if restart_required {
            replacement.bind.clone_from(&current.bind);
        }
        self.current.send_replace(Arc::new(replacement));
        Ok(ConfigReloadResult { restart_required })
    }

    /// 変更を永続化できた場合だけ、実行中の設定を更新する。
    pub fn update_and_save<F>(&self, update: F) -> Result<ConfigReloadResult>
    where
        F: FnOnce(&mut AppConfig),
    {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("設定の保存ロックを取得できません"))?;
        let current = self.current();
        // 画面に出さない項目を外部編集で変更していても、古い実行中設定で上書きしない。
        let mut replacement = AppConfig::load_from_path(self.path.as_ref())?;
        update(&mut replacement);
        replacement.validate()?;
        write_config_atomically(self.path.as_ref(), &replacement)?;

        let restart_required = replacement.bind != current.bind;
        if restart_required {
            replacement.bind.clone_from(&current.bind);
        }
        self.current.send_replace(Arc::new(replacement));
        Ok(ConfigReloadResult { restart_required })
    }
}

fn create_default_config(path: &Path, example_path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }

    let mut config = AppConfig::load_from_path(example_path).with_context(|| {
        format!(
            "初回設定の生成元を読み込めません: {}",
            example_path.display()
        )
    })?;
    config.admin_token = uuid::Uuid::new_v4().simple().to_string();
    config.event_identifier = uuid::Uuid::new_v4().simple().to_string();
    config.validate()?;
    let serialized =
        serde_json::to_vec_pretty(&config).context("初回設定をJSONへ変換できません")?;
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("初回設定ファイルを作成できません: {}", path.display()));
        }
    };
    let result = (|| -> Result<()> {
        file.write_all(&serialized)
            .context("初回設定ファイルへ書き込めません")?;
        file.write_all(b"\n")
            .context("初回設定ファイルへ書き込めません")?;
        file.sync_all()
            .context("初回設定ファイルを同期できません")?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(true)
}

fn write_config_atomically(path: &Path, config: &AppConfig) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let permissions = fs::metadata(path)
        .with_context(|| format!("設定ファイルの情報を取得できません: {}", path.display()))?
        .permissions();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let serialized = serde_json::to_vec_pretty(config).context("設定をJSONへ変換できません")?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| {
                format!("一時設定ファイルを作成できません: {}", temporary.display())
            })?;
        file.set_permissions(permissions)
            .context("一時設定ファイルへ現在のアクセス権を引き継げません")?;
        file.write_all(&serialized)
            .context("一時設定ファイルへ書き込めません")?;
        file.write_all(b"\n")
            .context("一時設定ファイルへ書き込めません")?;
        file.sync_all()
            .context("一時設定ファイルを同期できません")?;
        drop(file);
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, path: &Path) -> Result<()> {
    fs::rename(temporary, path)
        .with_context(|| format!("設定ファイルを置き換えられません: {}", path.display()))
}

#[cfg(windows)]
fn replace_file(temporary: &Path, path: &Path) -> Result<()> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    let destination = wide(path.as_os_str());
    let replacement = wide(temporary.as_os_str());
    // 設定ファイルは起動時に必ず存在するため、既存ファイルを原子的に置換する。
    let result = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        bail!(
            "設定ファイルを原子的に置き換えられません: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn validate_http_url(name: &str, value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value)
        .with_context(|| format!("設定項目 {name} はHTTP(S) URLにしてください"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("設定項目 {name} はHTTP(S) URLにしてください");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("設定項目 {name} のURLにユーザー名やパスワードを含めないでください");
    }
    Ok(())
}

pub fn validate_event_identifier(value: &str) -> Result<()> {
    if !(1..=64).contains(&value.len()) {
        bail!("設定項目 event_identifier は1文字から64文字にしてください");
    }
    if !value.bytes().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
    }) {
        bail!("設定項目 event_identifier は英小文字、数字、ハイフンだけにしてください");
    }
    if value.starts_with('-') || value.ends_with('-') {
        bail!("設定項目 event_identifier の先頭と末尾にハイフンは使用できません");
    }
    Ok(())
}

pub fn validate_public_base_url(value: &str) -> Result<()> {
    required("public_base_url", value)?;
    validate_http_url("public_base_url", value)?;
    let url = reqwest::Url::parse(value).expect("HTTP(S) URLの検証後は解析できる");
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        bail!("設定項目 public_base_url にはパス、クエリ、フラグメントを含めないでください");
    }
    Ok(())
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
    use uuid::Uuid;

    #[test]
    fn event_identifier_accepts_public_url_safe_value() {
        assert!(validate_event_identifier("summer-2026-8k2m").is_ok());
        assert!(validate_event_identifier("a").is_ok());
    }

    #[test]
    fn event_identifier_rejects_empty_uppercase_and_edge_hyphen() {
        assert!(validate_event_identifier("").is_err());
        assert!(validate_event_identifier(&"a".repeat(65)).is_err());
        assert!(validate_event_identifier("Event-2026").is_err());
        assert!(validate_event_identifier("-event-2026").is_err());
        assert!(validate_event_identifier("event-2026-").is_err());
    }

    #[test]
    fn public_base_url_accepts_an_origin_and_rejects_extra_components() {
        assert!(validate_public_base_url("https://event.example.com").is_ok());
        assert!(validate_public_base_url("http://192.168.1.2:3000/").is_ok());
        assert!(validate_public_base_url("https://event.example.com/path").is_err());
        assert!(validate_public_base_url("https://event.example.com/?key=value").is_err());
    }

    #[test]
    fn character_defaults_are_available() {
        let character: CharacterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(character.vrm_url, "/assets/model.vrm");
        assert!(character.antialias);
        assert_eq!(character.food_prop.size, 0.2);
        assert_eq!(character.camera.fov, 30.0);
        assert_eq!(character.background_music_volume, 0.3);
        assert_eq!(character.background_music_duck_ratio, 0.4);
        assert_eq!(character.screen_overlays.top_left.scale, 100);
        assert_eq!(character.screen_overlays.bottom_right.scale, 100);
        assert_eq!(character.light.ambient_intensity, 0.8);
        assert_eq!(character.light.brightness, 1.0);
    }

    #[test]
    fn previous_config_without_antialias_uses_default() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        value["character"]
            .as_object_mut()
            .unwrap()
            .remove("antialias");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.character.antialias);
        config.validate().unwrap();
    }

    #[test]
    fn example_config_is_valid() {
        let config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn screen_overlay_scales_must_be_between_one_and_one_hundred() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.character.screen_overlays.top_left.scale = 1;
        config.character.screen_overlays.bottom_right.scale = 100;
        assert!(config.validate().is_ok());

        config.character.screen_overlays.top_right.scale = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn missing_default_config_is_generated_once_with_random_identifiers() {
        let root = std::env::temp_dir().join(format!("web-aituber-config-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let example_path = root.join("config.example.json");
        let config_path = root.join("config.json");
        let source = include_str!("../config.example.json");
        fs::write(&example_path, source).unwrap();
        let example: AppConfig = serde_json::from_str(source).unwrap();

        assert!(create_default_config(&config_path, &example_path).unwrap());
        let generated = AppConfig::load_from_path(&config_path).unwrap();
        assert_ne!(generated.admin_token, example.admin_token);
        assert_ne!(generated.event_identifier, example.event_identifier);
        assert_eq!(generated.admin_token.len(), 32);
        assert_eq!(generated.event_identifier.len(), 32);

        fs::write(
            &config_path,
            source.replace("gpt-5.6-luna", "keep-this-model"),
        )
        .unwrap();
        assert!(!create_default_config(&config_path, &example_path).unwrap());
        assert_eq!(
            AppConfig::load_from_path(&config_path).unwrap().llm.model,
            "keep-this-model"
        );

        fs::remove_dir_all(root).unwrap();
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

    #[test]
    fn prompts_cannot_be_empty() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();

        config.llm.system_prompt.clear();
        assert!(config.validate().is_err());

        config.llm.system_prompt = "通常質問の指示".to_owned();
        config.llm.food_reaction_prompt = "   ".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn background_music_volumes_must_be_between_zero_and_one() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();

        for valid in [0.0, 0.3, 1.0] {
            config.character.background_music_volume = valid;
            assert!(config.validate().is_ok());
        }
        for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            config.character.background_music_volume = invalid;
            assert!(config.validate().is_err());
        }
        config.character.background_music_volume = 0.3;
        for valid in [0.0, 0.4, 1.0] {
            config.character.background_music_duck_ratio = valid;
            assert!(config.validate().is_ok());
        }
        for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            config.character.background_music_duck_ratio = invalid;
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn model_brightness_must_be_between_zero_and_two() {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();

        for valid in [0.0, 1.0, 2.0] {
            config.character.light.brightness = valid;
            assert!(config.validate().is_ok());
        }
        for invalid in [-0.1, 2.1, f32::NAN, f32::INFINITY] {
            config.character.light.brightness = invalid;
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn reload_replaces_valid_settings_and_keeps_the_previous_settings_on_error() {
        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let source = include_str!("../config.example.json");
        fs::write(&path, source).unwrap();
        let initial: AppConfig = serde_json::from_str(source).unwrap();
        let store = ConfigStore::new(&path, initial);

        let mut replacement: AppConfig = serde_json::from_str(source).unwrap();
        replacement.bind = "127.0.0.1:4000".to_owned();
        replacement.llm.model = "new-model".to_owned();
        replacement.llm.food_reaction_prompt = "設定再読み込み後の食事反応です。".to_owned();
        let replacement = serde_json::to_string_pretty(&replacement).unwrap();
        fs::write(&path, replacement).unwrap();
        let result = store.reload().unwrap();
        assert!(result.restart_required);
        assert_eq!(store.current().bind, "0.0.0.0:3000");
        assert_eq!(store.current().llm.model, "new-model");
        assert!(
            store
                .current()
                .llm
                .food_reaction_prompt
                .contains("設定再読み込み後の食事反応です。")
        );

        fs::write(&path, "{}").unwrap();
        assert!(store.reload().is_err());
        assert_eq!(store.current().llm.model, "new-model");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn update_and_save_persists_only_after_validation() {
        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let source = include_str!("../config.example.json");
        fs::write(&path, source).unwrap();
        let initial: AppConfig = serde_json::from_str(source).unwrap();
        let store = ConfigStore::new(&path, initial);

        store
            .update_and_save(|config| config.llm.model = "updated-model".to_owned())
            .unwrap();
        assert_eq!(store.current().llm.model, "updated-model");
        let saved = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.llm.model, "updated-model");

        assert!(
            store
                .update_and_save(|config| config.llm.system_prompt.clear())
                .is_err()
        );
        assert_eq!(store.current().llm.model, "updated-model");
        let saved = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.llm.model, "updated-model");

        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn update_and_save_keeps_config_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let source = include_str!("../config.example.json");
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let initial: AppConfig = serde_json::from_str(source).unwrap();
        let store = ConfigStore::new(&path, initial);

        store
            .update_and_save(|config| config.llm.model = "updated-model".to_owned())
            .unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn http_url_validation_rejects_non_http_schemes() {
        assert!(validate_http_url("test", "https://example.com/api").is_ok());
        assert!(validate_http_url("test", "ftp://example.com").is_err());
        assert!(validate_http_url("test", "https://user:password@example.com").is_err());
        assert!(validate_http_url("test", "not a url").is_err());
    }
}
