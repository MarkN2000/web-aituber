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
use std::time::Duration;
use tower_http::{services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{
    config::{TtsConfig, validate_http_url},
    protocol::{AdminSkipRequest, InputImage, ServerEvent, Submission, SubmissionKind},
    state::AppState,
    tts,
};

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_TEXT_REQUEST_BYTES: usize = 128 * 1024;
const MAX_FOOD_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;

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
        .nest_service("/assets", ServeDir::new("assets"))
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
    Json(config.character.clone()).into_response()
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
                search_filler_rotation: Arc::new(SearchFillerRotation::default()),
            },
            receiver,
        )
    }

    fn test_state() -> AppState {
        test_state_with_receiver().0
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
