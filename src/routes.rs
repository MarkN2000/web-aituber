use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Query, State, WebSocketUpgrade, ws::Message},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tower_http::{services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{
    protocol::{ClientMessage, InputImage, ServerEvent, Submission},
    state::AppState,
};

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = MAX_IMAGE_BYTES + 128 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(input_page))
        .route("/display", get(display_page))
        .route("/api/submissions", post(submit))
        .route("/api/display-config", get(display_config))
        .route("/ws", get(websocket))
        .nest_service("/static", ServeDir::new("web"))
        .nest_service("/assets", ServeDir::new("assets"))
        .nest_service("/audio", ServeDir::new(state.audio_dir.as_ref()))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn input_page() -> Response {
    html_file("web/input.html").await
}

async fn display_page(State(state): State<AppState>, Query(auth): Query<DisplayAuth>) -> Response {
    if !has_valid_token(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    html_file("web/display.html").await
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
    let mut image = None;

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
            Some("image") if image.is_none() => {
                let mime_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                if !mime_type.starts_with("image/") {
                    return Err(ApiError::bad_request("画像ファイルを選択してください"));
                }
                let data = field
                    .bytes()
                    .await
                    .map_err(|_| ApiError::bad_request("画像を読み取れませんでした"))?;
                if data.len() > MAX_IMAGE_BYTES {
                    return Err(ApiError::bad_request("画像は10MB以下にしてください"));
                }
                image = Some(InputImage {
                    mime_type,
                    data: data.to_vec(),
                });
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
            text,
            image,
        })
        .map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                ApiError::unavailable("現在混雑しています。少し待ってから再送してください")
            }
            tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                ApiError::unavailable("現在投稿を受け付けられません")
            }
        })?;

    Ok((StatusCode::ACCEPTED, Json(SubmitResponse { id })))
}

async fn display_config(
    State(state): State<AppState>,
    Query(auth): Query<DisplayAuth>,
) -> Response {
    if !has_valid_token(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(state.config.character.clone()).into_response()
}

async fn websocket(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(auth): Query<DisplayAuth>,
) -> Response {
    if !has_valid_token(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    websocket.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: axum::extract::ws::WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    let current = state.current.read().await.clone();

    if send_json(&mut sender, &ServerEvent::Snapshot { current })
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(message) = serde_json::from_str::<ClientMessage>(&text) {
                            handle_client_message(&state, message).await;
                        }
                    }
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
                        if send_json(&mut sender, &ServerEvent::Snapshot { current }).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn handle_client_message(state: &AppState, message: ClientMessage) {
    match message {
        ClientMessage::Skip { turn_id } => {
            let active = state.active.lock().await;
            if let Some(active) = active.as_ref()
                && active.turn_id == turn_id
                && !active.cancel.is_cancelled()
            {
                active.cancel.cancel();
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

fn has_valid_token(state: &AppState, auth: &DisplayAuth) -> bool {
    auth.token.as_deref() == Some(state.config.display_token.as_str())
}

#[derive(Deserialize)]
struct DisplayAuth {
    token: Option<String>,
}

#[derive(Serialize)]
struct SubmitResponse {
    id: String,
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
    use std::{path::PathBuf, sync::Arc};

    use axum::{body::to_bytes, http::Request};
    use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
    use tower::ServiceExt;

    use super::*;
    use crate::config::AppConfig;

    fn test_state() -> AppState {
        let mut config: AppConfig =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        config.display_token = "test-token".to_owned();
        let (submissions, _) = mpsc::channel(1);
        let (events, _) = broadcast::channel(1);
        AppState {
            config: Arc::new(config),
            http: reqwest::Client::new(),
            submissions,
            events,
            current: Arc::new(RwLock::new(None)),
            active: Arc::new(Mutex::new(None)),
            audio_dir: Arc::new(PathBuf::from("target/test-audio")),
        }
    }

    #[tokio::test]
    async fn input_page_is_public() {
        let response = router(test_state())
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn display_page_requires_token() {
        let app = router(test_state());
        let unauthorized = app
            .clone()
            .oneshot(Request::get("/display").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::get("/display?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn display_config_does_not_expose_secrets() {
        let response = router(test_state())
            .oneshot(
                Request::get("/api/display-config?token=test-token")
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
}
