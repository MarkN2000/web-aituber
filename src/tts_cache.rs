use std::{
    fs,
    future::Future,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    audio,
    config::{AppConfig, TtsConfig},
};

const MAX_BYTES: u64 = 512 * 1024 * 1024;

pub struct TtsCache {
    directory: PathBuf,
    state: Mutex<CacheState>,
    generation_lock: tokio::sync::Mutex<()>,
}

struct CacheState {
    context: String,
    generation: u64,
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    duration_ms: u64,
    audio_sha256: String,
}

pub struct CachedAudio {
    pub bytes: Vec<u8>,
    pub duration_ms: u64,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn key(config: &TtsConfig, text: &str, accent: Option<u32>) -> String {
    hash(
        &serde_json::to_vec(&(
            config.engine_url.trim_end_matches('/'),
            config.speaker_id,
            text,
            accent,
        ))
        .unwrap(),
    )
}

fn context(config: &AppConfig) -> String {
    hash(
        &serde_json::to_vec(&(
            env!("CARGO_PKG_VERSION"),
            &config.tts,
            &config.ffmpeg_path,
            "m4a-aac-lc-32k-mono",
        ))
        .unwrap(),
    )
}

impl TtsCache {
    pub fn new(directory: PathBuf, config: &AppConfig) -> Self {
        let cache = Self {
            directory,
            state: Mutex::new(CacheState {
                context: context(config),
                generation: 0,
                enabled: false,
            }),
            generation_lock: tokio::sync::Mutex::new(()),
        };
        let mut state = cache.state.lock().unwrap();
        if fs::read_to_string(cache.directory.join("context"))
            .ok()
            .as_ref()
            == Some(&state.context)
        {
            state.enabled = true;
        } else if cache.directory.exists() {
            if let Err(error) = cache.clear_locked(&mut state) {
                tracing::warn!(?error, "音声キャッシュを初期化できませんでした");
            }
        } else {
            // ディレクトリの作成は初回保存時まで遅らせる。
            state.enabled = true;
        }
        drop(state);
        cache
    }

    pub fn update_config(&self, config: &AppConfig) {
        let mut state = self.state.lock().unwrap();
        let next = context(config);
        if state.context != next {
            state.context = next;
            if let Err(error) = self.clear_locked(&mut state) {
                tracing::warn!(?error, "音声設定変更後のキャッシュ削除に失敗しました");
            }
        }
    }

    pub fn clear(&self) -> Result<()> {
        self.clear_locked(&mut self.state.lock().unwrap())
    }

    pub async fn before_dictionary_update(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let guard = self.generation_lock.lock().await;
        if let Err(error) = self.clear() {
            tracing::warn!(?error, "辞書更新前の音声キャッシュ削除に失敗しました");
        }
        guard
    }

    fn clear_locked(&self, state: &mut CacheState) -> Result<()> {
        state.generation = state.generation.wrapping_add(1);
        state.enabled = false;
        match fs::remove_dir_all(&self.directory) {
            Ok(()) => (),
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        state.enabled = true;
        Ok(())
    }

    pub async fn get_or_generate<E>(
        &self,
        key: &str,
        generate: impl Future<Output = std::result::Result<CachedAudio, E>>,
    ) -> std::result::Result<CachedAudio, E> {
        // ponytail: 全生成を直列化する。複数同時発話が必要になったらキー単位のロックへ変更する。
        let _guard = self.generation_lock.lock().await;
        let generation = {
            let state = self.state.lock().unwrap();
            if state.enabled
                && let Ok(audio) = self.read(key)
            {
                return Ok(audio);
            }
            state.generation
        };
        let audio = generate.await?;
        let state = self.state.lock().unwrap();
        if state.enabled
            && state.generation == generation
            && let Err(error) = self.store(key, &audio, &state.context, MAX_BYTES)
        {
            tracing::warn!(
                ?error,
                "音声キャッシュを保存できませんでした。生成した音声を使用します"
            );
        }
        Ok(audio)
    }

    fn read(&self, key: &str) -> Result<CachedAudio> {
        let metadata: Metadata =
            serde_json::from_slice(&fs::read(self.directory.join(format!("{key}.json")))?)?;
        let bytes = fs::read(self.directory.join(format!("{key}.m4a")))?;
        ensure!(
            !bytes.is_empty() && hash(&bytes) == metadata.audio_sha256,
            "音声キャッシュが破損しています"
        );
        Ok(CachedAudio {
            bytes,
            duration_ms: metadata.duration_ms,
        })
    }

    fn store(&self, key: &str, audio: &CachedAudio, context: &str, limit: u64) -> Result<()> {
        let metadata = serde_json::to_vec(&Metadata {
            duration_ms: audio.duration_ms,
            audio_sha256: hash(&audio.bytes),
        })?;
        let size = audio.bytes.len() as u64 + metadata.len() as u64 + context.len() as u64;
        if size > limit {
            return Ok(());
        }
        fs::create_dir_all(&self.directory)?;
        fs::write(self.directory.join("context"), context)?;
        self.prune(size, limit)?;
        let metadata_path = self.directory.join(format!("{key}.json"));
        match fs::remove_file(&metadata_path) {
            Ok(()) => (),
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        fs::write(self.directory.join(format!("{key}.m4a")), &audio.bytes)?;
        fs::write(metadata_path, metadata)?;
        Ok(())
    }

    fn prune(&self, incoming: u64, limit: u64) -> Result<()> {
        // ponytail: 保存時に容量上限内のファイルを全走査する。件数が増えたら索引を導入する。
        let mut files = Vec::new();
        let mut total = incoming;
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry.file_name() == "context" {
                continue;
            }
            let metadata = entry.metadata()?;
            if metadata.is_file() {
                total += metadata.len();
                files.push((metadata.modified()?, entry.path(), metadata.len()));
            }
        }
        files.sort_by_key(|entry| entry.0);
        for (_, path, size) in files {
            if total <= limit {
                break;
            }
            fs::remove_file(path)?;
            total -= size;
        }
        Ok(())
    }
}

/// キャッシュに失敗しても音声を返せるよう、変換先は配信用の一時領域を使う。
pub async fn convert(ffmpeg: &str, wav: &[u8], temporary_directory: &Path) -> Result<CachedAudio> {
    let temporary =
        TemporaryAudio(temporary_directory.join(format!("{}-cache.m4a", uuid::Uuid::new_v4())));
    let duration_ms = audio::transcode_to_aac(ffmpeg, wav, &temporary.0).await?;
    let bytes = tokio::fs::read(&temporary.0).await?;
    Ok(CachedAudio { bytes, duration_ms })
}

struct TemporaryAudio(PathBuf);
impl Drop for TemporaryAudio {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AppConfig {
        serde_json::from_str(include_str!("../config.example.json")).unwrap()
    }
    fn cache() -> TtsCache {
        TtsCache::new(
            std::env::temp_dir().join(format!("tts-cache-{}", uuid::Uuid::new_v4())),
            &config(),
        )
    }
    async fn generated() -> Result<CachedAudio> {
        Ok(CachedAudio {
            bytes: b"converted-m4a".to_vec(),
            duration_ms: 2450,
        })
    }
    async fn must_hit() -> Result<CachedAudio> {
        panic!("キャッシュヒットで生成・変換してはいけません")
    }

    #[test]
    fn キーは音声に影響する入力だけで決まる() {
        let mut voice = config().tts;
        let original = key(&voice, "こんにちは。", None);
        voice.engine_url.push('/');
        assert_eq!(original, key(&voice, "こんにちは。", None));
        assert_ne!(original, key(&voice, "こんにちは。\n", None));
        assert_ne!(original, key(&voice, "こんにちは。", Some(0)));
        voice.speaker_id += 1;
        assert_ne!(original, key(&voice, "こんにちは。", None));
        voice.engine_url.push_str("another");
        assert_ne!(
            key(&config().tts, "こんにちは。", None),
            key(&voice, "こんにちは。", None)
        );
    }

    #[tokio::test]
    async fn 再利用と再起動で生成も変換も省く() {
        let cache = cache();
        let first = cache.get_or_generate("key", generated()).await.unwrap();
        let reopened = TtsCache::new(cache.directory.clone(), &config());
        let second = reopened.get_or_generate("key", must_hit()).await.unwrap();
        assert_eq!(first.bytes, second.bytes);
        assert_eq!(second.duration_ms, 2450);
        cache.clear().unwrap();
    }

    #[tokio::test]
    async fn 旧形式キャッシュは同じアプリバージョンでも破棄する() {
        let cache = cache();
        let config = config();
        cache.get_or_generate("key", generated()).await.unwrap();
        fs::rename(
            cache.directory.join("key.m4a"),
            cache.directory.join("key.webm"),
        )
        .unwrap();
        let old_context = hash(
            &serde_json::to_vec(&(env!("CARGO_PKG_VERSION"), &config.tts, &config.ffmpeg_path))
                .unwrap(),
        );
        fs::write(cache.directory.join("context"), old_context).unwrap();
        let reopened = TtsCache::new(cache.directory.clone(), &config);
        assert!(!reopened.directory.exists());
        reopened.get_or_generate("key", generated()).await.unwrap();
        assert!(reopened.directory.join("key.m4a").exists());
        assert!(!reopened.directory.join("key.webm").exists());
        reopened.clear().unwrap();
    }

    #[tokio::test]
    async fn 欠損と破損は再生成し失敗を保存しない() {
        let cache = cache();
        cache.get_or_generate("key", generated()).await.unwrap();
        for (name, bytes) in [
            ("key.m4a", b"broken".as_slice()),
            ("key.json", b"{".as_slice()),
        ] {
            fs::write(cache.directory.join(name), bytes).unwrap();
            let audio = cache.get_or_generate("key", generated()).await.unwrap();
            assert_eq!(audio.bytes, b"converted-m4a");
        }
        fs::remove_file(cache.directory.join("key.m4a")).unwrap();
        cache.get_or_generate("key", generated()).await.unwrap();
        fs::remove_file(cache.directory.join("key.json")).unwrap();
        assert!(
            cache
                .get_or_generate("key", async { anyhow::bail!("生成失敗") })
                .await
                .is_err()
        );
        assert!(cache.read("key").is_err());
        cache.clear().unwrap();
    }

    #[tokio::test]
    async fn 設定とアプリ更新で全削除し無関係な設定変更では保持する() {
        let cache = cache();
        cache.get_or_generate("key", generated()).await.unwrap();
        let mut config = config();
        config.idle_speech.min_seconds += 1;
        cache.update_config(&config);
        cache.get_or_generate("key", must_hit()).await.unwrap();
        config.tts.speaker_id += 1;
        cache.update_config(&config);
        assert!(!cache.directory.exists());
        cache.get_or_generate("key", generated()).await.unwrap();
        fs::write(cache.directory.join("context"), "old-app-version").unwrap();
        let reopened = TtsCache::new(cache.directory.clone(), &config);
        assert!(reopened.read("key").is_err());
        reopened.clear().unwrap();
    }

    #[tokio::test]
    async fn 同時リクエストは一度だけ生成し削除前の生成は保存しない() {
        let cache = std::sync::Arc::new(cache());
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (finish, resume) = tokio::sync::oneshot::channel();
        let running = cache.clone();
        let task = tokio::spawn(async move {
            running
                .get_or_generate("key", async {
                    started.send(()).unwrap();
                    resume.await.unwrap();
                    generated().await
                })
                .await
                .unwrap()
        });
        waiting.await.unwrap();
        let same = cache.clone();
        let second =
            tokio::spawn(async move { same.get_or_generate("key", must_hit()).await.unwrap() });
        finish.send(()).unwrap();
        assert_eq!(task.await.unwrap().bytes, second.await.unwrap().bytes);

        cache.clear().unwrap();
        let result = cache
            .get_or_generate("key", async {
                cache.clear().unwrap();
                generated().await
            })
            .await
            .unwrap();
        assert_eq!(result.duration_ms, 2450);
        assert!(cache.read("key").is_err());
    }

    #[tokio::test]
    async fn 保存削除が失敗しても生成音声を返し古いキャッシュは使わない() {
        let cache = cache();
        fs::write(&cache.directory, "not-a-directory").unwrap();
        assert_eq!(
            cache
                .get_or_generate("key", generated())
                .await
                .unwrap()
                .duration_ms,
            2450
        );
        assert!(cache.clear().is_err());
        assert!(!cache.state.lock().unwrap().enabled);
        assert_eq!(
            cache
                .get_or_generate("key", generated())
                .await
                .unwrap()
                .duration_ms,
            2450
        );
        fs::remove_file(&cache.directory).unwrap();
        cache.clear().unwrap();
        cache.get_or_generate("key", generated()).await.unwrap();
        cache.get_or_generate("key", must_hit()).await.unwrap();
        cache.clear().unwrap();
    }

    #[tokio::test]
    async fn 容量上限で古い音声を整理し巨大な音声は保存しない() {
        let cache = cache();
        let audio = generated().await.unwrap();
        let context = context(&config());
        let entry_size = audio.bytes.len()
            + serde_json::to_vec(&Metadata {
                duration_ms: audio.duration_ms,
                audio_sha256: hash(&audio.bytes),
            })
            .unwrap()
            .len();
        let limit = (entry_size + context.len()) as u64;
        cache.store("old", &audio, &context, limit).unwrap();
        cache.store("new", &audio, &context, limit).unwrap();
        assert!(cache.read("old").is_err());
        assert!(cache.read("new").is_ok());
        let total: u64 = fs::read_dir(&cache.directory)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= limit);
        cache.store("huge", &audio, &context, 1).unwrap();
        assert!(cache.read("huge").is_err());
        cache.clear().unwrap();
    }
}
