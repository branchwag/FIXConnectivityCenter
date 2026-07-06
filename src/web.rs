//! axum web server: `/sessions` JSON endpoint + static file serving, mirroring
//! the Go `startWebServer`.

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use tower_http::services::ServeDir;

use crate::fix_app::SharedStatus;

#[derive(Serialize)]
struct SessionDetail {
    #[serde(rename = "SessionID")]
    session_id: String,
    #[serde(rename = "Status")]
    status: String,
}

async fn sessions(State(status): State<SharedStatus>) -> Json<Vec<SessionDetail>> {
    let map = status.lock().unwrap();
    let details = map
        .iter()
        .map(|(id, &connected)| SessionDetail {
            session_id: id.clone(),
            status: if connected { "Connected" } else { "Disconnected" }.to_string(),
        })
        .collect();
    Json(details)
}

pub async fn serve(status: SharedStatus) {
    let app = Router::new()
        .route("/sessions", get(sessions))
        .with_state(status)
        .fallback_service(ServeDir::new("."));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .expect("failed to bind :8080");
    println!("Server starting on http://:8081");
    axum::serve(listener, app).await.expect("web server error");
}
