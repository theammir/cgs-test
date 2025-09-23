use anyhow::{anyhow, Result};
use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing::{
    debug, debug_span,
    field::{self},
    info, warn, Instrument,
};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_attestation_service_client::{
    accounts::Attestation,
    instructions::{CreateAttestationBuilder, CreateCredentialBuilder, CreateSchemaBuilder},
    programs::SOLANA_ATTESTATION_SERVICE_ID,
};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    nonblocking::rpc_client::RpcClient,
    rpc_request::RpcError,
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    message::Message,
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use solana_system_interface::program;

pub const CREDENTIAL_NAME: &str = "Test Credential";
pub const SCHEMA_NAME: &str = "UserVerification";
pub const SCHEMA_VERSION: u8 = 1;
pub const SCHEMA_DESC: &str = "age: bool, country: bool";
const ATTESTATION_EXPIRY: Duration = Duration::from_secs(60 * 60 * 24 * 30);
const MIN_SOL_BALANCE: u32 = 2;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default)]
pub struct AttestationPayload {
    pub age: bool,
    pub country: bool,
}

impl AttestationPayload {
    pub const fn layout() -> [u8; 2] {
        [10, 10]
    }
    pub const fn fields() -> [&'static str; 2] {
        ["age", "country"]
    }
}

pub struct AttestationService {
    rpc: RpcClient,
    payer: Keypair,
    issuer: Keypair,
    signer: Keypair,

    pub cred_pda: Pubkey,
    pub schema_pda: Pubkey,
}

impl AttestationService {
    pub fn new(rpc_url: &str, payer: Keypair, issuer: Keypair, signer: Keypair) -> Self {
        let rpc =
            RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
        let cred_pda = Self::credential_pda(issuer.pubkey());
        let schema_pda = Self::schema_pda(cred_pda);
        Self {
            rpc,
            payer,
            issuer,
            signer,
            cred_pda,
            schema_pda,
        }
    }

    /// Airdrops some SOL to payer, so that a min threshold is passed,
    /// and tries to create credential and schema accounts if not already present.
    pub async fn init(&mut self) -> Result<()> {
        let balance = self.airdrop_up_to(MIN_SOL_BALANCE).await?;
        debug!(
            %balance,
            "airdropped sol to payer if needed"
        );
        if !self.account_exists(self.cred_pda).await? {
            let sig = self.create_credential().await?;
            debug!(%sig, "created new credential");
        }
        if !self.account_exists(self.schema_pda).await? {
            let sig = self.create_schema().await?;
            debug!(%sig, "created new schema");
        }
        info!("successfully initialized attestation service");
        Ok(())
    }

    /// [`Self::init`], but for using on a clean localnet.
    pub async fn init_unchecked(&mut self) -> Result<()> {
        self.airdrop_up_to(MIN_SOL_BALANCE).await?;
        self.create_credential().await?;
        self.create_schema().await?;
        Ok(())
    }

    pub fn try_from_env() -> std::result::Result<Self, Box<dyn Error>> {
        Ok(Self::new(
            &std::env::var("RPC_URL")?,
            read_keypair_file(&std::env::var("PAYER_CREDS")?)?,
            read_keypair_file(&std::env::var("ISSUER_CREDS")?)?,
            read_keypair_file(&std::env::var("SIGNER_CREDS")?)?,
        ))
    }
}

impl AttestationService {
    async fn account_exists(&self, pk: Pubkey) -> Result<bool> {
        let account = self.rpc.get_account(&pk).await;
        Ok(match account {
            Ok(_) => true,
            Err(ClientError {
                request: _,
                kind: ClientErrorKind::RpcError(RpcError::ForUser(_)),
            }) => false,
            Err(e) => Err(e)?,
        })
    }

    async fn send(
        &self,
        instruction: Instruction,
        extra_signers: &[&Keypair],
    ) -> Result<Signature> {
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend_from_slice(extra_signers);

        let msg = Message::new(
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(400_000),
                ComputeBudgetInstruction::set_compute_unit_price(1),
                instruction,
            ],
            Some(&self.payer.pubkey()),
        );

        let bh = self.rpc.get_latest_blockhash().await?;
        let tx = Transaction::new(&signers, msg, bh);
        let sig = self
            .rpc
            .send_and_confirm_transaction_with_spinner(&tx)
            .await?;
        Ok(sig)
    }

    pub fn payer(&self) -> Keypair {
        self.payer.insecure_clone() // kinda bad, but it's for a different service
    }

    pub fn credential_pda(issuer: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[b"credential", issuer.as_ref(), CREDENTIAL_NAME.as_bytes()],
            &SOLANA_ATTESTATION_SERVICE_ID,
        )
        .0
    }

    pub fn schema_pda(credential_pda: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                b"schema",
                credential_pda.as_ref(),
                SCHEMA_NAME.as_bytes(),
                &[SCHEMA_VERSION],
            ],
            &SOLANA_ATTESTATION_SERVICE_ID,
        )
        .0
    }

    pub fn attestation_pda(credential_pda: Pubkey, schema_pda: Pubkey, user: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                b"attestation",
                credential_pda.as_ref(),
                schema_pda.as_ref(),
                user.as_ref(),
            ],
            &SOLANA_ATTESTATION_SERVICE_ID,
        )
        .0
    }

    /// On success, returns factual balance in lamperts after possible airdrop.
    /// It should be no less than `amount_sol`.
    async fn airdrop_up_to(&self, amount_sol: u32) -> Result<u64> {
        let amount_lamperts = (amount_sol as u64) * (LAMPORTS_PER_SOL);
        let balance = self.rpc.get_balance(&self.payer.pubkey()).await?;
        if balance >= amount_lamperts {
            return Ok(balance);
        }

        let sig = self
            .rpc
            .request_airdrop(&self.payer.pubkey(), (amount_lamperts - balance) as u64)
            .await?;
        self.rpc
            .confirm_transaction_with_spinner(
                &sig,
                &self.rpc.get_latest_blockhash().await?,
                CommitmentConfig::confirmed(),
            )
            .await?;
        Ok(amount_lamperts)
    }

    async fn create_credential(&self) -> Result<Signature> {
        let instruction = CreateCredentialBuilder::new()
            .payer(self.payer.pubkey())
            .credential(self.cred_pda)
            .authority(self.issuer.pubkey())
            .system_program(program::id())
            .name(CREDENTIAL_NAME.to_string())
            .signers(vec![self.signer.pubkey()])
            .instruction();

        self.send(instruction, &[&self.issuer]).await
    }

    async fn create_schema(&self) -> Result<Signature> {
        let instruction = CreateSchemaBuilder::new()
            .payer(self.payer.pubkey())
            .authority(self.issuer.pubkey())
            .credential(self.cred_pda)
            .schema(self.schema_pda)
            .name(SCHEMA_NAME.to_string())
            .description(SCHEMA_DESC.to_string())
            .layout(AttestationPayload::layout().to_vec())
            .field_names(
                AttestationPayload::fields()
                    .map(String::from)
                    .into_iter()
                    .collect(),
            )
            .instruction();

        self.send(instruction, &[&self.issuer]).await
    }
}

impl AttestationService {
    pub async fn create_attestation(
        &self,
        user: Pubkey,
        payload: AttestationPayload,
    ) -> Result<Pubkey> {
        let mut data = Vec::with_capacity(2);
        payload.serialize(&mut data)?;

        let expiry = (SystemTime::now() + ATTESTATION_EXPIRY)
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let attestation_pda = Self::attestation_pda(self.cred_pda, self.schema_pda, user);

        let instruction = CreateAttestationBuilder::new()
            .payer(self.payer.pubkey())
            .authority(self.signer.pubkey())
            .credential(self.cred_pda)
            .schema(self.schema_pda)
            .attestation(attestation_pda)
            .data(data)
            .nonce(user)
            .expiry(expiry)
            .instruction();
        debug!(?instruction);

        _ = self.send(instruction, &[&self.signer]).await?;

        Ok(attestation_pda)
    }

    pub async fn fetch_attestation(&self, user: Pubkey) -> Result<Option<AttestationPayload>> {
        let attestation_pda = Self::attestation_pda(self.cred_pda, self.schema_pda, user);

        let span = debug_span!("attestation.get", pda = %attestation_pda, success = field::Empty);
        let Ok(acc) = self
            .rpc
            .get_account(&attestation_pda)
            .instrument(span.clone())
            .await
        else {
            span.record("success", true);
            return Ok(None);
        };
        span.record("success", true);

        let span = debug_span!("attestation.parse.header",
            pda = %attestation_pda,
            owner = %acc.owner,
            success = field::Empty
        );
        let attestation = match Attestation::from_bytes(&acc.data) {
            Ok(attestation) => {
                span.record("success", true);
                attestation
            }
            Err(err) => {
                span.record("success", false);
                warn!(%err, "couldn't parse attestation header");
                return Err(anyhow!("couldn't parse attestation header: {err}"));
            }
        };

        let span = debug_span!("attestation.parse.payload",
            pda = %attestation_pda,
            owner = %acc.owner,
            success = field::Empty
        );
        let payload = match AttestationPayload::try_from_slice(attestation.data.as_slice()) {
            Ok(payload) => {
                span.record("success", true);
                payload
            }
            Err(err) => {
                span.record("success", false);
                warn!(%err, "couldn't parse attestation payload");
                return Err(anyhow!("couldn't decode payload: {err}"));
            }
        };

        Ok(Some(payload))
    }
}
