use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, FromRequestParts, Multipart, Path, Query, State, WebSocketUpgrade,
        multipart::Field, ws::Message,
    },
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Component, Path as FilePath, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;
use tower_http::{services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{
    background_music,
    config::{
        CharacterConfig, TtsConfig, validate_event_identifier, validate_http_url,
        validate_public_base_url,
    },
    protocol::{AdminSkipRequest, InputImage, ServerEvent, Submission, SubmissionKind},
    state::AppState,
    tts, update,
};

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_VRM_MODEL_BYTES: usize = 100 * 1024 * 1024;
const MAX_TEXT_REQUEST_BYTES: usize = 128 * 1024;
const MAX_FOOD_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;
const MAX_BACKGROUND_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;
const MAX_SCREEN_OVERLAY_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;
const MAX_VRM_MODEL_REQUEST_BYTES: usize = MAX_VRM_MODEL_BYTES + 128 * 1024;
const MAX_BACKGROUND_MUSIC_REQUEST_BYTES: usize = background_music::MAX_SOURCE_BYTES + 128 * 1024;
const VRM_MODEL_FILE_NAME: &str = "model.vrm";
const BACKGROUND_IMAGE_FILE_NAME: &str = "background.webp";
const PREPARATION_IMAGE_FILE_NAME: &str = "preparation.webp";
const UPDATE_SHUTDOWN_DELAY: Duration = Duration::from_millis(750);

pub fn router(state: AppState) -> Router {
    let asset_routes = Router::new()
        .nest_service("/assets", ServeDir::new(state.assets_dir.as_ref()))
        .layer(middleware::from_fn(add_asset_cache_headers));
    let admin_api = Router::new()
        .route("/api/admin/skip", post(skip))
        .route(
            "/api/admin/conversation-history",
            axum::routing::delete(clear_conversation_history),
        )
        .route("/api/admin/reload-config", post(reload_config))
        .route("/api/admin/version", get(admin_version))
        .route("/api/admin/update", get(check_update).post(apply_update))
        .route(
            "/api/admin/event-access",
            get(admin_event_access).put(update_admin_event_access),
        )
        .route("/api/admin/qr-code", post(admin_qr_code))
        .route("/api/admin/display-config", get(admin_display_config))
        .route(
            "/api/admin/preparation-mode",
            axum::routing::put(update_preparation_mode),
        )
        .route(
            "/api/admin/model-brightness",
            axum::routing::put(update_model_brightness),
        )
        .route(
            "/api/admin/model-antialias",
            axum::routing::put(update_model_antialias),
        )
        .route(
            "/api/admin/drawing-stabilization",
            axum::routing::put(update_drawing_stabilization),
        )
        .route(
            "/api/admin/model-layout",
            axum::routing::put(update_model_layout),
        )
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
            "/api/admin/vrm-model",
            post(upload_vrm_model).layer(DefaultBodyLimit::max(MAX_VRM_MODEL_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/background-image",
            post(upload_background_image)
                .delete(delete_background_image)
                .layer(DefaultBodyLimit::max(MAX_BACKGROUND_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/preparation-image",
            post(upload_preparation_image)
                .delete(delete_preparation_image)
                .layer(DefaultBodyLimit::max(MAX_BACKGROUND_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/screen-overlays/{slot}",
            post(upload_screen_overlay)
                .delete(delete_screen_overlay)
                .layer(DefaultBodyLimit::max(MAX_SCREEN_OVERLAY_REQUEST_BYTES)),
        )
        .route(
            "/api/admin/screen-overlays/{slot}/scale",
            axum::routing::put(update_screen_overlay_scale),
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

    let public_event = Router::new()
        .route("/event/{event_identifier}", get(main_page))
        .route("/event/{event_identifier}/input", get(input_page))
        .route("/event/{event_identifier}/draw", get(draw_page))
        .route(
            "/event/{event_identifier}/api/submissions",
            post(submit).layer(DefaultBodyLimit::max(MAX_TEXT_REQUEST_BYTES)),
        )
        .route(
            "/event/{event_identifier}/api/food-submissions",
            post(submit_food).layer(DefaultBodyLimit::max(MAX_FOOD_REQUEST_BYTES)),
        )
        .route(
            "/event/{event_identifier}/api/display-config",
            get(event_display_config),
        )
        .route("/event/{event_identifier}/ws", get(event_websocket));

    Router::new()
        .route("/admin", get(admin_page))
        .merge(public_event)
        .merge(admin_api)
        .route("/ws", get(admin_websocket))
        .route("/food-images/{id}", get(food_image))
        .nest_service("/static", ServeDir::new("web"))
        .merge(asset_routes)
        .nest_service("/audio", ServeDir::new(state.audio_dir.as_ref()))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn main_page(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !has_valid_event_identifier(&state, &event_identifier) {
        return invalid_event_page().await;
    }
    html_file("web/main.html").await
}

async fn input_page(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !has_valid_event_identifier(&state, &event_identifier) {
        return invalid_event_page().await;
    }
    html_file("web/input.html").await
}

async fn draw_page(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !has_valid_event_identifier(&state, &event_identifier) {
        return invalid_event_page().await;
    }
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

async fn invalid_event_page() -> Response {
    let mut response = html_file("web/invalid-event.html").await;
    if response.status().is_success() {
        *response.status_mut() = StatusCode::NOT_FOUND;
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn submit(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmitResponse>), ApiError> {
    require_accepting_submissions(&state, &event_identifier)?;
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
    require_accepting_submissions(&state, &event_identifier)?;

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
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmitResponse>), ApiError> {
    require_accepting_submissions(&state, &event_identifier)?;
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
    require_accepting_submissions(&state, &event_identifier)?;
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

fn notify_display_config_changed(state: &AppState) {
    let _ = state.events.send(ServerEvent::DisplayConfigChanged);
}

fn notify_event_access_changed(state: &AppState) {
    let _ = state.events.send(ServerEvent::EventAccessChanged);
}

async fn event_display_config(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !has_valid_event_identifier(&state, &event_identifier) {
        return event_ended_response();
    }
    display_config_response(&state).await
}

async fn admin_display_config(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    admin_no_store(display_config_response(&state).await)
}

async fn display_config_response(state: &AppState) -> Response {
    let config = state.config.current();
    let background_image_path = state.assets_dir.join(BACKGROUND_IMAGE_FILE_NAME);
    let background_image_url = existing_versioned_asset_url(
        &background_image_path,
        &format!("/assets/{BACKGROUND_IMAGE_FILE_NAME}"),
        "背景画像",
    )
    .await;
    let preparation_image_path = state.assets_dir.join(PREPARATION_IMAGE_FILE_NAME);
    let preparation_image_url = existing_versioned_asset_url(
        &preparation_image_path,
        &format!("/assets/{PREPARATION_IMAGE_FILE_NAME}"),
        "準備中画像",
    )
    .await;
    let background_music_path = state.assets_dir.join(background_music::FILE_NAME);
    let background_music_url = existing_versioned_asset_url(
        &background_music_path,
        &format!("/assets/{}", background_music::FILE_NAME),
        "BGM",
    )
    .await;
    let character =
        version_character_asset_urls(config.character.clone(), state.assets_dir.as_ref()).await;
    let response = DisplayConfigDto {
        screen_overlays: screen_overlays_display_config(state, &character).await,
        drawing: config.drawing.clone(),
        character: DisplayCharacterConfig::from(character),
        preparation_image_url,
        background_image_url,
        background_music_url,
    };
    no_store(Json(response).into_response())
}

async fn screen_overlays_display_config(
    state: &AppState,
    character: &CharacterConfig,
) -> ScreenOverlaysDisplayConfigDto {
    async fn slot(
        state: &AppState,
        slot: ScreenOverlaySlot,
        scale: u8,
    ) -> ScreenOverlayDisplayConfigDto {
        let file_name = slot.file_name();
        let image_url = existing_versioned_asset_url(
            &state.assets_dir.join(file_name),
            &format!("/assets/{file_name}"),
            "画面オーバーレイ",
        )
        .await;
        ScreenOverlayDisplayConfigDto { image_url, scale }
    }

    ScreenOverlaysDisplayConfigDto {
        top_left: slot(
            state,
            ScreenOverlaySlot::TopLeft,
            character.screen_overlays.top_left.scale,
        )
        .await,
        top_right: slot(
            state,
            ScreenOverlaySlot::TopRight,
            character.screen_overlays.top_right.scale,
        )
        .await,
        bottom_left: slot(
            state,
            ScreenOverlaySlot::BottomLeft,
            character.screen_overlays.bottom_left.scale,
        )
        .await,
        bottom_right: slot(
            state,
            ScreenOverlaySlot::BottomRight,
            character.screen_overlays.bottom_right.scale,
        )
        .await,
    }
}

async fn add_asset_cache_headers(
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let is_versioned = request.uri().query().is_some_and(|query| {
        query.split('&').any(|parameter| {
            parameter
                .strip_prefix("v=")
                .is_some_and(|value| !value.is_empty())
        })
    });
    let mut response = next.run(request).await;
    let can_cache_immutably =
        response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if is_versioned && can_cache_immutably {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=0, must-revalidate"
        }),
    );
    response
}

async fn version_character_asset_urls(
    mut character: CharacterConfig,
    assets_dir: &FilePath,
) -> CharacterConfig {
    character.vrm_url = version_local_asset_url(assets_dir, &character.vrm_url).await;
    for url in &mut character.idle_motions {
        *url = version_local_asset_url(assets_dir, url).await;
    }
    for url in character.emotion_motions.values_mut() {
        *url = version_local_asset_url(assets_dir, url).await;
    }
    character
}

async fn version_local_asset_url(assets_dir: &FilePath, url: &str) -> String {
    let Some(path) = local_asset_path(assets_dir, url) else {
        return url.to_owned();
    };
    match tokio::fs::metadata(path).await {
        Ok(metadata) => asset_version(&metadata)
            .map(|version| append_asset_version(url, &version))
            .unwrap_or_else(|| url.to_owned()),
        Err(_) => url.to_owned(),
    }
}

async fn existing_versioned_asset_url(
    path: &FilePath,
    url: &str,
    asset_name: &str,
) -> Option<String> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Some(
            asset_version(&metadata)
                .map(|version| append_asset_version(url, &version))
                .unwrap_or_else(|| url.to_owned()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(path = %path.display(), error = ?error, asset_name, "表示アセットを確認できませんでした");
            None
        }
    }
}

fn local_asset_path(assets_dir: &FilePath, url: &str) -> Option<PathBuf> {
    let path = url.split(['?', '#']).next()?;
    let relative = path.strip_prefix("/assets/")?;
    let relative = FilePath::new(relative);
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(assets_dir.join(relative))
}

fn asset_version(metadata: &fs::Metadata) -> Option<String> {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())?
        .as_nanos();
    Some(format!("{:x}-{modified:x}", metadata.len()))
}

fn append_asset_version(url: &str, version: &str) -> String {
    let (url, fragment) = url
        .split_once('#')
        .map_or((url, None), |(url, fragment)| (url, Some(fragment)));
    let separator = if url.contains('?') { '&' } else { '?' };
    let fragment = fragment.map_or_else(String::new, |fragment| format!("#{fragment}"));
    format!("{url}{separator}v={version}{fragment}")
}

async fn upload_vrm_model(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    mut multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if state.config.current().character.vrm_url != "/assets/model.vrm" {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "character.vrm_urlを/assets/model.vrmに設定してください",
        );
    }

    let mut model = None;
    while let Some(field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(_) => return admin_error(StatusCode::BAD_REQUEST, "VRMモデルを読み取れませんでした"),
    } {
        if field.name() != Some("model") || model.is_some() {
            continue;
        }
        let is_vrm_file = field
            .file_name()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".vrm"));
        if !is_vrm_file {
            return admin_error(StatusCode::BAD_REQUEST, ".vrmファイルを選択してください");
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return admin_error(StatusCode::BAD_REQUEST, "VRMモデルを読み取れませんでした");
            }
        };
        if bytes.len() > MAX_VRM_MODEL_BYTES {
            return admin_error(
                StatusCode::BAD_REQUEST,
                "VRMモデルは100MiB以下にしてください",
            );
        }
        if !has_valid_vrm_model(&bytes) {
            return admin_error(StatusCode::BAD_REQUEST, "VRMモデルの形式が不正です");
        }
        model = Some(bytes);
    }

    let Some(model) = model else {
        return admin_error(StatusCode::BAD_REQUEST, "VRMモデルがありません");
    };
    let _guard = state.vrm_model_lock.lock().await;
    let path = state.assets_dir.join(VRM_MODEL_FILE_NAME);
    match tokio::task::spawn_blocking(move || write_file_atomically(&path, &model)).await {
        Ok(Ok(())) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Ok(Err(error)) => {
            tracing::error!(error = ?error, "VRMモデルを保存できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "VRMモデルを保存できませんでした",
            )
        }
        Err(error) => {
            tracing::error!(error = ?error, "VRMモデルの保存処理を実行できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "VRMモデルを保存できませんでした",
            )
        }
    }
}

async fn upload_background_image(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let image = match read_webp_image(multipart, "背景画像").await {
        Ok(image) => image,
        Err(response) => return response,
    };
    save_webp_image(
        &state,
        &state.background_image_lock,
        BACKGROUND_IMAGE_FILE_NAME,
        "背景画像",
        image,
    )
    .await
}

async fn upload_preparation_image(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let image = match read_webp_image(multipart, "準備中画像").await {
        Ok(image) => image,
        Err(response) => return response,
    };
    save_webp_image(
        &state,
        &state.preparation_image_lock,
        PREPARATION_IMAGE_FILE_NAME,
        "準備中画像",
        image,
    )
    .await
}

async fn save_webp_image(
    state: &AppState,
    lock: &tokio::sync::Mutex<()>,
    file_name: &str,
    label: &'static str,
    image: Bytes,
) -> Response {
    let _guard = lock.lock().await;
    let path = state.assets_dir.join(file_name);
    match tokio::task::spawn_blocking(move || write_file_atomically(&path, &image)).await {
        Ok(Ok(())) => {
            notify_display_config_changed(state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Ok(Err(error)) => {
            tracing::error!(asset = label, error = ?error, "画像を保存できませんでした");
            admin_error_owned(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{label}を保存できませんでした"),
            )
        }
        Err(error) => {
            tracing::error!(asset = label, error = ?error, "画像の保存処理を実行できませんでした");
            admin_error_owned(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{label}を保存できませんでした"),
            )
        }
    }
}

async fn delete_preparation_image(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let _guard = state.preparation_image_lock.lock().await;
    if state.config.current().character.preparation_mode {
        return admin_error(
            StatusCode::CONFLICT,
            "準備中モードをOFFにしてから画像を削除してください",
        );
    }
    let path = state.assets_dir.join(PREPARATION_IMAGE_FILE_NAME);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::error!(path = %path.display(), error = ?error, "準備中画像を削除できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "準備中画像を削除できませんでした",
            )
        }
    }
}

async fn read_webp_image(mut multipart: Multipart, label: &str) -> Result<Bytes, Response> {
    let mut image = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        admin_error_owned(
            StatusCode::BAD_REQUEST,
            format!("{label}を読み取れませんでした"),
        )
    })? {
        if field.name() != Some("image") || image.is_some() {
            continue;
        }
        if field.content_type() != Some("image/webp") {
            return Err(admin_error_owned(
                StatusCode::BAD_REQUEST,
                format!("{label}はWebP形式で送信してください"),
            ));
        }
        let bytes = field.bytes().await.map_err(|_| {
            admin_error_owned(
                StatusCode::BAD_REQUEST,
                format!("{label}を読み取れませんでした"),
            )
        })?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(admin_error_owned(
                StatusCode::BAD_REQUEST,
                format!("{label}は10MiB以下にしてください"),
            ));
        }
        if !has_valid_webp_container(&bytes) {
            return Err(admin_error_owned(
                StatusCode::BAD_REQUEST,
                format!("{label}のWebP形式が不正です"),
            ));
        }
        image = Some(bytes);
    }

    image.ok_or_else(|| admin_error_owned(StatusCode::BAD_REQUEST, format!("{label}がありません")))
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
        Ok(()) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notify_display_config_changed(&state);
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

async fn upload_screen_overlay(
    Path(slot): Path<String>,
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    multipart: Multipart,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    let Ok(slot) = ScreenOverlaySlot::try_from(slot.as_str()) else {
        return admin_error(StatusCode::BAD_REQUEST, "画面オーバーレイの位置が不正です");
    };

    let image = match read_webp_image(multipart, "画面オーバーレイ").await {
        Ok(image) => image,
        Err(response) => return response,
    };
    save_webp_image(
        &state,
        &state.screen_overlay_lock,
        slot.file_name(),
        "画面オーバーレイ",
        image,
    )
    .await
}

async fn delete_screen_overlay(
    Path(slot): Path<String>,
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    let Ok(slot) = ScreenOverlaySlot::try_from(slot.as_str()) else {
        return admin_error(StatusCode::BAD_REQUEST, "画面オーバーレイの位置が不正です");
    };

    let _guard = state.screen_overlay_lock.lock().await;
    let path = state.assets_dir.join(slot.file_name());
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::error!(path = %path.display(), error = ?error, "画面オーバーレイを削除できませんでした");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "画面オーバーレイを削除できませんでした",
            )
        }
    }
}

async fn update_screen_overlay_scale(
    Path(slot): Path<String>,
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<ScreenOverlayScaleRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    let Ok(slot) = ScreenOverlaySlot::try_from(slot.as_str()) else {
        return admin_error(StatusCode::BAD_REQUEST, "画面オーバーレイの位置が不正です");
    };
    if !(1..=100).contains(&request.scale) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "画面オーバーレイの表示倍率は1から100の整数で指定してください",
        );
    }

    match state.config.update_and_save(move |config| match slot {
        ScreenOverlaySlot::TopLeft => {
            config.character.screen_overlays.top_left.scale = request.scale
        }
        ScreenOverlaySlot::TopRight => {
            config.character.screen_overlays.top_right.scale = request.scale
        }
        ScreenOverlaySlot::BottomLeft => {
            config.character.screen_overlays.bottom_left.scale = request.scale
        }
        ScreenOverlaySlot::BottomRight => {
            config.character.screen_overlays.bottom_right.scale = request.scale
        }
    }) {
        Ok(_) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "画面オーバーレイの表示倍率を保存できませんでした");
            admin_error(
                StatusCode::BAD_REQUEST,
                "画面オーバーレイの表示倍率を保存できませんでした",
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
        Ok(()) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
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
        Ok(()) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notify_display_config_changed(&state);
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
        Ok(_) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "BGM音量を保存できませんでした");
            admin_error(StatusCode::BAD_REQUEST, "BGM音量を保存できませんでした")
        }
    }
}

async fn update_model_brightness(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<ModelBrightnessRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if !request.brightness.is_finite() || !(0.0..=2.0).contains(&request.brightness) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "モデルの明るさは0.0から2.0で指定してください",
        );
    }

    match state.config.update_and_save(move |config| {
        config.character.light.brightness = request.brightness;
    }) {
        Ok(_) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "モデルの明るさを保存できませんでした");
            admin_error(
                StatusCode::BAD_REQUEST,
                "モデルの明るさを保存できませんでした",
            )
        }
    }
}

async fn update_model_antialias(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<ModelAntialiasRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    match state.config.update_and_save(move |config| {
        config.character.antialias = request.antialias;
    }) {
        Ok(_) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "アンチエイリアス設定を保存できませんでした");
            admin_error(
                StatusCode::BAD_REQUEST,
                "アンチエイリアス設定を保存できませんでした",
            )
        }
    }
}

async fn update_preparation_mode(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<PreparationModeRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let _guard = state.preparation_image_lock.lock().await;
    let has_image = tokio::fs::metadata(state.assets_dir.join(PREPARATION_IMAGE_FILE_NAME))
        .await
        .is_ok_and(|metadata| metadata.is_file());
    if request.enabled && !has_image {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "準備中画像をアップロードしてから有効にしてください",
        );
    }

    match state.config.update_and_save(move |config| {
        config.character.preparation_mode = request.enabled;
    }) {
        Ok(_) => {
            if request.enabled {
                let active = state.active.lock().await;
                if let Some(active) = active.as_ref() {
                    active.cancel.cancel();
                }
            }
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "準備中モードを保存できませんでした");
            admin_error(
                StatusCode::BAD_REQUEST,
                "準備中モードを保存できませんでした",
            )
        }
    }
}

async fn update_drawing_stabilization(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<DrawingStabilizationRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if request.stabilization > 10 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "手ブレ補正は0から10の整数で指定してください",
        );
    }

    match state.config.update_and_save(move |config| {
        config.drawing.stabilization = request.stabilization;
    }) {
        Ok(_) => admin_no_store(StatusCode::NO_CONTENT.into_response()),
        Err(error) => {
            tracing::warn!(error = ?error, "手ブレ補正を保存できませんでした");
            admin_error(StatusCode::BAD_REQUEST, "手ブレ補正を保存できませんでした")
        }
    }
}

async fn update_model_layout(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<ModelLayoutRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if request
        .camera_position
        .iter()
        .chain(request.food_prop_position.iter())
        .chain(request.food_prop_rotation_degrees.iter())
        .any(|value| !value.is_finite())
        || !request.food_prop_scale.is_finite()
        || request.food_prop_scale <= 0.0
    {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "位置と回転は有限値、Scaleは0より大きい有限値で指定してください",
        );
    }

    match state.config.update_and_save(move |config| {
        config.character.camera.position = request.camera_position;
        config.character.food_prop.position = request.food_prop_position;
        config.character.food_prop.rotation_degrees = request.food_prop_rotation_degrees;
        config.character.food_prop.size = request.food_prop_scale;
    }) {
        Ok(_) => {
            notify_display_config_changed(&state);
            admin_no_store(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "モデル配置を保存できませんでした");
            admin_error(StatusCode::BAD_REQUEST, "モデル配置を保存できませんでした")
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

fn has_valid_vrm_model(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || !bytes.len().is_multiple_of(4) || &bytes[..4] != b"glTF" {
        return false;
    }
    if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2 {
        return false;
    }
    let declared_length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if declared_length != bytes.len() {
        return false;
    }

    let mut offset = 12_usize;
    let mut first_chunk = true;
    let mut has_vrm_extension = false;
    while offset < bytes.len() {
        let Some(header_end) = offset.checked_add(8) else {
            return false;
        };
        if header_end > bytes.len() {
            return false;
        }
        let chunk_length =
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if !chunk_length.is_multiple_of(4) {
            return false;
        }
        let Some(chunk_end) = header_end.checked_add(chunk_length) else {
            return false;
        };
        if chunk_end > bytes.len() {
            return false;
        }
        if first_chunk {
            if &bytes[offset + 4..header_end] != b"JSON" {
                return false;
            }
            let Ok(root) =
                serde_json::from_slice::<serde_json::Value>(&bytes[header_end..chunk_end])
            else {
                return false;
            };
            has_vrm_extension = root
                .get("extensions")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|extensions| {
                    extensions.contains_key("VRM") || extensions.contains_key("VRMC_vrm")
                });
            first_chunk = false;
        }
        offset = chunk_end;
    }
    !first_chunk && has_vrm_extension
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

async fn event_websocket(
    Path(event_identifier): Path<String>,
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    if !has_valid_event_identifier(&state, &event_identifier) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (mut parts, _) = request.into_parts();
    let websocket = match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(websocket) => websocket,
        Err(rejection) => return rejection.into_response(),
    };
    websocket.on_upgrade(move |socket| {
        handle_websocket(socket, state, WebsocketAudience::Public(event_identifier))
    })
}

async fn admin_websocket(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    request: axum::extract::Request,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (mut parts, _) = request.into_parts();
    let websocket = match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(websocket) => websocket,
        Err(rejection) => return rejection.into_response(),
    };
    websocket.on_upgrade(move |socket| handle_websocket(socket, state, WebsocketAudience::Admin))
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

async fn clear_conversation_history(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    state.history.lock().await.clear();
    let _ = state
        .events
        .send(ServerEvent::History { turns: Vec::new() });
    admin_no_store(StatusCode::NO_CONTENT.into_response())
}

async fn reload_config(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    let previous = state.config.current();
    let previous_event_identifier = previous.event_identifier.clone();
    let previous_preparation_mode = previous.character.preparation_mode;
    match state.config.reload() {
        Ok(result) => {
            let current = state.config.current();
            if current.event_identifier != previous_event_identifier {
                notify_event_access_changed(&state);
            }
            if current.character.preparation_mode && !previous_preparation_mode {
                let active = state.active.lock().await;
                if let Some(active) = active.as_ref() {
                    active.cancel.cancel();
                }
            }
            notify_display_config_changed(&state);
            admin_no_store(
                Json(AdminReloadResponse {
                    restart_required: result.restart_required,
                })
                .into_response(),
            )
        }
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

async fn check_update(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }

    match update::check(&state.http).await {
        Ok(result) => admin_no_store(Json(result).into_response()),
        Err(error) => {
            tracing::warn!(error = ?error, "アップデートを確認できませんでした");
            admin_error_owned(
                StatusCode::BAD_GATEWAY,
                "アップデートを確認できませんでした。通信状態を確認してください。",
            )
        }
    }
}

async fn admin_version(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    let unsupported_reason = update::unsupported_reason();
    admin_no_store(
        Json(AdminVersionResponse {
            current_version: env!("CARGO_PKG_VERSION"),
            self_update_supported: unsupported_reason.is_none(),
            unsupported_reason,
        })
        .into_response(),
    )
}

async fn apply_update(State(state): State<AppState>, Query(auth): Query<AdminAuth>) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if let Some(reason) = update::unsupported_reason() {
        return admin_error_owned(StatusCode::CONFLICT, reason);
    }
    if state
        .update_in_progress
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return admin_error_owned(StatusCode::CONFLICT, "別のアップデート処理が進行中です");
    }

    match update::prepare_and_launch(&state.http).await {
        Ok(prepared) => {
            let shutdown = state.shutdown.clone();
            std::thread::spawn(move || {
                std::thread::sleep(UPDATE_SHUTDOWN_DELAY);
                let _ = shutdown.send(true);
            });
            admin_no_store(
                (
                    StatusCode::ACCEPTED,
                    Json(AdminUpdateResponse {
                        version: prepared.version,
                    }),
                )
                    .into_response(),
            )
        }
        Err(error) => {
            state.update_in_progress.store(false, Ordering::Release);
            tracing::error!(error = ?error, "アップデートを準備できませんでした");
            admin_error_owned(
                StatusCode::BAD_GATEWAY,
                "アップデートを準備できませんでした。現在のバージョンは変更されていません。",
            )
        }
    }
}

async fn admin_event_access(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    admin_no_store(
        Json(AdminEventAccessDto {
            public_base_url: state.config.current().public_base_url.clone(),
            event_identifier: state.config.current().event_identifier.clone(),
        })
        .into_response(),
    )
}

async fn update_admin_event_access(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminEventAccessDto>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if let Err(error) = validate_event_identifier(&request.event_identifier) {
        return admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response(),
        );
    }
    if let Err(error) = validate_public_base_url(&request.public_base_url) {
        return admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response(),
        );
    }
    let previous_event_identifier = state.config.current().event_identifier.clone();
    let next_event_identifier = request.event_identifier;
    let next_public_base_url = request.public_base_url.trim_end_matches('/').to_owned();
    match state.config.update_and_save(move |config| {
        config.event_identifier = next_event_identifier;
        config.public_base_url = next_public_base_url;
    }) {
        Ok(_) => {
            let event_identifier = state.config.current().event_identifier.clone();
            if event_identifier != previous_event_identifier {
                notify_event_access_changed(&state);
            }
            admin_no_store(
                Json(AdminEventAccessDto {
                    public_base_url: state.config.current().public_base_url.clone(),
                    event_identifier,
                })
                .into_response(),
            )
        }
        Err(error) => admin_no_store(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("公開URLを保存できません: {error}") })),
            )
                .into_response(),
        ),
    }
}

async fn admin_qr_code(
    State(state): State<AppState>,
    Query(auth): Query<AdminAuth>,
    Json(request): Json<AdminQrCodeRequest>,
) -> Response {
    if !has_valid_admin_token(&state, &auth) {
        return admin_no_store(StatusCode::UNAUTHORIZED.into_response());
    }
    if request.url.len() > 2_048 || validate_http_url("QRコードURL", &request.url).is_err() {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "QRコードにするURLは2048文字以下のHTTP(S) URLにしてください",
        );
    }
    let code = match qrcode::QrCode::new(request.url.as_bytes()) {
        Ok(code) => code,
        Err(_) => {
            return admin_error(
                StatusCode::BAD_REQUEST,
                "URLが長すぎるためQRコードを生成できません",
            );
        }
    };
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build();
    admin_no_store(
        (
            [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
            svg,
        )
            .into_response(),
    )
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

enum WebsocketAudience {
    Public(String),
    Admin,
}

async fn handle_websocket(
    socket: axum::extract::ws::WebSocket,
    state: AppState,
    audience: WebsocketAudience,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    let mut shutdown = state.shutdown.subscribe();
    if let WebsocketAudience::Public(event_identifier) = &audience
        && !has_valid_event_identifier(&state, event_identifier)
    {
        let _ = send_json(&mut sender, &ServerEvent::EventEnded).await;
        return;
    }
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
            changed = shutdown.changed() => {
                let should_close = changed.is_err() || *shutdown.borrow_and_update();
                if should_close {
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if matches!(event, ServerEvent::EventAccessChanged) {
                            if matches!(&audience, WebsocketAudience::Public(_)) {
                                let _ = send_json(&mut sender, &ServerEvent::EventEnded).await;
                                break;
                            }
                            continue;
                        }
                        if send_json(&mut sender, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let WebsocketAudience::Public(event_identifier) = &audience
                            && !has_valid_event_identifier(&state, event_identifier)
                        {
                            let _ = send_json(&mut sender, &ServerEvent::EventEnded).await;
                            break;
                        }
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

fn has_valid_event_identifier(state: &AppState, event_identifier: &str) -> bool {
    state.config.current().event_identifier == event_identifier
}

fn require_valid_event_identifier(
    state: &AppState,
    event_identifier: &str,
) -> Result<(), ApiError> {
    if has_valid_event_identifier(state, event_identifier) {
        Ok(())
    } else {
        Err(ApiError::not_found("このURLは使用できません。"))
    }
}

fn require_accepting_submissions(state: &AppState, event_identifier: &str) -> Result<(), ApiError> {
    require_valid_event_identifier(state, event_identifier)?;
    if state.config.current().character.preparation_mode {
        Err(ApiError::unavailable("現在は準備中です"))
    } else {
        Ok(())
    }
}

fn event_ended_response() -> Response {
    ApiError::not_found("このURLは使用できません。").into_response()
}

#[derive(Deserialize)]
struct AdminAuth {
    token: Option<String>,
}

#[derive(Serialize)]
struct SubmitResponse {
    id: String,
}

#[derive(Deserialize, Serialize)]
struct AdminEventAccessDto {
    public_base_url: String,
    event_identifier: String,
}

#[derive(Deserialize)]
struct AdminQrCodeRequest {
    url: String,
}

#[derive(Serialize)]
struct DisplayConfigDto {
    #[serde(flatten)]
    character: DisplayCharacterConfig,
    preparation_image_url: Option<String>,
    background_image_url: Option<String>,
    background_music_url: Option<String>,
    screen_overlays: ScreenOverlaysDisplayConfigDto,
    drawing: crate::config::DrawingConfig,
}

#[derive(Serialize)]
struct DisplayCharacterConfig {
    preparation_mode: bool,
    vrm_url: String,
    antialias: bool,
    idle_motions: Vec<String>,
    emotion_motions: HashMap<String, String>,
    food_prop: crate::config::FoodPropConfig,
    camera: crate::config::CameraConfig,
    background_color: String,
    background_music_volume: f32,
    background_music_duck_ratio: f32,
    light: crate::config::LightConfig,
}

impl From<CharacterConfig> for DisplayCharacterConfig {
    fn from(character: CharacterConfig) -> Self {
        Self {
            preparation_mode: character.preparation_mode,
            vrm_url: character.vrm_url,
            antialias: character.antialias,
            idle_motions: character.idle_motions,
            emotion_motions: character.emotion_motions,
            food_prop: character.food_prop,
            camera: character.camera,
            background_color: character.background_color,
            background_music_volume: character.background_music_volume,
            background_music_duck_ratio: character.background_music_duck_ratio,
            light: character.light,
        }
    }
}

#[derive(Serialize)]
struct ScreenOverlaysDisplayConfigDto {
    top_left: ScreenOverlayDisplayConfigDto,
    top_right: ScreenOverlayDisplayConfigDto,
    bottom_left: ScreenOverlayDisplayConfigDto,
    bottom_right: ScreenOverlayDisplayConfigDto,
}

#[derive(Serialize)]
struct ScreenOverlayDisplayConfigDto {
    image_url: Option<String>,
    scale: u8,
}

#[derive(Clone, Copy)]
enum ScreenOverlaySlot {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScreenOverlaySlot {
    fn file_name(self) -> &'static str {
        match self {
            Self::TopLeft => "screen-overlay-top-left.webp",
            Self::TopRight => "screen-overlay-top-right.webp",
            Self::BottomLeft => "screen-overlay-bottom-left.webp",
            Self::BottomRight => "screen-overlay-bottom-right.webp",
        }
    }
}

impl TryFrom<&str> for ScreenOverlaySlot {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "top-left" => Ok(Self::TopLeft),
            "top-right" => Ok(Self::TopRight),
            "bottom-left" => Ok(Self::BottomLeft),
            "bottom-right" => Ok(Self::BottomRight),
            _ => Err(()),
        }
    }
}

#[derive(Deserialize)]
struct BackgroundMusicVolumeRequest {
    volume: f32,
    duck_ratio: f32,
}

#[derive(Deserialize)]
struct ScreenOverlayScaleRequest {
    scale: u8,
}

#[derive(Deserialize)]
struct ModelBrightnessRequest {
    brightness: f32,
}

#[derive(Deserialize)]
struct ModelAntialiasRequest {
    antialias: bool,
}

#[derive(Deserialize)]
struct PreparationModeRequest {
    enabled: bool,
}

#[derive(Deserialize)]
struct DrawingStabilizationRequest {
    stabilization: u8,
}

#[derive(Deserialize)]
struct ModelLayoutRequest {
    camera_position: [f32; 3],
    food_prop_position: [f32; 3],
    food_prop_rotation_degrees: [f32; 3],
    food_prop_scale: f32,
}

#[derive(Serialize)]
struct AdminReloadResponse {
    restart_required: bool,
}

#[derive(Serialize)]
struct AdminUpdateResponse {
    version: String,
}

#[derive(Serialize)]
struct AdminVersionResponse {
    current_version: &'static str,
    self_update_supported: bool,
    unsupported_reason: Option<String>,
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

fn admin_error_owned(status: StatusCode, message: impl Into<String>) -> Response {
    admin_no_store((status, Json(serde_json::json!({ "error": message.into() }))).into_response())
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

    fn not_found(message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
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
        config.event_identifier = "event-8k2m4q7x9p".to_owned();
        config.character.background_music_volume = 0.3;
        config.character.background_music_duck_ratio = 0.4;
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
                vrm_model_lock: Arc::new(Mutex::new(())),
                background_image_lock: Arc::new(Mutex::new(())),
                preparation_image_lock: Arc::new(Mutex::new(())),
                screen_overlay_lock: Arc::new(Mutex::new(())),
                background_music_lock: Arc::new(Mutex::new(())),
                update_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                shutdown: tokio::sync::watch::channel(false).0,
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

    fn multipart_vrm_body(boundary: &str, file_name: &str, model: &[u8]) -> Vec<u8> {
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(model);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    fn vrm_container(extension: &str) -> Vec<u8> {
        let mut json =
            format!(r#"{{"asset":{{"version":"2.0"}},"extensions":{{"{extension}":{{}}}}}}"#)
                .into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total_length = 20 + json.len();
        let mut model = Vec::with_capacity(total_length);
        model.extend_from_slice(b"glTF");
        model.extend_from_slice(&2_u32.to_le_bytes());
        model.extend_from_slice(&u32::try_from(total_length).unwrap().to_le_bytes());
        model.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
        model.extend_from_slice(b"JSON");
        model.extend_from_slice(&json);
        model
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
    async fn only_current_event_public_pages_are_available() {
        let app = router(test_state());
        let main = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(main.status(), StatusCode::OK);

        let input = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/input")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(input.status(), StatusCode::OK);

        let draw = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/draw")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(draw.status(), StatusCode::OK);

        for path in [
            "/event/old-event-2026",
            "/event/old-event-2026/input",
            "/event/old-event-2026/draw",
        ] {
            let old = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(old.status(), StatusCode::NOT_FOUND);
            assert_eq!(old.headers()[header::CACHE_CONTROL], "no-store");
            let body = to_bytes(old.into_body(), 32 * 1024).await.unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains("このURLは使用できません。"));
            assert!(!body.contains("このイベントリンクは終了しました。"));
            assert!(!body.contains("<button"));
        }
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
    async fn update_routes_require_admin_token_and_version_does_not_contact_github() {
        let app = router(test_state());
        let unauthorized_version = app
            .clone()
            .oneshot(
                Request::get("/api/admin/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized_version.status(), StatusCode::UNAUTHORIZED);

        let unauthorized_update = app
            .clone()
            .oneshot(
                Request::get("/api/admin/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized_update.status(), StatusCode::UNAUTHORIZED);

        let version = app
            .oneshot(
                Request::get("/api/admin/version?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(version.status(), StatusCode::OK);
        assert_eq!(version.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(version.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["current_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["self_update_supported"], false);
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
    async fn event_access_requires_token_persists_change_and_invalidates_old_routes() {
        let path = std::env::temp_dir().join(format!("web-aituber-{}.json", Uuid::new_v4()));
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.admin_token = "test-token".to_owned();
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

        let mut state = test_state();
        state.config = ConfigStore::new(&path, config);
        let mut events = state.events.subscribe();
        let app = router(state.clone());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::get("/api/admin/event-access")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .clone()
            .oneshot(
                Request::put("/api/admin/event-access?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"public_base_url":"https://event.example.com","event_identifier":"短い"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let changed = app
            .clone()
            .oneshot(
                Request::put("/api/admin/event-access?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"public_base_url":"https://event.example.com","event_identifier":"x"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changed.status(), StatusCode::OK);
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::EventAccessChanged
        ));
        assert_eq!(
            AppConfig::load_from_path(&path).unwrap().event_identifier,
            "x"
        );
        assert_eq!(
            AppConfig::load_from_path(&path).unwrap().public_base_url,
            "https://event.example.com"
        );

        let old = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old.status(), StatusCode::NOT_FOUND);
        let current = app
            .oneshot(Request::get("/event/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(current.status(), StatusCode::OK);

        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn qr_code_requires_token_and_returns_uncached_svg() {
        let app = router(test_state());
        let body = r#"{"url":"https://event.example.com/event/test-event-2026"}"#;
        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/qr-code")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .clone()
            .oneshot(
                Request::post("/api/admin/qr-code?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"url":"javascript:alert(1)"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let response = app
            .oneshot(
                Request::post("/api/admin/qr-code?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "image/svg+xml; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("<svg"));
        assert!(!body.contains("test-token"));
    }

    #[tokio::test]
    async fn websocket_routes_validate_their_access_keys() {
        let app = router(test_state());
        let admin = app
            .clone()
            .oneshot(Request::get("/ws").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(admin.status(), StatusCode::UNAUTHORIZED);

        let old_event = app
            .oneshot(
                Request::get("/event/old-event-2026/ws")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old_event.status(), StatusCode::NOT_FOUND);
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
    async fn tts_user_dict_preview_merges_phrases_and_refreshes_pitch() {
        async fn audio_query(
            Query(query): Query<HashMap<String, String>>,
        ) -> Json<serde_json::Value> {
            assert_eq!(query.get("text").map(String::as_str), Some("タンタンメン"));
            assert_eq!(query.get("speaker").map(String::as_str), Some("7"));
            Json(serde_json::json!({
                "accent_phrases": [
                    {
                        "moras": [{ "text": "タ", "pitch": 1.0 }, { "text": "ン", "pitch": 1.0 }],
                        "accent": 1,
                        "pause_mora": null,
                        "is_interrogative": false
                    },
                    {
                        "moras": [
                            { "text": "タ", "pitch": 1.0 }, { "text": "ン", "pitch": 1.0 },
                            { "text": "メ", "pitch": 1.0 }, { "text": "ン", "pitch": 1.0 }
                        ],
                        "accent": 1,
                        "pause_mora": null,
                        "is_interrogative": false
                    }
                ],
                "kana": "タン'/タンメン'",
                "tempoDynamicsScale": 1.2
            }))
        }
        async fn mora_pitch(
            Query(query): Query<HashMap<String, String>>,
            Json(mut phrases): Json<serde_json::Value>,
        ) -> Json<serde_json::Value> {
            assert_eq!(query.get("speaker").map(String::as_str), Some("7"));
            assert_eq!(phrases.as_array().unwrap().len(), 1);
            assert_eq!(phrases[0]["accent"], 3);
            assert_eq!(phrases[0]["moras"].as_array().unwrap().len(), 6);
            for mora in phrases[0]["moras"].as_array_mut().unwrap() {
                mora["pitch"] = serde_json::json!(9.0);
            }
            Json(phrases)
        }
        async fn synthesis(Json(query): Json<serde_json::Value>) -> Response {
            assert_eq!(query["accent_phrases"].as_array().unwrap().len(), 1);
            assert_eq!(query["accent_phrases"][0]["accent"], 3);
            assert_eq!(query["accent_phrases"][0]["moras"][0]["pitch"], 9.0);
            assert_eq!(query["kana"], "タン'/タンメン'");
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
                    .route("/mora_pitch", post(mora_pitch))
                    .route("/synthesis", post(synthesis)),
            )
            .await
            .unwrap();
        });
        let body = serde_json::json!({
            "tts": { "engine_url": format!("http://{address}"), "speaker_id": 7 },
            "pronunciation": "タンタンメン",
            "accent_type": 3
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
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
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
        assert!(body.contains(r#""antialias":true"#));
        assert!(body.contains(r#""drawing":{"stabilization":3}"#));
    }

    #[test]
    fn vrm_model_validation_accepts_vrm_zero_and_one_only() {
        assert!(has_valid_vrm_model(&vrm_container("VRM")));
        assert!(has_valid_vrm_model(&vrm_container("VRMC_vrm")));
        assert!(!has_valid_vrm_model(&vrm_container("OTHER")));
        assert!(!has_valid_vrm_model(b"not-a-vrm"));
    }

    #[tokio::test]
    async fn vrm_model_upload_requires_token_validates_and_atomically_replaces_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join(VRM_MODEL_FILE_NAME);
        std::fs::write(&path, b"current-model").unwrap();
        let valid_model = vrm_container("VRMC_vrm");
        let boundary = "vrm-model-boundary";
        let app = router(state.clone());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/vrm-model")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_vrm_body(
                        boundary,
                        "model.vrm",
                        &valid_model,
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(std::fs::read(&path).unwrap(), b"current-model");

        let invalid = app
            .clone()
            .oneshot(
                Request::post("/api/admin/vrm-model?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_vrm_body(
                        boundary,
                        "model.vrm",
                        b"invalid-model",
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&path).unwrap(), b"current-model");

        let mut events = state.events.subscribe();
        let updated = app
            .oneshot(
                Request::post("/api/admin/vrm-model?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_vrm_body(
                        boundary,
                        "model.VRM",
                        &valid_model,
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::NO_CONTENT);
        assert_eq!(updated.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(std::fs::read(&path).unwrap(), valid_model);
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));

        std::fs::remove_dir_all(assets_dir).unwrap();
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
    async fn preparation_mode_requires_image_cancels_active_turn_and_rejects_submissions() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let config = state.config.current().as_ref().clone();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let app = router(state.clone());

        let without_image = app
            .clone()
            .oneshot(
                Request::put("/api/admin/preparation-mode?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"enabled":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(without_image.status(), StatusCode::BAD_REQUEST);

        let boundary = "preparation-upload-boundary";
        let image = webp_container(b"preparation-image");
        let uploaded = app
            .clone()
            .oneshot(
                Request::post("/api/admin/preparation-image?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(multipart_image_body(
                        boundary,
                        "image/webp",
                        &image,
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            std::fs::read(assets_dir.join(PREPARATION_IMAGE_FILE_NAME)).unwrap(),
            image
        );

        let cancel = CancellationToken::new();
        *state.active.lock().await = Some(crate::state::ActiveTurn {
            turn_id: "active-turn".to_owned(),
            cancel: cancel.clone(),
        });
        let enabled = app
            .clone()
            .oneshot(
                Request::put("/api/admin/preparation-mode?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"enabled":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(enabled.status(), StatusCode::NO_CONTENT);
        assert!(cancel.is_cancelled());
        assert!(state.config.current().character.preparation_mode);
        assert!(
            AppConfig::load_from_path(&config_path)
                .unwrap()
                .character
                .preparation_mode
        );

        let display = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let display_json: serde_json::Value =
            serde_json::from_slice(&to_bytes(display.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(display_json["preparation_mode"], true);
        assert!(
            display_json["preparation_image_url"]
                .as_str()
                .unwrap()
                .starts_with("/assets/preparation.webp?v=")
        );

        for path in ["api/submissions", "api/food-submissions"] {
            let response = app
                .clone()
                .oneshot(
                    Request::post(format!("/event/event-8k2m4q7x9p/{path}"))
                        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=empty")
                        .body(Body::from("--empty--\r\n"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let json: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                    .unwrap();
            assert_eq!(json["error"], "現在は準備中です");
        }

        let delete_while_enabled = app
            .clone()
            .oneshot(
                Request::delete("/api/admin/preparation-image?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_while_enabled.status(), StatusCode::CONFLICT);

        let disabled = app
            .clone()
            .oneshot(
                Request::put("/api/admin/preparation-mode?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(disabled.status(), StatusCode::NO_CONTENT);
        let deleted = app
            .oneshot(
                Request::delete("/api/admin/preparation-image?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        assert!(!assets_dir.join(PREPARATION_IMAGE_FILE_NAME).exists());

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn screen_overlay_upload_delete_and_display_config_use_fixed_slot_file() {
        let (state, assets_dir) = state_with_temporary_assets();
        let path = assets_dir.join("screen-overlay-top-left.webp");
        let image = webp_container(b"top-left-overlay");
        let boundary = "screen-overlay-upload";
        let body = multipart_image_body(boundary, "image/webp", &image);
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/api/admin/screen-overlays/top-left")
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
        assert!(!path.exists());

        let uploaded = app
            .clone()
            .oneshot(
                Request::post("/api/admin/screen-overlays/top-left?token=test-token")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(&path).unwrap(), image);

        let response = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["screen_overlays"]["top_left"]["scale"], 100);
        assert!(
            json["screen_overlays"]["top_left"]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("/assets/screen-overlay-top-left.webp?v=")
        );
        assert_eq!(
            json["screen_overlays"]["top_right"]["image_url"],
            serde_json::Value::Null
        );

        for _ in 0..2 {
            let deleted = app
                .clone()
                .oneshot(
                    Request::delete("/api/admin/screen-overlays/top-left?token=test-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        }
        assert!(!path.exists());

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn screen_overlay_scale_is_validated_persisted_and_notifies_display_clients() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let config = state.config.current().as_ref().clone();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let mut events = state.events.subscribe();
        let app = router(state.clone());

        let invalid = app
            .clone()
            .oneshot(
                Request::put("/api/admin/screen-overlays/top-left/scale?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"scale":0}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let updated = app
            .oneshot(
                Request::put("/api/admin/screen-overlays/bottom-right/scale?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"scale":65}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::NO_CONTENT);
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));
        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.character.screen_overlays.bottom_right.scale, 65);
        assert_eq!(saved.character.screen_overlays.top_left.scale, 100);
        assert_eq!(
            state
                .config
                .current()
                .character
                .screen_overlays
                .bottom_right
                .scale,
            65
        );

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn display_config_returns_stable_background_version_until_image_changes() {
        let (state, assets_dir) = state_with_temporary_assets();
        let app = router(state);

        let missing = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
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
                    Request::get("/event/event-8k2m4q7x9p/api/display-config")
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
        assert_eq!(urls[0], urls[1]);

        std::fs::write(
            assets_dir.join(BACKGROUND_IMAGE_FILE_NAME),
            b"updated-background",
        )
        .unwrap();
        let changed = app
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(changed.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_ne!(json["background_image_url"], urls[0]);

        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn display_config_versions_character_assets_and_sets_cache_headers() {
        let (state, assets_dir) = state_with_temporary_assets();
        std::fs::create_dir_all(assets_dir.join("motions")).unwrap();
        std::fs::write(assets_dir.join("model.vrm"), b"model").unwrap();
        std::fs::write(assets_dir.join("motions/VRMA_01.vrma"), b"motion").unwrap();
        let app = router(state);

        let load_config = || async {
            let response = app
                .clone()
                .oneshot(
                    Request::get("/event/event-8k2m4q7x9p/api/display-config")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        };
        let first = load_config().await;
        let second = load_config().await;
        let vrm_url = first["vrm_url"].as_str().unwrap();
        let motion_url = first["idle_motions"][0].as_str().unwrap();
        assert!(vrm_url.starts_with("/assets/model.vrm?v="));
        assert!(motion_url.starts_with("/assets/motions/VRMA_01.vrma?v="));
        assert_eq!(first["vrm_url"], second["vrm_url"]);
        assert_eq!(first["idle_motions"][0], second["idle_motions"][0]);

        let versioned = app
            .clone()
            .oneshot(Request::get(vrm_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            versioned.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        let unversioned = app
            .clone()
            .oneshot(
                Request::get("/assets/model.vrm")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            unversioned.headers()[header::CACHE_CONTROL],
            "public, max-age=0, must-revalidate"
        );
        let range = app
            .clone()
            .oneshot(
                Request::get(vrm_url)
                    .header(header::RANGE, "bytes=0-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            range.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        for response in [
            app.clone()
                .oneshot(
                    Request::get("/assets/missing.vrm?v=missing")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
            app.clone()
                .oneshot(
                    Request::get(vrm_url)
                        .header(header::RANGE, "bytes=999-1000")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        ] {
            assert!(matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::RANGE_NOT_SATISFIABLE
            ));
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, max-age=0, must-revalidate"
            );
        }

        std::fs::write(assets_dir.join("model.vrm"), b"updated-model").unwrap();
        let changed = load_config().await;
        assert_ne!(first["vrm_url"], changed["vrm_url"]);
        assert_eq!(
            version_local_asset_url(&assets_dir, "https://example.com/model.vrm").await,
            "https://example.com/model.vrm"
        );
        assert_eq!(
            version_local_asset_url(&assets_dir, "/static/model.vrm").await,
            "/static/model.vrm"
        );
        assert!(local_asset_path(&assets_dir, "/assets/../config.json").is_none());
        assert_eq!(
            append_asset_version("/assets/model.vrm?quality=high#preview", "version"),
            "/assets/model.vrm?quality=high&v=version#preview"
        );

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
    async fn display_config_returns_music_volume_and_stable_version() {
        let (state, assets_dir) = state_with_temporary_assets();
        let app = router(state);

        let missing = app
            .clone()
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
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
                    Request::get("/event/event-8k2m4q7x9p/api/display-config")
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
        assert_eq!(urls[0], urls[1]);

        std::fs::write(
            assets_dir.join(background_music::FILE_NAME),
            b"updated-music",
        )
        .unwrap();
        let changed = app
            .oneshot(
                Request::get("/event/event-8k2m4q7x9p/api/display-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(changed.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_ne!(json["background_music_url"], urls[0]);
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

        let mut events = state.events.subscribe();
        let response = app
            .oneshot(
                Request::put("/api/admin/background-music-volume?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"volume":0.7,"duck_ratio":0.6}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));

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
    async fn model_brightness_update_requires_token_validates_and_preserves_other_settings() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let config = (*state.config.current()).clone();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let app = router(state.clone());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::put("/api/admin/model-brightness")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"brightness":1.25}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        for invalid in [-0.1, 2.1] {
            let response = app
                .clone()
                .oneshot(
                    Request::put("/api/admin/model-brightness?token=test-token")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(r#"{{"brightness":{invalid}}}"#)))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let mut events = state.events.subscribe();
        let response = app
            .oneshot(
                Request::put("/api/admin/model-brightness?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"brightness":1.25}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));

        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.character.light.brightness, 1.25);
        assert_eq!(saved.character.light.intensity, 1.5);
        assert_eq!(saved.character.light.ambient_intensity, 0.8);
        assert_eq!(state.config.current().character.light.brightness, 1.25);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn model_antialias_update_requires_token_and_preserves_other_settings() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let config = (*state.config.current()).clone();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let app = router(state.clone());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::put("/api/admin/model-antialias")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"antialias":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let mut events = state.events.subscribe();
        let response = app
            .oneshot(
                Request::put("/api/admin/model-antialias?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"antialias":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));

        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert!(!saved.character.antialias);
        assert_eq!(saved.character.light.brightness, 1.0);
        assert!(!state.config.current().character.antialias);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn drawing_stabilization_update_requires_token_validates_and_preserves_other_settings() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let config = (*state.config.current()).clone();
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let app = router(state.clone());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::put("/api/admin/drawing-stabilization")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"stabilization":7}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .clone()
            .oneshot(
                Request::put("/api/admin/drawing-stabilization?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"stabilization":11}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.config.current().drawing.stabilization, 3);

        let response = app
            .oneshot(
                Request::put("/api/admin/drawing-stabilization?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"stabilization":7}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");

        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.drawing.stabilization, 7);
        assert_eq!(saved.character.light.brightness, 1.0);
        assert_eq!(state.config.current().drawing.stabilization, 7);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn model_layout_update_validates_and_preserves_camera_target_and_fov() {
        let (mut state, assets_dir) = state_with_temporary_assets();
        let config_path = assets_dir.join("config.json");
        let mut config = (*state.config.current()).clone();
        config.character.camera.target = [0.0, 1.25, 0.0];
        config.character.camera.fov = 35.0;
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        state.config = ConfigStore::new(&config_path, config);
        let app = router(state.clone());
        let request = r#"{"camera_position":[0.1,1.5,2.8],"food_prop_position":[0.01,0.02,0.03],"food_prop_rotation_degrees":[10.0,20.0,30.0],"food_prop_scale":0.25}"#;

        let unauthorized = app
            .clone()
            .oneshot(
                Request::put("/api/admin/model-layout")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .clone()
            .oneshot(
                Request::put("/api/admin/model-layout?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request.replace("0.25", "0.0")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let mut events = state.events.subscribe();
        let response = app
            .oneshot(
                Request::put("/api/admin/model-layout?token=test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(matches!(
            events.recv().await.unwrap(),
            ServerEvent::DisplayConfigChanged
        ));

        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.character.camera.position, [0.1, 1.5, 2.8]);
        assert_eq!(saved.character.camera.target, [0.0, 1.25, 0.0]);
        assert_eq!(saved.character.camera.fov, 35.0);
        assert_eq!(saved.character.food_prop.position, [0.01, 0.02, 0.03]);
        assert_eq!(
            saved.character.food_prop.rotation_degrees,
            [10.0, 20.0, 30.0]
        );
        assert_eq!(saved.character.food_prop.size, 0.25);
        std::fs::remove_dir_all(assets_dir).unwrap();
    }

    #[tokio::test]
    async fn conversation_history_delete_requires_token_and_broadcasts_empty_history() {
        let state = test_state();
        state
            .history
            .lock()
            .await
            .record(crate::protocol::ConversationTurn {
                turn_id: "turn-1".to_owned(),
                question: "質問".to_owned(),
                answer: "回答".to_owned(),
                sources: Vec::new(),
            });
        let history = state.history.clone();
        let mut events = state.events.subscribe();
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::delete("/api/admin/conversation-history")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(history.lock().await.snapshot().len(), 1);

        let authorized = app
            .oneshot(
                Request::delete("/api/admin/conversation-history?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::NO_CONTENT);
        assert!(history.lock().await.snapshot().is_empty());
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, ServerEvent::History { turns } if turns.is_empty()));
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
                Request::post("/event/event-8k2m4q7x9p/api/submissions")
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
                Request::post("/event/event-8k2m4q7x9p/api/submissions")
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
                Request::post("/event/event-8k2m4q7x9p/api/food-submissions")
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
                Request::post("/event/event-8k2m4q7x9p/api/food-submissions")
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
                Request::post("/event/event-8k2m4q7x9p/api/food-submissions")
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
                Request::post("/event/event-8k2m4q7x9p/api/food-submissions")
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
