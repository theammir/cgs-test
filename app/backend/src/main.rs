use std::error::Error;
use std::sync::Arc;

use anchor_client::{Client, Cluster, Program};
use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use sas_client::AttestationService;
use solana_sdk::signature::{read_keypair_file, Keypair};
use tokio::net::TcpListener;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_error::ErrorLayer;
use tracing_subscriber::{
    fmt::{format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

mod validate;
mod verification;

pub(crate) struct AppState {
    pub sas: AttestationService,
    pub validate_program: Program<Arc<Keypair>>,
}

impl AppState {
    pub fn try_from_env(sas: AttestationService) -> std::result::Result<Self, Box<dyn Error>> {
        let payer = read_keypair_file(std::env::var("PAYER_CREDS")?)?;
        let client = Client::new(
            if std::env::var("CLUSTER").is_ok_and(|cluster| cluster == "devnet") {
                Cluster::Devnet
            } else {
                Cluster::Localnet
            },
            Arc::new(payer),
        );
        let program = client.program(test_solana_program::ID)?;
        Ok(Self {
            sas,
            validate_program: program,
        })
    }
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
        Arc::new(AppState::try_from_env(sas).unwrap())
    };
    let app = Router::new()
        .route(
            "/verification",
            post({
                let state = Arc::clone(&shared_state);
                move |payload| verification::verification_handler(payload, state)
            }),
        )
        .route(
            "/validate",
            get({
                let state = Arc::clone(&shared_state);
                move |payload| validate::validate_handler(payload, state)
            }),
        );

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
