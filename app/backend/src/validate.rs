use std::{str::FromStr, sync::Arc};

use anchor_client::{
    solana_sdk::{pubkey::Pubkey, signature::Signature, sysvar},
    ClientError,
};

use anchor_lang::{InstructionData, ToAccountMetas};
use axum::{extract::Query, Json};
use sas_client::AttestationService;
use serde::{Deserialize, Serialize};
use solana_sdk::instruction::Instruction;
use test_solana_program::accounts::Validate as ValidateAccounts;
use test_solana_program::instruction::Validate as ValidateIx;
use tracing::{field, instrument, warn, Span};

use crate::AppState;

impl AppState {
    pub(crate) async fn call_validate(&self, user: Pubkey) -> Result<Signature, ClientError> {
        let accounts = ValidateAccounts {
            attestation: AttestationService::attestation_pda(
                self.sas.cred_pda,
                self.sas.schema_pda,
                user,
            ),
            credential: self.sas.cred_pda,
            schema: self.sas.schema_pda,
            clock: sysvar::clock::ID,
        };

        let ix = Instruction {
            program_id: self.validate_program.id(),
            accounts: accounts.to_account_metas(None),
            data: ValidateIx { user_wallet: user }.data(),
        };

        self.validate_program.request().instruction(ix).send().await
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ValidatePayload {
    address: String,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ValidateResponse {
    address: String,
    valid: bool,
}

#[instrument(
    skip(state),
    fields(pubkey = %payload.address, success = field::Empty))
]
pub(crate) async fn validate_handler(
    Query(payload): Query<ValidatePayload>,
    state: Arc<AppState>,
) -> Json<ValidateResponse> {
    let span = Span::current();

    let mut response = ValidateResponse {
        address: payload.address,
        valid: true,
    };

    let pubkey = match Pubkey::from_str(&response.address) {
        Ok(pubkey) => pubkey,
        Err(err) => {
            span.record("success", false);
            warn!(%response.address, %err, "invalid pubkey");
            response.valid = false;
            return Json(response);
        }
    };

    match state.call_validate(pubkey).await {
        Ok(_sig) => {
            // FIX: We don't actually know if the program returned true or false.
            // Retrieving that info is clunky. I guess, for now we can assume that attestations are
            // always created with {true, true}.
            span.record("success", true);
            Json(response)
        }
        Err(err) => {
            span.record("success", false);
            warn!(%err, "couldn't validate attestation");
            response.valid = false;
            Json(response)
        }
    }
}
