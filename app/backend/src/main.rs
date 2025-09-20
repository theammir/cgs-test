use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
struct VerificationPayload {
    address: String,
}

#[derive(Debug, Serialize)]
struct VerificationResponse {
    age: bool,
    country: bool,
}

async fn verification_handler(
    Json(_payload): Json<VerificationPayload>,
) -> Json<VerificationResponse> {
    // mock, but worth validating the `address` here.
    Json(VerificationResponse {
        age: true,
        country: true,
    })
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/verification", post(verification_handler));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
