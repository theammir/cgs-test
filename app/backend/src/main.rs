use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use axum::{routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use tokio::net::TcpListener;
use tracing::{field, info, info_span, instrument, warn, Instrument};
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_error::ErrorLayer;
use tracing_subscriber::{
    fmt::{format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

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

#[instrument(name = "handlers.verification",
    skip(state),
    fields(pubkey = %payload.address))
]
async fn verification_handler(
    Json(payload): Json<VerificationPayload>,
    state: Arc<AppState>,
) -> Json<VerificationResponse> {
    let success_response = Json(VerificationResponse {
        age: true,
        country: true,
    });

    let span = info_span!("attestation.fetch",
        pubkey = %payload.address,
        success = field::Empty
    );
    match Pubkey::from_str(&payload.address) {
        Ok(user_pubkey) => match state
            .sas
            .fetch_user_attestation(user_pubkey)
            .instrument(span.clone())
            .await
        {
            Ok(None) => {
                span.record("success", true);
                let span = info_span!("attestation.create",
                    pubkey = %payload.address,
                    success = field::Empty
                );
                if let Err(err) = state
                    .sas
                    .create_attestation(user_pubkey, success_response.0.into())
                    .instrument(span.clone())
                    .await
                {
                    span.record("success", false);
                    warn!(%err, "couldn't attest user")
                } else {
                    span.record("success", true);
                }
            }
            Ok(Some(_)) => {
                span.record("success", true);
                info!("attestation exists, skipping");
            }
            Err(err) => {
                span.record("success", false);
                warn!(%err, "couldn't fetch attestation");
            }
        },
        Err(err) => {
            span.record("success", false);
            warn!(pubkey = %payload.address, %err, "invalid pubkey");
            return Json(VerificationResponse {
                age: false,
                country: false,
            });
        }
    }

    success_response
}

fn init_tracing() -> (WorkerGuard, WorkerGuard) {
    let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());

    let file_appender = rolling::daily("logs", "app.jsonl");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let pretty_stdout = tracing_subscriber::fmt::layer()
        .with_writer(stdout_writer)
        .with_timer(UtcTime::rfc_3339())
        .with_ansi(std::env::var("NO_COLOR").is_err())
        .with_target(true)
        .with_span_events(FmtSpan::CLOSE)
        .pretty();

    let json_file = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_timer(UtcTime::rfc_3339())
        .with_ansi(false)
        .with_target(true)
        .with_span_events(FmtSpan::CLOSE)
        .json();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(ErrorLayer::default())
        .with(pretty_stdout)
        .with(json_file)
        .init();

    (stdout_guard, file_guard)
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;
    let _tracing_guards = init_tracing();

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
