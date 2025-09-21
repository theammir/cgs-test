use anyhow::{anyhow, Result};
use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_attestation_service_client::{
    accounts::Attestation,
    instructions::{CreateAttestationBuilder, CreateCredentialBuilder, CreateSchemaBuilder},
    programs::SOLANA_ATTESTATION_SERVICE_ID,
};
use solana_client::nonblocking::rpc_client::RpcClient;
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

const CREDENTIAL_NAME: &str = "CREDENTIAL";
const SCHEMA_NAME: &str = "VERIFICATION";
const SCHEMA_VERSION: u8 = 1;
const SCHEMA_DESC: &str = "{age: bool, country: bool}";

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default)]
pub struct AttestationPayload {
    pub age: bool,
    pub country: bool,
}

impl AttestationPayload {
    pub const fn layout() -> [u8; 2] {
        [1, 1]
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

    cred_pda: Pubkey,
    schema_pda: Pubkey,
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

    pub async fn init(&mut self) -> Result<()> {
        self.airdrop_up_to(2).await?;
        if !self.account_exists(self.cred_pda).await? {
            self.create_credential().await?;
        }
        if !self.account_exists(self.schema_pda).await? {
            self.create_schema().await?;
        }
        Ok(())
    }

    pub fn try_from_env() -> std::result::Result<Self, Box<dyn Error>> {
        Ok(Self::new(
            &std::env::var("SAS_RPC_URL")?,
            read_keypair_file(&std::env::var("SAS_PAYER_CREDS")?)?,
            read_keypair_file(&std::env::var("SAS_ISSUER_CREDS")?)?,
            read_keypair_file(&std::env::var("SAS_SIGNER_CREDS")?)?,
        ))
    }
}

impl AttestationService {
    async fn account_exists(&self, pk: Pubkey) -> Result<bool> {
        Ok(self.rpc.get_account(&pk).await.is_ok())
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

    fn credential_pda(issuer: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[b"credential", issuer.as_ref(), CREDENTIAL_NAME.as_bytes()],
            &SOLANA_ATTESTATION_SERVICE_ID,
        )
        .0
    }

    fn schema_pda(credential_pda: Pubkey) -> Pubkey {
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

    fn attestation_pda(&self, credential_pda: Pubkey, schema_pda: Pubkey, user: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                b"attestation",
                &credential_pda.to_bytes(),
                &schema_pda.to_bytes(),
                &user.to_bytes(),
            ],
            &SOLANA_ATTESTATION_SERVICE_ID,
        )
        .0
    }

    async fn airdrop_up_to(&self, amount_sol: u32) -> Result<()> {
        let amount_lamperts = (amount_sol as u64) * (LAMPORTS_PER_SOL);
        let balance = self.rpc.get_balance(&self.payer.pubkey()).await?;
        if balance >= amount_lamperts {
            return Ok(());
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
        Ok(())
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

        let expiry = (SystemTime::now() + Duration::from_secs(60 * 60 * 24 * 365))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let attestation_pda = self.attestation_pda(self.cred_pda, self.schema_pda, user);

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

        self.send(instruction, &[&self.signer]).await?;
        Ok(attestation_pda)
    }

    pub async fn fetch_user_attestation(&self, user: Pubkey) -> Result<Option<AttestationPayload>> {
        let attestation_pda = self.attestation_pda(self.cred_pda, self.schema_pda, user);

        let Ok(acc) = self.rpc.get_account(&attestation_pda).await else {
            return Ok(None);
        };

        let attestation = Attestation::from_bytes(&acc.data)
            .map_err(|e| anyhow!("failed to parse attestation header: {e}"))?;

        let payload_bytes: &[u8] = &attestation.data;
        let payload = AttestationPayload::try_from_slice(payload_bytes)
            .map_err(|e| anyhow!("failed to decode payload: {e}"))?;

        Ok(Some(payload))
    }
}
