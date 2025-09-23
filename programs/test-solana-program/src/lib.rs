#![expect(deprecated)] // #[program] attribute reports deprecated stuff

use anchor_lang::prelude::*;

use solana_attestation_service_client::{
    accounts::Attestation as SasAttestation, programs::SOLANA_ATTESTATION_SERVICE_ID,
};

declare_id!("FSzAQ5gnGcpGTc6HoPb28JMBnVWyZ7Uj1NXZ2zrwYLyh");

#[program]
pub mod test_solana_program {
    use super::*;

    /// Validate that user has an attestation with payload { age: true, country: true }
    /// and that it hasn’t expired.
    pub fn validate(ctx: Context<Validate>, user_wallet: Pubkey) -> Result<()> {
        validate_impl(ctx, user_wallet)
    }
}

fn validate_impl(ctx: Context<Validate>, user_wallet: Pubkey) -> Result<()> {
    let attestation_ai = &ctx.accounts.attestation;
    let credential_ai = &ctx.accounts.credential;
    let schema_ai = &ctx.accounts.schema;
    let clock = &ctx.accounts.clock;

    // 1) Owner check (must be SAS program)
    require_keys_eq!(
        *attestation_ai.owner,
        SOLANA_ATTESTATION_SERVICE_ID,
        AttestError::WrongOwner
    );

    // 2) PDA check: attestation = PDA(b"attestation", credential, schema, user)
    let (expected_attestation, _bump) = Pubkey::find_program_address(
        &[
            b"attestation",
            credential_ai.key.as_ref(),
            schema_ai.key.as_ref(),
            user_wallet.as_ref(),
        ],
        &SOLANA_ATTESTATION_SERVICE_ID,
    );
    require_keys_eq!(
        attestation_ai.key(),
        expected_attestation,
        AttestError::InvalidAttestationPda
    );

    // 3) Parse SAS Attestation header
    let data = attestation_ai.try_borrow_data()?;
    let att = SasAttestation::from_bytes(&data).map_err(|_| error!(AttestError::DecodeFailed))?;

    // Sanity: header should reference passed credential/schema and nonce == user
    require_keys_eq!(
        att.credential,
        credential_ai.key(),
        AttestError::HeaderMismatch
    );
    require_keys_eq!(att.schema, schema_ai.key(), AttestError::HeaderMismatch);
    require_keys_eq!(att.nonce, user_wallet, AttestError::HeaderMismatch);

    // 4) Expiry check against Clock sysvar
    let now = clock.unix_timestamp; // seconds
    require!(now < att.expiry, AttestError::Expired);

    // 5) Payload check: expecting exactly two bytes [1, 1]
    let payload: &[u8] = &att.data;
    require!(payload.len() == 2, AttestError::SchemaMismatch);
    let age_true = payload[0] != 0;
    let country_true = payload[1] != 0;
    let valid = age_true && country_true;

    emit!(ValidationResult {
        user: user_wallet,
        valid,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct Validate<'info> {
    /// CHECK: SAS attestation PDA for (credential, schema, user)
    pub attestation: UncheckedAccount<'info>,
    /// CHECK: SAS credential PDA for the issuer and CREDENTIAL_NAME
    pub credential: UncheckedAccount<'info>,
    /// CHECK: SAS schema PDA for the credential, SCHEMA_NAME, SCHEMA_VERSION
    pub schema: UncheckedAccount<'info>,
    pub clock: Sysvar<'info, Clock>,
}

#[event]
pub struct ValidationResult {
    pub user: Pubkey,
    pub valid: bool,
}

#[error_code]
pub enum AttestError {
    #[msg("Attestation account is not owned by SAS program")]
    WrongOwner,
    #[msg("Attestation PDA mismatch")]
    InvalidAttestationPda,
    #[msg("Could not decode SAS attestation account")]
    DecodeFailed,
    #[msg("Attestation header mismatch")]
    HeaderMismatch,
    #[msg("Attestation expired")]
    Expired,
    #[msg("Schema/payload length mismatch")]
    SchemaMismatch,
}
