use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use axum::{routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use tokio::net::TcpListener;

use crate::sas_client::AttestationService;

mod sas_client;

#[derive(Debug, Deserialize, Clone)]
struct VerificationPayload {
    address: String,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct VerificationResponse {
    age: bool,
    country: bool,
}

impl From<sas_client::AttestationPayload> for VerificationResponse {
    fn from(value: sas_client::AttestationPayload) -> Self {
        Self {
            age: value.age,
            country: value.country,
        }
    }
}

impl From<VerificationResponse> for sas_client::AttestationPayload {
    fn from(value: VerificationResponse) -> Self {
        Self {
            age: value.age,
            country: value.country,
        }
    }
}

struct AppState {
    pub sas: AttestationService,
}

async fn verification_handler(
    Json(payload): Json<VerificationPayload>,
    state: Arc<AppState>,
) -> Json<VerificationResponse> {
    // mock, but worth validating the `address` here.
    let verification = Json(VerificationResponse {
        age: true,
        country: true,
    });

    match Pubkey::from_str(&payload.address) {
        Ok(user_pubkey) => match state.sas.fetch_user_attestation(user_pubkey).await {
            Ok(None) => {
                _ = state
                    .sas
                    .create_attestation(user_pubkey, verification.0.into())
                    .await;
            }
            Ok(Some(_)) => {
                // attestation already exists
                todo!()
            }
            Err(_) => {
                // attestation couldn't have been fetched
                todo!()
            }
        },
        Err(_) => {
            // invalid pubkey
            todo!()
        }
    }

    verification
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let shared_state = {
        let mut sas = AttestationService::try_from_env().unwrap();
        sas.init().await.unwrap();
        Arc::new(AppState { sas })
    };
    let app = Router::new().route(
        "/verification",
        post({
            let state = Arc::clone(&shared_state);
            move |payload| verification_handler(payload, state)
        }),
    );

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
