use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Multipart, Path, Query, State, WebSocketUpgrade, multipart::Field,
        ws::Message,
    },
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path as FilePath, time::Duration};
use tokio::io::AsyncWriteExt;
use tower_http::{services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{
    background_music,
    config::{TtsConfig, validate_http_url},
    protocol::{AdminSkipRequest, InputImage, ServerEvent, Submission, SubmissionKind},
    state::AppState,
    tts,
};

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_TEXT_REQUEST_BYTES: usize = 128 * 1024;
const MAX_FOOD_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;
const MAX_BACKGROUND_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;
const MAX_BACKGROUND_MUSIC_REQUEST_BYTES: usize = background_music::MAX_SOURCE_BYTES + 128 * 1024;
const BACKGROUND_IMAGE_FILE_NAME: &str = "background.webp";

pub fn router(state: AppState) -> Router {
    let admin_api = Router::new()
        .route("/api/admin/skip", post(skip))
        .route("/api/admin/reload-config", post(reload_config))
        .route(
            "/api/admin/config",
            get(admin_config).put(update_admin_config),
        )
        .route("/api/admin/tts-preview", post(tts_preview))
        .route("/api/admin/tts-speakers", post(tts_speakers))
        .route(
            "/api/admin/tts-user-dict-preview",
            post(tts_user_dict_preview),
        )
        .route("/api/admin/tts-user-dict", post(tts_user_dict))
        .route(
            "/api/admin/tts-user-dict-word",
            post(add_tts_user_dict_word),
        )
        .route(
            "/api/admin/tts-user-dict-word/{word_uuid}",
            axum::routing::put(update_tts_user_dict_word).delete(delete_tts_user_dict_word),
        )
        .route(
            "/api/admin/background-image",
            post(upload_background_image)
                .delete(delete_background_image)
                .layer(DefaultBodyLimit::max(MAX_BACKGROUND_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/background-music",
            post(upload_background_music)
                .delete(delete_background_music)
                .layer(DefaultBodyLimit::max(MAX_BACKGROUND_MUSIC_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/background-music-volume",
            axum::routing::put(update_background_music_volume),
        )
        .layer(middleware::map_response(add_admin_response_headers));

    Router::new()
        .route("/", get(main_page))
        .route("/input", get(input_page))
        .route("/draw", get(draw_page))
        .route("/admin", get(admin_page))
        .route(
            "/api/submissions",
            post(submit).layer(DefaultBodyLimit::max(MAX_TEXT_REQUEST_BYTES)),
        )
        .route(
            "/api/food-submissions",
            post(submit_food).layer(DefaultBodyLimit::max(MAX_FOOD_REQUEST_BYTES)),
        )
        .route("/api/display-config", get(display_config))
        .merge(admin_api)
        .route("/ws", get(websocket))
        .route("/food-images/{id}", get(food_image))
        .nest_service("/static", ServeDir::new("web"))
        .nest_service("/assets", ServeDir::new(state.assets_dir.as_ref()))
        .nest_service("/audio", ServeDir::new(state.audio_dir.as_ref()))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn main_page() -> Response {
    html_file("web/main.html").await
}

async fn input_page() -> Response {
    html_file("web/input.html").await
}

async fn draw_page() -> Response {
    html_file("web/draw.html").await
}

async fn admin_page(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    admin_no_store(html_file("web/admin.html").await)
}

async fn html_file(path: &str) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], bytes).into_response(),
        Err(error) => {
            tracing::error!(path, error = ?error, "HTMLファイルを読み込めませんでした");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn submit(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmitResponse>), ApiError> {
    let mut text = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad_request("投稿を読み取れませんでした"))?
    {
        match field.name() {
            Some("text") => {
                text = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ApiError::bad_request("テキストを読み取れませんでした"))?,
                );
            }
            Some("image") => {
                return Err(ApiError::bad_request("通常の質問には画像を送信できません"));
            }
            _ => {}
        }
    }

    let text = text.unwrap_or_default().trim().to_owned();
    if text.is_empty() {
        return Err(ApiError::bad_request("質問を入力してください"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError::bad_request("質問は2000文字以下にしてください"));
    }

    let id = Uuid::new_v4().to_string();
    state
        .submissions
        .try_send(Submission {
            id: id.clone(),
            kind: SubmissionKind::Question,
            text,
        })
        .map_err(queue_error)?;

    Ok((StatusCode::ACCEPTED, Json(SubmitResponse { id })))
}

async fn submit_food(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmitResponse>), ApiError> {
    let mut vrm_image = None;
    let mut ai_image = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad_request("投稿を読み取れませんでした"))?
    {
        match field.name() {
            Some("vrm_image") if vrm_image.is_none() => {
                vrm_image = Some(read_food_image(field).await?);
            }
            Some("ai_image") if ai_image.is_none() => {
                ai_image = Some(read_food_image(field).await?);
            }
            _ => {}
        }
    }

    let vrm_image =
        vrm_image.ok_or_else(|| ApiError::bad_request("VRM表示用の食事画像がありません"))?;
    let ai_image =
        ai_image.ok_or_else(|| ApiError::bad_request("AI入力用の食事画像がありません"))?;
    let id = Uuid::new_v4().to_string();
    state
        .submissions
        .try_send(Submission {
            id: id.clone(),
            kind: SubmissionKind::Food {
                vrm_image,
                ai_image,
            },
            text: "食べ物の絵を送りました".to_owned(),
        })
        .map_err(queue_error)?;

    Ok((StatusCode::ACCEPTED, Json(SubmitResponse { id })))
}

async fn read_food_image(field: Field<'_>) -> Result<InputImage, ApiError> {
    if field.file_name().is_some_and(str::is_empty) {
        return Err(ApiError::bad_request("食べ物の絵を描いてください"));
    }
    let mime_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    if mime_type != "image/webp" {
        return Err(ApiError::bad_request(
            "食べ物の絵はWebP形式で送信してください",
        ));
    }
    let data = field
        .bytes()
        .await
        .map_err(|_| ApiError::bad_request("画像を読み取れませんでした"))?;
    if data.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::bad_request("画像は10MB以下にしてください"));
    }
    Ok(InputImage {
        mime_type,
        data: data.to_vec(),
    })
}

fn queue_error(error: tokio::sync::mpsc::error::TrySendError<Submission>) -> ApiError {
    match error {
        tokio::sync::mpsc::error::TrySendError::Full(_) => {
            ApiError::unavailable("現在混雑しています。少し待ってから再送してください")
        }
        tokio::sync::mpsc::error::TrySendError::Closed(_) => {
            ApiError::unavailable("現在投稿を受け付けられません")
        }
    }
}

async fn display_config(State(state): State<AppState>) -> Response {
    let config = state.config.current();
    let background_image_path = state.assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
    let background_image_url = match tokio::fs::try_exists(&background_image_path).await {
        Ok(true) => Some(format!(
            "/assets/{BACKGROUND_IMAGE_FILE_NAME}?v={}",
            Uuid::new_v4()
        )),
        Ok(false) => None,
        Err(error) => {
            tracing::warn!(path = %background_image_path.display(), error = ?error, "背景画像の存在を確認できませんでした");
            None
        }
    };
    let background_music_path = state.assets_dir.join(background_music::FILE_NAME);
    let background_music_url = match tokio::fs::try_exists(&background_music_path).await {
        Ok(true) => Some(format!(
            "/assets/{}?v={}",
            background_music::FILE_NAME,
            Uuid::new_v4()
        )),
        Ok(false) => None,
        Err(error) => {
            tracing::warn!(path = %background_music_path.display(), error = ?error, "BGMの存在を確認できませんでした");
            None
        }
    };
    let response = DisplayConfigDto {
        character: config.character.clone(),
        background_image_url,
        background_music_url,
    };
    no_store(Json(response).into_response())
}

async fn upload_background_image(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    mut multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let mut image = None;
    while let Some(field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(_) => return admin_error(StatusCode::BAD_REQUEST, "背景画像を読み取れませんでした"),
    } {
        if field.name() != Some("image") || image.is_some() {
            continue;
        }
        if field.content_type() != Some("image/webp") {
            return admin_error(
                StatusCode::BAD_REQUEST,
                "背景画像はWebP形式で送信してください",
            );
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return admin_error(StatusCode::BAD_REQUEST, "背景画像を読み取れませんでした");
            }
        };
        if bytes.len() > MAX_IMAGE_BYTES {
            return admin_error(StatusCode::BAD_REQUEST, "背景画像は10MiB以下にしてください");
        }
        if !has_valid_webp_container(&bytes) {
            return admin_error(StatusCode::BAD_REQUEST, "背景画像のWebP形式が不正です");
        }
        image = Some(bytes);
    }

    let Some(image) = image else {
        return admin_error(StatusCode::BAD_REQUEST, "背景画像がありません");
    };
    let _guard = state.background_image_lock.lock().await;
    let path = state.assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
    match tokio::task::spawn_blocking(move || write_file_atomically(&path, &image)).await {
        Ok(Ok(())) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(error)) => {
            tracing::error!(error = ?error, "背景画像を保存できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "背景画像を保存できませんでした",
            )
        }
        Err(error) => {
            tracing::error!(error = ?error, "背景画像の保存処理を実行できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "背景画像を保存できませんでした",
            )
        }
    }
}

async fn delete_background_image(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let _guard = state.background_image_lock.lock().await;
    let path = state.assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::error!(path = %path.display(), error = ?error, "背景画像を削除できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "背景画像を削除できませんでした",
            )
        }
    }
}

async fn upload_background_music(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    mut multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let guard = state.background_music_lock.clone().lock_owned().await;
    if let Err(error) = tokio::fs::create_dir_all(state.assets_dir.as_ref()).await {
        tracing::error!(error = ?error, "BGMの保存先を作成できませんでした");
        return admin_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "BGMを保存できませんでした",
        );
    }

    let mut uploaded = None;
    while let Some(mut field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(_) => return admin_error(StatusCode::BAD_REQUEST, "BGMを読み取れませんでした"),
    } {
        if field.name() != Some("audio") || uploaded.is_some() {
            continue;
        }
        let Some(extension) = field
            .file_name()
            .and_then(background_music::accepted_extension)
        else {
            return admin_error(
                StatusCode::BAD_REQUEST,
                "BGMはMP3、OGG、WAV形式で送信してください",
            );
        };
        let temporary = background_music::TemporaryFiles::new(&state.assets_dir, extension);
        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary.input())
            .await
        {
            Ok(file) => file,
            Err(error) => {
                tracing::error!(error = ?error, "BGMの一時ファイルを作成できませんでした");
                return admin_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "BGMを保存できませんでした",
                );
            }
        };

        let mut size = 0_usize;
        loop {
            let chunk = match field.chunk().await {
                Ok(chunk) => chunk,
                Err(_) => {
                    return admin_error(StatusCode::BAD_REQUEST, "BGMを読み取れませんでした");
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            size = match checked_background_music_size(size, chunk.len()) {
                Some(size) => size,
                None => {
                    return admin_error(StatusCode::BAD_REQUEST, "BGMは100MiB以下にしてください");
                }
            };
            if let Err(error) = file.write_all(&chunk).await {
                tracing::error!(error = ?error, "BGMの一時ファイルへ書き込めませんでした");
                return admin_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "BGMを保存できませんでした",
                );
            }
        }
        if size == 0 {
            return admin_error(StatusCode::BAD_REQUEST, "BGMが空です");
        }
        if let Err(error) = file.sync_all().await {
            tracing::error!(error = ?error, "BGMの一時ファイルを同期できませんでした");
            return admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BGMを保存できませんでした",
            );
        }
        drop(file);
        uploaded = Some(temporary);
    }

    let Some(temporary) = uploaded else {
        return admin_error(StatusCode::BAD_REQUEST, "BGMがありません");
    };
    let ffmpeg_path = state.config.current().ffmpeg_path.clone();
    let conversion = tokio::spawn(async move {
        let result =
            background_music::convert(&ffmpeg_path, temporary.input(), temporary.output()).await;
        (result, temporary, guard)
    });
    let (conversion_result, temporary, _guard) = match conversion.await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(error = ?error, "BGM変換タスクが異常終了しました");
            return admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BGMを変換できませんでした",
            );
        }
    };
    if let Err(error) = conversion_result {
        tracing::warn!(error = ?error, "BGMを変換できませんでした");
        return admin_error(StatusCode::BAD_REQUEST, "BGMを変換できませんでした");
    }

    let source = temporary.output().to_owned();
    let destination = state.assets_dir.join(background_music::FILE_NAME);
    match background_music::install_atomically(&source, &destination) {
        Ok(()) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Err(error) => {
            tracing::error!(error = ?error, "BGMを保存できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BGMを保存できませんでした",
            )
        }
    }
}

fn checked_background_music_size(current: usize, chunk: usize) -> Option<usize> {
    current
        .checked_add(chunk)
        .filter(|size| *size <= background_music::MAX_SOURCE_BYTES)
}

async fn delete_background_music(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let _guard = state.background_music_lock.lock().await;
    let path = state.assets_dir.join(background_music::FILE_NAME);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::error!(path = %path.display(), error = ?error, "BGMを削除できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BGMを削除できませんでした",
            )
        }
    }
}

async fn update_background_music_volume(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<BackgroundMusicVolumeRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if !request.volume.is_finite()
        || !(0.0..=1.0).contains(&request.volume)
        || !request.duck_ratio.is_finite()
        || !(0.0..=1.0).contains(&request.duck_ratio)
    {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "BGM音量と発話中比率は0.0から1.0で指定してください",
        );
    }

    match state.config.update_and_save(move |config| {
        config.character.background_music_volume = request.volume;
        config.character.background_music_duck_ratio = request.duck_ratio;
    }) {
        Ok(_) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Err(error) => {
            tracing::warn!(error = ?error, "BGM音量を保存できませんでした");
            admin_error(StatusCode::BAD_REQUEST, "BGM音量を保存できませんでした")
        }
    }
}

fn has_valid_webp_container(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if riff_size.checked_add(8) != Some(bytes.len()) {
        return false;
    }
    if !matches!(&bytes[12..16], b"VP8 " | b"VP8L" | b"VP8X") {
        return false;
    }
    let chunk_size = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let padded_chunk_size = match chunk_size.checked_add(chunk_size % 2) {
        Some(size) => size,
        None => return false,
    };
    20_usize
        .checked_add(padded_chunk_size)
        .is_some_and(|end| end <= bytes.len())
}

fn write_file_atomically(path: &FilePath, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| FilePath::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(BACKGROUND_IMAGE_FILE_NAME);
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_or_create_file(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_or_create_file(temporary: &FilePath, path: &FilePath) -> anyhow::Result<()> {
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_or_create_file(temporary: &FilePath, path: &FilePath) -> anyhow::Result<()> {
    if !path.exists() {
        fs::rename(temporary, path)?;
        return Ok(());
    }

    use anyhow::bail;
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    let destination = wide(path.as_os_str());
    let replacement = wide(temporary.as_os_str());
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
        bail!("背景画像を原子的に置き換えられません: {}", path.display());
    }
    Ok(())
}

async fn food_image(Path(id): Path<String>, State(state): State<AppState>) -> Response {
    let image = state.food_images.read().await.get(&id).cloned();
    match image {
        Some(image) => (
            [
                (header::CONTENT_TYPE, image.mime_type),
                (header::CACHE_CONTROL, "no-store".to_owned()),
            ],
            image.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn websocket(websocket: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    websocket.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn skip(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminSkipRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let active = state.active.lock().await;
    if let Some(active) = active.as_ref()
        && active.turn_id == request.turn_id
        && !active.cancel.is_cancelled()
    {
        active.cancel.cancel();
    }
    admin_no_store(StatusCode::NO_CONTENT.into_response())
}

async fn reload_config(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    match state.config.reload() {
        Ok(result) => admin_no_store(
            Json(AdminReloadResponse {
                restart_required: result.restart_required,
            })
            .into_response(),
        ),
        Err(error) => {
            tracing::warn!(error = ?error, "設定の再読み込みに失敗しました");
            admin_no_store(
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("設定を再読み込みできません: {error}"),
                    })),
                )
                    .into_response(),
            )
        }
    }
}

async fn admin_config(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    let config = state.config.current();
    admin_no_store(Json(AdminConfigDto::from_config(&config)).into_response())
}

async fn update_admin_config(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminConfigDto>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    match state.config.update_and_save(move |config| {
        config.llm.api_url = request.llm.api_url;
        config.llm.model = request.llm.model;
        config.llm.system_prompt = request.llm.system_prompt;
        config.llm.food_reaction_prompt = request.llm.food_reaction_prompt;
        config.llm.search_fillers = request.llm.search_fillers;
        config.tts.engine_url = request.tts.engine_url;
        config.tts.speaker_id = request.tts.speaker_id;
    }) {
        Ok(result) => admin_no_store(
            Json(AdminReloadResponse {
                restart_required: result.restart_required,
            })
            .into_response(),
        ),
        Err(error) => admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("設定を保存できません: {error}") })),
            )
                .into_response(),
        ),
    }
}

async fn tts_preview(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsPreviewRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if validate_http_url("tts.engine_url", &request.tts.engine_url).is_err() {
        return admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TTSの接続先はHTTP(S) URLにしてください"
                })),
            )
                .into_response(),
        );
    }
    let config = TtsConfig {
        engine_url: request.tts.engine_url,
        speaker_id: request.tts.speaker_id,
    };
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::synthesize(&state.http, &config, "こんにちは。音声の試聴です。"),
    )
    .await
    {
        Ok(Ok(wav)) => admin_no_store(([(header::CONTENT_TYPE, "audio/wav")], wav).into_response()),
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSの試聴に失敗しました");
            admin_no_store(
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "TTSの試聴に失敗しました" })),
                )
                    .into_response(),
            )
        }
        Err(_) => admin_no_store(
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({ "error": "TTSの試聴が時間切れになりました" })),
            )
                .into_response(),
        ),
    }
}

async fn tts_speakers(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsSpeakersRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if validate_http_url("tts.engine_url", &request.engine_url).is_err() {
        return admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TTSの接続先はHTTP(S) URLにしてください"
                })),
            )
                .into_response(),
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::fetch_speakers(&state.http, &request.engine_url),
    )
    .await
    {
        Ok(Ok(speakers)) => {
            admin_no_store(Json(AdminTtsSpeakersResponse { speakers }).into_response())
        }
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSの話者一覧取得に失敗しました");
            admin_no_store(
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "TTSの話者一覧を取得できません" })),
                )
                    .into_response(),
            )
        }
        Err(_) => admin_no_store(
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({ "error": "TTSの話者一覧取得が時間切れになりました" })),
            )
                .into_response(),
        ),
    }
}

async fn tts_user_dict(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsEngineRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if validate_http_url("tts.engine_url", &request.engine_url).is_err() {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "TTSの接続先はHTTP(S) URLにしてください",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::fetch_user_dict(&state.http, &request.engine_url),
    )
    .await
    {
        Ok(Ok(dictionary)) => admin_no_store(Json(dictionary).into_response()),
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSのユーザー辞書取得に失敗しました");
            admin_error(
                StatusCode::BAD_GATEWAY,
                "TTSのユーザー辞書を取得できませんでした",
            )
        }
        Err(_) => admin_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TTSのユーザー辞書取得が時間切れになりました",
        ),
    }
}

async fn tts_user_dict_preview(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsUserDictPreviewRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if validate_http_url("tts.engine_url", &request.tts.engine_url).is_err() {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "TTSの接続先はHTTP(S) URLにしてください",
        );
    }
    if !is_user_dict_pronunciation(&request.pronunciation) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ユーザー辞書の読みはカタカナで入力してください",
        );
    }
    let config = TtsConfig {
        engine_url: request.tts.engine_url,
        speaker_id: request.tts.speaker_id,
    };
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::synthesize_user_dict_preview(
            &state.http,
            &config,
            &request.pronunciation,
            request.accent_type,
        ),
    )
    .await
    {
        Ok(Ok(wav)) => admin_no_store(([(header::CONTENT_TYPE, "audio/wav")], wav).into_response()),
        Ok(Err(tts::UserDictPreviewError::InvalidInput)) => admin_error(
            StatusCode::BAD_REQUEST,
            "単語の読みまたはアクセント位置を確認してください",
        ),
        Ok(Err(tts::UserDictPreviewError::Engine(error))) => {
            tracing::warn!(error = ?error, "TTSのユーザー辞書試聴に失敗しました");
            admin_error(
                StatusCode::BAD_GATEWAY,
                "TTSのユーザー辞書を試聴できませんでした",
            )
        }
        Err(_) => admin_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TTSのユーザー辞書試聴が時間切れになりました",
        ),
    }
}

async fn add_tts_user_dict_word(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsUserDictWordRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if let Err(message) = validate_tts_user_dict_request(&request) {
        return admin_error(StatusCode::BAD_REQUEST, message);
    }
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::add_user_dict_word(&state.http, &request.engine_url, &request.word),
    )
    .await
    {
        Ok(Ok(())) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSのユーザー辞書追加に失敗しました");
            if tts::is_user_dict_input_error(&error) {
                return admin_error(
                    StatusCode::BAD_REQUEST,
                    "単語の読みまたはアクセント位置を確認してください",
                );
            }
            admin_error(
                StatusCode::BAD_GATEWAY,
                "TTSのユーザー辞書へ単語を追加できませんでした",
            )
        }
        Err(_) => admin_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TTSのユーザー辞書追加が時間切れになりました",
        ),
    }
}

async fn update_tts_user_dict_word(
    State(state): State<AppState>,
    Path(word_uuid): Path<String>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsUserDictWordRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if let Err(message) = validate_tts_user_dict_request(&request) {
        return admin_error(StatusCode::BAD_REQUEST, message);
    }
    let Ok(word_uuid) = Uuid::parse_str(&word_uuid) else {
        return admin_error(StatusCode::BAD_REQUEST, "ユーザー辞書の単語IDが不正です");
    };
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::update_user_dict_word(&state.http, &request.engine_url, word_uuid, &request.word),
    )
    .await
    {
        Ok(Ok(())) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSのユーザー辞書更新に失敗しました");
            if tts::is_user_dict_input_error(&error) {
                return admin_error(
                    StatusCode::BAD_REQUEST,
                    "単語の読みまたはアクセント位置を確認してください",
                );
            }
            admin_error(
                StatusCode::BAD_GATEWAY,
                "TTSのユーザー辞書にある単語を更新できませんでした",
            )
        }
        Err(_) => admin_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TTSのユーザー辞書更新が時間切れになりました",
        ),
    }
}

async fn delete_tts_user_dict_word(
    State(state): State<AppState>,
    Path(word_uuid): Path<String>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminTtsEngineRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if validate_http_url("tts.engine_url", &request.engine_url).is_err() {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "TTSの接続先はHTTP(S) URLにしてください",
        );
    }
    let Ok(word_uuid) = Uuid::parse_str(&word_uuid) else {
        return admin_error(StatusCode::BAD_REQUEST, "ユーザー辞書の単語IDが不正です");
    };
    match tokio::time::timeout(
        Duration::from_secs(10),
        tts::delete_user_dict_word(&state.http, &request.engine_url, word_uuid),
    )
    .await
    {
        Ok(Ok(())) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(error)) => {
            tracing::warn!(error = ?error, "TTSのユーザー辞書削除に失敗しました");
            if tts::is_user_dict_input_error(&error) {
                return admin_error(
                    StatusCode::BAD_REQUEST,
                    "削除する単語がユーザー辞書に見つかりません",
                );
            }
            admin_error(
                StatusCode::BAD_GATEWAY,
                "TTSのユーザー辞書から単語を削除できませんでした",
            )
        }
        Err(_) => admin_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TTSのユーザー辞書削除が時間切れになりました",
        ),
    }
}

fn validate_tts_user_dict_request(
    request: &AdminTtsUserDictWordRequest,
) -> Result<(), &'static str> {
    if validate_http_url("tts.engine_url", &request.engine_url).is_err() {
        return Err("TTSの接続先はHTTP(S) URLにしてください");
    }
    if request.word.surface.trim().is_empty() {
        return Err("ユーザー辞書の単語を入力してください");
    }
    if !is_user_dict_pronunciation(&request.word.pronunciation) {
        return Err("ユーザー辞書の読みはカタカナで入力してください");
    }
    if request.word.priority > 10 {
        return Err("ユーザー辞書の優先度は0から10にしてください");
    }
    Ok(())
}

fn is_user_dict_pronunciation(pronunciation: &str) -> bool {
    !pronunciation.is_empty()
        && pronunciation
            .chars()
            .all(|character| ('ァ'..='ヴ').contains(&character) || character == 'ー')
}

async fn handle_websocket(socket: axum::extract::ws::WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    let current = state.current.read().await.clone();
    let history = state.history.lock().await.snapshot();

    if send_json(&mut sender, &ServerEvent::Snapshot { current, history })
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if send_json(&mut sender, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let current = state.current.read().await.clone();
                        let history = state.history.lock().await.snapshot();
                        if send_json(&mut sender, &ServerEvent::Snapshot { current, history }).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn send_json(
    sender: &mut futures_util::stream::SplitSink<axum::extract::ws::WebSocket, Message>,
    event: &ServerEvent,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).expect("ServerEventはJSONへ変換できる");
    sender.send(Message::Text(json.into())).await
}

fn has_valid_admin_token(state: &AppState, auth: &AdminAuth) -> bool {
    let config = state.config.current();
    auth.token.as_deref() == Some(config.admin_token.as_str())
}

#[derive(Deserialize)]
struct AdminAuth {
    token: Option<String>,
}

#[derive(Serialize)]
struct SubmitResponse {
    id: String,
}

#[derive(Serialize)]
struct DisplayConfigDto {
    #[serde(flatten)]
    character: crate::config::CharacterConfig,
    background_image_url: Option<String>,
    background_music_url: Option<String>,
}

#[derive(Deserialize)]
struct BackgroundMusicVolumeRequest {
    volume: f32,
    duck_ratio: f32,
}

#[derive(Serialize)]
struct AdminReloadResponse {
    restart_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdminConfigDto {
    llm: AdminLlmConfigDto,
    tts: AdminTtsConfigDto,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdminLlmConfigDto {
    api_url: String,
    model: String,
    system_prompt: String,
    food_reaction_prompt: String,
    search_fillers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdminTtsConfigDto {
    engine_url: String,
    speaker_id: u32,
}

impl AdminConfigDto {
    fn from_config(config: &crate::config::AppConfig) -> Self {
        Self {
            llm: AdminLlmConfigDto {
                api_url: config.llm.api_url.clone(),
                model: config.llm.model.clone(),
                system_prompt: config.llm.system_prompt.clone(),
                food_reaction_prompt: config.llm.food_reaction_prompt.clone(),
                search_fillers: config.llm.search_fillers.clone(),
            },
            tts: AdminTtsConfigDto {
                engine_url: config.tts.engine_url.clone(),
                speaker_id: config.tts.speaker_id,
            },
        }
    }
}

#[derive(Deserialize)]
struct AdminTtsPreviewRequest {
    tts: AdminTtsConfigDto,
}

#[derive(Deserialize)]
struct AdminTtsSpeakersRequest {
    engine_url: String,
}

#[derive(Serialize)]
struct AdminTtsSpeakersResponse {
    speakers: Vec<tts::Speaker>,
}

#[derive(Deserialize)]
struct AdminTtsEngineRequest {
    engine_url: String,
}

#[derive(Deserialize)]
struct AdminTtsUserDictPreviewRequest {
    tts: AdminTtsConfigDto,
    pronunciation: String,
    accent_type: u32,
}

#[derive(Deserialize)]
struct AdminTtsUserDictWordRequest {
    engine_url: String,
    #[serde(flatten)]
    word: tts::UserDictWordInput,
}

fn admin_no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn admin_error(status: StatusCode, message: &'static str) -> Response {
    admin_no_store((status, Json(serde_json::json!({ "error": message }))).into_response())
}

async fn add_admin_response_headers(response: Response) -> Response {
    admin_no_store(response)
}

struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn unavailable(message: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Arc};

    use axum::{body::to_bytes, http::Request};
    use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        config::{AppConfig, ConfigStore},
        state::{ConversationHistory, SearchFillerRotation},
    };

    fn test_state_with_receiver() -> (AppState, mpsc::Receiver<Submission>) {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.admin_token = "test-token".to_owned();
        let (submissions, receiver) = mpsc::channel(1);
        let (events, _) = broadcast::channel(1);
        (
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
                background_image_lock: Arc::new(Mutex::new(())),
                background_music_lock: Arc::new(Mutex::new(())),
                search_filler_rotation: Arc::new(SearchFillerRotation::default()),
            },
            receiver,
        )
    }

    fn test_state() -> AppState {
        test_state_with_receiver().0
    }

    fn state_with_temporary_assets() -> (AppState, PathBuf) {
        let assets_dir =
            std::env::temp_dir().join(format!("web-aituber-assets-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&assets_dir).unwrap();
        let mut state = test_state();
        state.assets_dir = Arc::new(assets_dir.clone());
        (state, assets_dir)
    }

    fn multipart_image_body(boundary: &str, content_type: &str, image: &[u8]) -> Vec<u8> {
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"background.webp\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(image);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    fn multipart_audio_body(
        boundary: &str,
        file_name: &str,
        content_type: &str,
        audio: &[u8],
    ) -> Vec<u8> {
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(audio);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    fn fake_ffmpeg(directory: &FilePath, succeeds: bool) -> PathBuf {
        let source_path = directory.join("fake-ffmpeg.rs");
        let source = if succeeds {
            r#"fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::fs::copy(&args[5], args.last().unwrap()).unwrap();
}
"#
        } else {
            "fn main() { std::process::exit(1); }\n"
        };
        std::fs::write(&source_path, source).unwrap();
        let path = directory.join(if cfg!(windows) {
            "fake-ffmpeg.exe"
        } else {
            "fake-ffmpeg"
        });
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    fn slow_fake_ffmpeg(directory: &FilePath) -> (PathBuf, PathBuf) {
        let source_path = directory.join("slow-fake-ffmpeg.rs");
        let marker_path = directory.join("slow-ffmpeg-started");
        let source = r#"fn main() {
    let marker = std::env::current_exe().unwrap().with_file_name("slow-ffmpeg-started");
    std::fs::write(marker, b"started").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::fs::copy(&args[5], args.last().unwrap()).unwrap();
}
"#;
        std::fs::write(&source_path, source).unwrap();
        let path = directory.join(if cfg!(windows) {
            "slow-fake-ffmpeg.exe"
        } else {
            "slow-fake-ffmpeg"
        });
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        (path, marker_path)
    }

    fn state_with_temporary_music_config(
        assets_dir: &FilePath,
        ffmpeg_path: &FilePath,
    ) -> (AppState, PathBuf) {
        let mut state = test_state();
        state.assets_dir = Arc::new(assets_dir.to_owned());
        let config_path =
            std::env::temp_dir().join(format!("web-aituber-config-{}.json", Uuid::new_v4()));
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.admin_token = "test-token".to_owned();
        config.ffmpeg_path = ffmpeg_path.to_string_lossy().into_owned();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        (state, config_path)
    }

    fn assert_no_background_music_temporary_files(assets_dir: &FilePath) {
        assert!(!has_background_music_temporary_files(assets_dir));
    }

    fn has_background_music_temporary_files(assets_dir: &FilePath) -> bool {
        std::fs::read_dir(assets_dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".background-music.webm.")
        })
    }

    #[test]
    fn background_music_size_limit_accepts_100_mib_only() {
        assert_eq!(
            checked_background_music_size(0, background_music::MAX_SOURCE_BYTES),
            Some(background_music::MAX_SOURCE_BYTES)
        );
        assert_eq!(
            checked_background_music_size(background_music::MAX_SOURCE_BYTES, 1),
            None
        );
        assert_eq!(checked_background_music_size(usize::MAX, 1), None);
    }

    fn webp_container(payload: &[u8]) -> Vec<u8> {
        let padding = payload.len() % 2;
        let file_size = 20 + payload.len() + padding;
        let mut image = Vec::with_capacity(file_size);
        image.extend_from_slice(b"RIFF");
        image.extend_from_slice(&u32::try_from(file_size - 8).unwrap().to_le_bytes());
        image.extend_from_slice(b"WEBPVP8L");
        image.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        image.extend_from_slice(payload);
        if padding != 0 {
            image.push(0);
        }
        image
    }

    #[tokio::test]
    async fn public_pages_are_available() {
        let app = router(test_state());
        let main = app
            .clone()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(main.status(), StatusCode::OK);

        let input = app
            .clone()
            .oneshot(Request::get("/input").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(input.status(), StatusCode::OK);

        let draw = app
            .oneshot(Request::get("/draw").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(draw.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_page_requires_token() {
        let app = router(test_state());
        let unauthorized = app
            .clone()
            .oneshot(Request::get("/admin").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = router(test_state())
            .oneshot(
                Request::get("/admin?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn config_reload_requires_admin_token_and_succeeds() {
        let unauthorized = router(test_state())
            .oneshot(
                Request::post("/api/admin/reload-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = router(test_state())
            .oneshot(
                Request::post("/api/admin/reload-config?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn admin_config_requires_token_and_never_returns_secrets() {
        let unauthorized = router(test_state())
            .oneshot(
                Request::get("/api/admin/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unauthorized.headers()[header::CACHE_CONTROL], "no-store");

        let response = router(test_state())
            .oneshot(
                Request::get("/api/admin/config?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("food_reaction_prompt"));
        assert!(body.contains("engine_url"));
        assert!(!body.contains("api_key"));
        assert!(!body.contains("test-token"));
    }

    #[tokio::test]
    async fn malformed_admin_json_is_not_cached() {
        let response = router(test_state())
            .oneshot(
                Request::put("/api/admin/config?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status().is_client_error());
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    }

    #[tokio::test]
    async fn admin_config_update_persists_editable_fields_and_keeps_secrets() {
        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.admin_token = "test-token".to_owned();
        let bind = config.bind.clone();
        let character = config.character.clone();
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

        let mut state = test_state();
        state.config = ConfigStore::new(&path, config);
        let mut externally_edited = AppConfig::load_from_path(&path).unwrap();
        externally_edited.llm.api_key = "externally-updated-key".to_owned();
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&externally_edited).unwrap(),
        )
        .unwrap();
        let body = serde_json::json!({
            "llm": {
                "api_url": "https://example.com/v1/responses",
                "model": "updated-model",
                "system_prompt": "更新後の通常プロンプト",
                "food_reaction_prompt": "更新後の食事プロンプト",
                "search_fillers": ["確認します。", "調べます。"]
            },
            "tts": {
                "engine_url": "http://127.0.0.1:50021",
                "speaker_id": 42
            }
        });
        let response = router(state.clone())
            .oneshot(
                Request::put("/api/admin/config?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let saved = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.llm.model, "updated-model");
        assert_eq!(saved.tts.speaker_id, 42);
        assert_eq!(saved.llm.api_key, "externally-updated-key");
        assert_eq!(saved.admin_token, "test-token");
        assert_eq!(saved.bind, bind);
        assert_eq!(saved.character.vrm_url, character.vrm_url);
        assert_eq!(state.config.current().llm.model, "updated-model");

        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn invalid_admin_config_update_keeps_file_and_running_config() {
        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.admin_token = "test-token".to_owned();
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let original = std::fs::read(&path).unwrap();

        let mut state = test_state();
        state.config = ConfigStore::new(&path, config);
        let body = serde_json::json!({
            "llm": {
                "api_url": "ftp://example.com/responses",
                "model": "updated-model",
                "system_prompt": "通常プロンプト",
                "food_reaction_prompt": "食事プロンプト",
                "search_fillers": ["確認します。"]
            },
            "tts": {
                "engine_url": "http://127.0.0.1:50021",
                "speaker_id": 42
            }
        });
        let response = router(state.clone())
            .oneshot(
                Request::put("/api/admin/config?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.config.current().llm.model, "gpt-5.6-luna");
        assert_eq!(std::fs::read(&path).unwrap(), original);

        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn tts_preview_requires_token_and_returns_uncached_wav() {
        async fn audio_query() -> Json<serde_json::Value> {
            Json(serde_json::json!({ "query": "ok" }))
        }
        async fn synthesis() -> Response {
            (
                [(header::CONTENT_TYPE, "audio/wav")],
                b"preview-wav".to_vec(),
            )
                .into_response()
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/audio_query", post(audio_query))
                    .route("/synthesis", post(synthesis)),
            )
            .await
            .unwrap();
        });
        let body = serde_json::json!({
            "tts": { "engine_url": format!("http://{address}"), "speaker_id": 7 }
        })
        .to_string();

        let app = router(test_state());
        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/tts-preview")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::post("/api/admin/tts-preview?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/wav");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let wav = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&wav[..], b"preview-wav");

        server.abort();
    }

    #[tokio::test]
    async fn tts_user_dict_preview_uses_unsaved_pronunciation_and_accent() {
        async fn audio_query(
            Query(query): Query<HashMap<String, String>>,
        ) -> Json<serde_json::Value> {
            assert_eq!(
                query.get("text").map(String::as_str),
                Some("エーアイチューバー")
            );
            assert_eq!(query.get("speaker").map(String::as_str), Some("7"));
            Json(serde_json::json!({
                "accent_phrases": [{
                    "moras": [
                        { "text": "エ" }, { "text": "ー" }, { "text": "ア" },
                        { "text": "イ" }, { "text": "チュ" }, { "text": "ー" },
                        { "text": "バ" }, { "text": "ー" }
                    ],
                    "accent": 1
                }],
                "kana": "エーアイチューバー'",
                "tempoDynamicsScale": 1.2
            }))
        }
        async fn synthesis(Json(query): Json<serde_json::Value>) -> Response {
            assert_eq!(query["accent_phrases"][0]["accent"], 4);
            assert_eq!(query["kana"], "エーアイチューバー'");
            assert_eq!(query["tempoDynamicsScale"], 1.2);
            (
                [(header::CONTENT_TYPE, "audio/wav")],
                b"dictionary-preview-wav".to_vec(),
            )
                .into_response()
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/audio_query", post(audio_query))
                    .route("/synthesis", post(synthesis)),
            )
            .await
            .unwrap();
        });
        let body = serde_json::json!({
            "tts": { "engine_url": format!("http://{address}"), "speaker_id": 7 },
            "pronunciation": "エーアイチューバー",
            "accent_type": 4
        })
        .to_string();
        let app = router(test_state());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/tts-user-dict-preview")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::post("/api/admin/tts-user-dict-preview?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/wav");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let wav = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&wav[..], b"dictionary-preview-wav");

        server.abort();
    }

    #[tokio::test]
    async fn tts_user_dict_preview_rejects_accent_beyond_mora_count() {
        async fn audio_query() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "accent_phrases": [{
                    "moras": [{ "text": "テ" }, { "text": "ス" }, { "text": "ト" }],
                    "accent": 1
                }]
            }))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/audio_query", post(audio_query)),
            )
            .await
            .unwrap();
        });
        let body = serde_json::json!({
            "tts": { "engine_url": format!("http://{address}"), "speaker_id": 7 },
            "pronunciation": "テスト",
            "accent_type": 4
        });

        let response = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-user-dict-preview?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(
            String::from_utf8(body.to_vec())
                .unwrap()
                .contains("アクセント位置")
        );

        server.abort();
    }

    #[tokio::test]
    async fn tts_speakers_requires_token_and_flattens_engine_response() {
        async fn speakers() -> Json<serde_json::Value> {
            Json(serde_json::json!([
                {
                    "name": "話者A",
                    "styles": [
                        { "id": 1, "name": "通常" },
                        { "id": 2, "name": "喜び" }
                    ]
                },
                { "name": "話者B", "styles": [{ "id": 3, "name": "落ち着き" }] }
            ]))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/speakers", get(speakers)))
                .await
                .unwrap();
        });
        let body = serde_json::json!({ "engine_url": format!("http://{address}") }).to_string();
        let app = router(test_state());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/tts-speakers")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unauthorized.headers()[header::CACHE_CONTROL], "no-store");

        let response = app
            .oneshot(
                Request::post("/api/admin/tts-speakers?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({ "speakers": [
                { "id": 1, "speaker_name": "話者A", "style_name": "通常" },
                { "id": 2, "speaker_name": "話者A", "style_name": "喜び" },
                { "id": 3, "speaker_name": "話者B", "style_name": "落ち着き" }
            ]})
        );

        server.abort();
    }

    #[tokio::test]
    async fn tts_speakers_rejects_invalid_url_and_hides_engine_failure_detail() {
        let invalid = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-speakers?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"engine_url":"ftp://example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(invalid.headers()[header::CACHE_CONTROL], "no-store");

        async fn unavailable() -> StatusCode {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/speakers", get(unavailable)))
                .await
                .unwrap();
        });
        let engine_url = format!("http://{address}");
        let response = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-speakers?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "engine_url": engine_url }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("127.0.0.1"));
        assert!(!body.contains("500"));

        server.abort();
    }

    #[tokio::test]
    async fn tts_user_dict_crud_uses_common_engine_contract() {
        async fn user_dict(
            Query(query): Query<HashMap<String, String>>,
        ) -> Json<serde_json::Value> {
            assert_eq!(
                query.get("enable_compound_accent").map(String::as_str),
                Some("true")
            );
            Json(serde_json::json!({
                "00000000-0000-0000-0000-000000000001": {
                    "surface": "AITuber",
                    "pronunciation": "エーアイチューバー",
                    "accent_type": 0,
                    "priority": 5,
                    "context_id": 1348
                },
                "00000000-0000-0000-0000-000000000002": {
                    "surface": "東京",
                    "pronunciation": ["トーキョー"],
                    "accent_type": [0],
                    "priority": 5,
                    "context_id": 1348,
                    "word_type": "LOCATION_NAME"
                },
                "00000000-0000-0000-0000-000000000003": {
                    "surface": "新田真剣佑",
                    "pronunciation": ["アラタ", "マッケンユウ"],
                    "accent_type": [1, 3],
                    "priority": 5,
                    "context_id": 1348,
                    "word_type": "PROPER_NOUN"
                }
            }))
        }
        async fn write_word(Query(query): Query<HashMap<String, String>>) -> StatusCode {
            assert_eq!(query.len(), 5);
            assert_eq!(query.get("surface").map(String::as_str), Some("OpenAI"));
            assert_eq!(
                query.get("pronunciation").map(String::as_str),
                Some("オープンエーアイ")
            );
            assert_eq!(query.get("accent_type").map(String::as_str), Some("4"));
            assert_eq!(
                query.get("word_type").map(String::as_str),
                Some("PROPER_NOUN")
            );
            assert_eq!(query.get("priority").map(String::as_str), Some("7"));
            StatusCode::NO_CONTENT
        }
        async fn delete_word(Path(word_uuid): Path<String>) -> StatusCode {
            assert_eq!(
                Uuid::parse_str(&word_uuid).unwrap(),
                Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap()
            );
            StatusCode::NO_CONTENT
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/user_dict", get(user_dict))
                    .route("/user_dict_word", post(write_word))
                    .route(
                        "/user_dict_word/{word_uuid}",
                        axum::routing::put(write_word).delete(delete_word),
                    ),
            )
            .await
            .unwrap();
        });
        let engine_url = format!("http://{address}");
        let engine_body = serde_json::json!({ "engine_url": engine_url }).to_string();
        let word_body = serde_json::json!({
            "engine_url": format!("http://{address}"),
            "surface": "OpenAI",
            "pronunciation": "オープンエーアイ",
            "accent_type": 4,
            "word_type": "PROPER_NOUN",
            "priority": 7
        })
        .to_string();
        let app = router(test_state());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/tts-user-dict")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(engine_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let list = app
            .clone()
            .oneshot(
                Request::post("/api/admin/tts-user-dict?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(engine_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers()[header::CACHE_CONTROL], "no-store");
        let list_body = to_bytes(list.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&list_body).unwrap(),
            serde_json::json!({
                "words": [{
                    "uuid": "00000000-0000-0000-0000-000000000001",
                    "surface": "AITuber",
                    "pronunciation": "エーアイチューバー",
                    "accent_type": 0,
                    "word_type": "PROPER_NOUN",
                    "priority": 5
                }],
                "has_excluded_words": true
            })
        );

        for (method, path, body) in [
            (
                "POST",
                "/api/admin/tts-user-dict-word?token=test-token",
                word_body.clone(),
            ),
            (
                "PUT",
                "/api/admin/tts-user-dict-word/00000000-0000-0000-0000-000000000004?token=test-token",
                word_body,
            ),
            (
                "DELETE",
                "/api/admin/tts-user-dict-word/00000000-0000-0000-0000-000000000004?token=test-token",
                engine_body,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }

        server.abort();
    }

    #[tokio::test]
    async fn tts_user_dict_rejects_invalid_url_and_word() {
        let invalid_url = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-user-dict?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"engine_url":"ftp://example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_url.status(), StatusCode::BAD_REQUEST);

        let invalid_word = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-user-dict-word?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "engine_url": "http://127.0.0.1:50021",
                            "surface": "AITuber",
                            "pronunciation": "えーあいちゅーばー",
                            "accent_type": 0,
                            "word_type": "PROPER_NOUN",
                            "priority": 5
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_word.status(), StatusCode::BAD_REQUEST);
        assert_eq!(invalid_word.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn tts_user_dict_maps_engine_input_error_to_bad_request() {
        async fn invalid_word() -> (StatusCode, &'static str) {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "engine-internal-validation-detail",
            )
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/user_dict_word", post(invalid_word)),
            )
            .await
            .unwrap();
        });
        let body = serde_json::json!({
            "engine_url": format!("http://{address}"),
            "surface": "AITuber",
            "pronunciation": "エーアイチューバー",
            "accent_type": 99,
            "word_type": "PROPER_NOUN",
            "priority": 5
        });

        let response = router(test_state())
            .oneshot(
                Request::post("/api/admin/tts-user-dict-word?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("アクセント位置"));
        assert!(!body.contains("engine-internal-validation-detail"));

        server.abort();
    }

    #[tokio::test]
    async fn display_config_is_public_and_does_not_expose_secrets() {
        let response = router(test_state())
            .oneshot(
                Request::get("/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("api_key"));
        assert!(!body.contains("test-token"));
        assert!(body.contains("vrm_url"));
    }

    #[tokio::test]
    async fn background_image_upload_requires_token_and_atomically_replaces_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
        std::fs::write(&path, b"old-image").unwrap();
        let boundary = "background-upload-boundary";
        let image = webp_container(b"new-image");
        let body = multipart_image_body(boundary, "image/webp", &image);
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/background-image")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(std::fs::read(&path).unwrap(), b"old-image");

        let response = app
            .oneshot(
                Request::post("/api/admin/background-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(std::fs::read(&path).unwrap(), image);
        assert!(std::fs::read_dir(&assets_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_image_upload_creates_initial_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
        let boundary = "initial-background-boundary";
        let image = webp_container(b"initial-image");
        let body = multipart_image_body(boundary, "image/webp", &image);

        let response = router(state)
            .oneshot(
                Request::post("/api/admin/background-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(&path).unwrap(), image);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn invalid_background_image_does_not_replace_current_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
        std::fs::write(&path, b"current-image").unwrap();
        let boundary = "invalid-background-boundary";
        let body = multipart_image_body(boundary, "image/webp", b"not-a-webp");

        let response = router(state)
            .oneshot(
                Request::post("/api/admin/background-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(std::fs::read(&path).unwrap(), b"current-image");

        let boundary = "wrong-mime-background-boundary";
        let body = multipart_image_body(boundary, "image/png", &webp_container(b"valid-container"));
        let mut state = test_state();
        state.assets_dir = Arc::new(assets_dir.clone());
        let response = router(state)
            .oneshot(
                Request::post("/api/admin/background-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&path).unwrap(), b"current-image");

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn oversized_background_image_does_not_replace_current_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
        std::fs::write(&path, b"current-image").unwrap();
        let boundary = "oversized-background-boundary";
        let mut image = vec![0_u8; MAX_IMAGE_BYTES + 1];
        let image_len = image.len();
        image[..4].copy_from_slice(b"RIFF");
        image[4..8].copy_from_slice(&u32::try_from(image_len - 8).unwrap().to_le_bytes());
        image[8..12].copy_from_slice(b"WEBP");
        image[12..16].copy_from_slice(b"VP8L");
        image[16..20].copy_from_slice(&u32::try_from(image_len - 20).unwrap().to_le_bytes());
        let body = multipart_image_body(boundary, "image/webp", &image);

        let response = router(state)
            .oneshot(
                Request::post("/api/admin/background-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&path).unwrap(), b"current-image");

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_image_delete_is_authenticated_and_idempotent() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
        std::fs::write(&path, b"background").unwrap();
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::delete("/api/admin/background-image")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(path.exists());

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::delete("/api/admin/background-image?token=test-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        assert!(!path.exists());

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn display_config_returns_fresh_background_url_only_when_image_exists() {
        let (state, assets_dir) = state_with_temporary_assets();
        let app = router(state);

        let missing = app
            .clone()
            .oneshot(
                Request::get("/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.headers()[header::CACHE_CONTROL], "no-store");
        let missing_body = to_bytes(missing.into_body(), 64 * 1024).await.unwrap();
        let missing_json: serde_json::Value = serde_json::from_slice(&missing_body).unwrap();
        assert_eq!(
            missing_json["background_image_url"],
            serde_json::Value::Null
        );

        std::fs::write(assets_dir.join(BACKGROUND_IMAGE_FILE_NAME), b"background").unwrap();
        let mut urls = Vec::new();
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::get("/api/display-config")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let url = json["background_image_url"].as_str().unwrap().to_owned();
            assert!(url.starts_with("/assets/background.webp?v="));
            urls.push(url);
        }
        assert_ne!(urls[0], urls[1]);

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_music_upload_requires_token_and_creates_then_replaces_file() {
        let (_base_state, assets_dir) = state_with_temporary_assets();
        let ffmpeg_path = fake_ffmpeg(&assets_dir, true);
        let (state, config_path) = state_with_temporary_music_config(&assets_dir, &ffmpeg_path);
        let destination = assets_dir.join(background_music::FILE_NAME);
        let boundary = "background-music-upload";
        let first = multipart_audio_body(boundary, "music.WAV", "audio/wav", b"first-audio");
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/background-music")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(first.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(!destination.exists());

        let created = app
            .clone()
            .oneshot(
                Request::post("/api/admin/background-music?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(first))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::NO_CONTENT);
        assert_eq!(created.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(std::fs::read(&destination).unwrap(), b"first-audio");

        let second = multipart_audio_body(
            boundary,
            "music.oGg",
            "application/octet-stream",
            b"second-audio",
        );
        let replaced = app
            .oneshot(
                Request::post("/api/admin/background-music?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(second))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replaced.status(), StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(&destination).unwrap(), b"second-audio");
        assert_no_background_music_temporary_files(&assets_dir);

        std::fs::remove_file(config_path).unwrap();
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_music_request_cancellation_waits_for_conversion_and_cleans_up() {
        let (_base_state, assets_dir) = state_with_temporary_assets();
        let (ffmpeg_path, marker_path) = slow_fake_ffmpeg(&assets_dir);
        let (state, config_path) = state_with_temporary_music_config(&assets_dir, &ffmpeg_path);
        let lock = state.background_music_lock.clone();
        let destination = assets_dir.join(background_music::FILE_NAME);
        let boundary = "background-music-cancel";
        let request = Request::post("/api/admin/background-music?token=test-token")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(multipart_audio_body(
                boundary,
                "music.wav",
                "audio/wav",
                b"audio",
            )))
            .unwrap();
        let upload = tokio::spawn(router(state).oneshot(request));

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !marker_path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        upload.abort();
        let _ = upload.await;
        assert!(lock.try_lock().is_err());

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while has_background_music_temporary_files(&assets_dir) || lock.try_lock().is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!destination.exists());
        assert_no_background_music_temporary_files(&assets_dir);

        std::fs::remove_file(config_path).unwrap();
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_music_rejects_unsupported_extension_and_failed_conversion_keeps_current_file()
     {
        let (_base_state, assets_dir) = state_with_temporary_assets();
        let ffmpeg_path = fake_ffmpeg(&assets_dir, false);
        let (state, config_path) = state_with_temporary_music_config(&assets_dir, &ffmpeg_path);
        let destination = assets_dir.join(background_music::FILE_NAME);
        std::fs::write(&destination, b"current-music").unwrap();
        let boundary = "background-music-invalid";
        let app = router(state);

        let unsupported = app
            .clone()
            .oneshot(
                Request::post("/api/admin/background-music?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_audio_body(
                        boundary,
                        "music.webm",
                        "audio/webm",
                        b"unsupported",
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&destination).unwrap(), b"current-music");

        let failed = app
            .oneshot(
                Request::post("/api/admin/background-music?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_audio_body(
                        boundary,
                        "music.mp3",
                        "text/plain",
                        b"invalid-audio",
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&destination).unwrap(), b"current-music");
        assert_no_background_music_temporary_files(&assets_dir);

        std::fs::remove_file(config_path).unwrap();
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_music_delete_is_authenticated_and_idempotent() {
        let (state, assets_dir) = state_with_temporary_assets();
        let destination = assets_dir.join(background_music::FILE_NAME);
        std::fs::write(&destination, b"music").unwrap();
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::delete("/api/admin/background-music")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(destination.exists());

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::delete("/api/admin/background-music?token=test-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        assert!(!destination.exists());
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn display_config_returns_music_volume_and_fresh_url() {
        let (state, assets_dir) = state_with_temporary_assets();
        let app = router(state);

        let missing = app
            .clone()
            .oneshot(
                Request::get("/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(missing.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["background_music_volume"], 0.3);
        assert_eq!(json["background_music_duck_ratio"], 0.4);
        assert_eq!(json["background_music_url"], serde_json::Value::Null);

        std::fs::write(assets_dir.join(background_music::FILE_NAME), b"music").unwrap();
        let mut urls = Vec::new();
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::get("/api/display-config")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let url = json["background_music_url"].as_str().unwrap().to_owned();
            assert!(url.starts_with("/assets/background-music.webm?v="));
            urls.push(url);
        }
        assert_ne!(urls[0], urls[1]);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn background_music_volume_update_validates_ranges_and_preserves_other_settings() {
        let assets_dir =
            std::env::temp_dir().join(format!("web-aituber-assets-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&assets_dir).unwrap();
        let ffmpeg_path = fake_ffmpeg(&assets_dir, true);
        let (state, config_path) = state_with_temporary_music_config(&assets_dir, &ffmpeg_path);
        let app = router(state.clone());

        let mut externally_updated = AppConfig::load_from_path(&config_path).unwrap();
        externally_updated.llm.model = "externally-updated-model".to_owned();
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&externally_updated).unwrap(),
        )
        .unwrap();

        let unauthorized = app
            .clone()
            .oneshot(
                Request::put("/api/admin/background-music-volume")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"volume":0.7,"duck_ratio":0.6}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        for invalid in [
            r#"{"volume":-0.1,"duck_ratio":0.4}"#,
            r#"{"volume":1.1,"duck_ratio":0.4}"#,
            r#"{"volume":0.3,"duck_ratio":-0.1}"#,
            r#"{"volume":0.3,"duck_ratio":1.1}"#,
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::put("/api/admin/background-music-volume?token=test-token")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(invalid))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        for (volume, duck_ratio) in [(0.0, 0.0), (1.0, 1.0), (0.7, 0.6)] {
            let response = app
                .clone()
                .oneshot(
                    Request::put("/api/admin/background-music-volume?token=test-token")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(
                            r#"{{"volume":{volume},"duck_ratio":{duck_ratio}}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }

        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.character.background_music_volume, 0.7);
        assert_eq!(saved.character.background_music_duck_ratio, 0.6);
        assert_eq!(saved.llm.model, "externally-updated-model");
        assert_eq!(state.config.current().llm.model, "externally-updated-model");
        assert_eq!(
            state.config.current().character.background_music_volume,
            0.7
        );
        assert_eq!(
            state.config.current().character.background_music_duck_ratio,
            0.6
        );
        std::fs::remove_file(config_path).unwrap();
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn admin_skip_requires_token_and_cancels_matching_turn() {
        let state = test_state();
        let cancel = CancellationToken::new();
        *state.active.lock().await = Some(crate::state::ActiveTurn {
            turn_id: "turn-1".to_owned(),
            cancel: cancel.clone(),
        });
        let app = router(state);
        let request_body = r#"{"turn_id":"turn-1"}"#;

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/skip")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(!cancel.is_cancelled());

        let authorized = app
            .oneshot(
                Request::post("/api/admin/skip?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::NO_CONTENT);
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn regular_submission_is_text_only() {
        let boundary = "test-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"text\"\r\n\r\n\
             テキストだけの質問\r\n\
             --{boundary}--\r\n"
        );
        let (state, mut submissions) = test_state_with_receiver();

        let response = router(state)
            .oneshot(
                Request::post("/api/submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let submission = submissions.recv().await.unwrap();
        assert!(matches!(submission.kind, SubmissionKind::Question));
        assert_eq!(submission.text, "テキストだけの質問");
    }

    #[tokio::test]
    async fn regular_submission_rejects_an_image_field() {
        let boundary = "question-image-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"text\"\r\n\r\n\
             画像付きの質問\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"image\"; filename=\"image.webp\"\r\n\
             Content-Type: image/webp\r\n\r\n\
             image-data\r\n\
             --{boundary}--\r\n"
        );

        let response = router(test_state())
            .oneshot(
                Request::post("/api/submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn food_submission_requires_both_images_and_uses_food_kind() {
        let missing_boundary = "missing-food-boundary";
        let missing = router(test_state())
            .oneshot(
                Request::post("/api/food-submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={missing_boundary}"),
                    )
                    .body(Body::from(format!("--{missing_boundary}--\r\n")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

        let vrm_only_boundary = "vrm-only-boundary";
        let vrm_only_body = format!(
            "--{vrm_only_boundary}\r\n\
             Content-Disposition: form-data; name=\"vrm_image\"; filename=\"food-vrm.webp\"\r\n\
             Content-Type: image/webp\r\n\r\n\
             vrm-image\r\n\
             --{vrm_only_boundary}--\r\n"
        );
        let vrm_only = router(test_state())
            .oneshot(
                Request::post("/api/food-submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={vrm_only_boundary}"),
                    )
                    .body(Body::from(vrm_only_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(vrm_only.status(), StatusCode::BAD_REQUEST);

        let ai_only_boundary = "ai-only-boundary";
        let ai_only_body = format!(
            "--{ai_only_boundary}\r\n\
             Content-Disposition: form-data; name=\"ai_image\"; filename=\"food-ai.webp\"\r\n\
             Content-Type: image/webp\r\n\r\n\
             ai-image\r\n\
             --{ai_only_boundary}--\r\n"
        );
        let ai_only = router(test_state())
            .oneshot(
                Request::post("/api/food-submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={ai_only_boundary}"),
                    )
                    .body(Body::from(ai_only_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ai_only.status(), StatusCode::BAD_REQUEST);

        let boundary = "food-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"vrm_image\"; filename=\"food-vrm.webp\"\r\n\
             Content-Type: image/webp\r\n\r\n\
             vrm-image\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"ai_image\"; filename=\"food-ai.webp\"\r\n\
             Content-Type: image/webp\r\n\r\n\
             ai-image\r\n\
             --{boundary}--\r\n"
        );
        let (state, mut submissions) = test_state_with_receiver();

        let response = router(state)
            .oneshot(
                Request::post("/api/food-submissions")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let submission = submissions.recv().await.unwrap();
        assert_eq!(submission.text, "食べ物の絵を送りました");
        let SubmissionKind::Food {
            vrm_image,
            ai_image,
        } = submission.kind
        else {
            panic!("食事投稿として受け付けられていません");
        };
        assert_eq!(vrm_image.mime_type, "image/webp");
        assert_eq!(vrm_image.data, b"vrm-image");
        assert_eq!(ai_image.mime_type, "image/webp");
        assert_eq!(ai_image.data, b"ai-image");
    }

    #[tokio::test]
    async fn temporary_food_image_is_public_and_not_cached() {
        let state = test_state();
        state.food_images.write().await.insert(
            "turn-1".to_owned(),
            InputImage {
                mime_type: "image/webp".to_owned(),
                data: b"image-data".to_vec(),
            },
        );

        let response = router(state)
            .oneshot(
                Request::get("/food-images/turn-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        assert_eq!(body.as_ref(), b"image-data");
    }
}
