use std::{str::FromStr, sync::Arc};

use axum::Json;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use tracing::{debug_span, field, info, instrument, warn, Instrument};

use crate::AppState;

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct VerificationPayload {
    address: String,
}

#[derive(Debug, Serialize, Clone, Copy)]
pub(crate) struct VerificationResponse {
    age: bool,
    country: bool,
}

impl From<VerificationResponse> for sas_client::AttestationPayload {
    fn from(value: VerificationResponse) -> Self {
        Self {
            age: value.age,
            country: value.country,
        }
    }
}

#[instrument(
    skip(state),
    fields(pubkey = %payload.address))
]
pub(crate) async fn verification_handler(
    Json(payload): Json<VerificationPayload>,
    state: Arc<AppState>,
) -> Json<VerificationResponse> {
    let success_response = Json(VerificationResponse {
        age: true,
        country: true,
    });

    let span = debug_span!("attestation.fetch",
        pubkey = %payload.address,
        success = field::Empty
    );
    match Pubkey::from_str(&payload.address) {
        Ok(user_pubkey) => match state
            .sas
            .fetch_attestation(user_pubkey)
            .instrument(span.clone())
            .await
        {
            Ok(None) => {
                span.record("success", true);
                let span = debug_span!("attestation.create",
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
