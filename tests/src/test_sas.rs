use std::ops::Deref;

use anchor_client::{
    solana_sdk::{
        commitment_config::CommitmentConfig,
        instruction::Instruction,
        pubkey::Pubkey,
        signature::{read_keypair_file, Signature},
        signer::Signer,
        sysvar,
    },
    Client, ClientError, Cluster, Program,
};
use anchor_lang::{
    solana_program::{self},
    InstructionData, ToAccountMetas,
};
use sas_client::{AttestationPayload, AttestationService};

use test_solana_program::accounts::Validate as ValidateAccounts;
use test_solana_program::instruction::Validate as ValidateIx;

async fn init_sas() -> AttestationService {
    let anchor_wallet = std::env::var("ANCHOR_WALLET").unwrap();

    let payer = read_keypair_file(&anchor_wallet).unwrap();
    let issuer = payer.insecure_clone();
    let signer = payer.insecure_clone();
    let mut service = AttestationService::new("http://127.0.0.1:8899", payer, issuer, signer);

    service.init_unchecked().await.unwrap();
    service
}

async fn call_validate<C: Deref<Target = impl Signer> + Clone>(
    program: &Program<C>,
    attestation: Pubkey,
    credential: Pubkey,
    schema: Pubkey,
    user_wallet: Pubkey,
) -> Result<Signature, ClientError> {
    let accounts = ValidateAccounts {
        attestation,
        credential,
        schema,
        clock: sysvar::clock::ID,
    };

    let ix = Instruction {
        program_id: program.id(),
        accounts: accounts.to_account_metas(None),
        data: ValidateIx { user_wallet }.data(),
    };

    program.request().instruction(ix).send().await
}

/// TODO: Would be better to split test cases, but the init code would be repetitive, and I can't
/// guard it behind OnceLock because initialization is asynchronous.
#[tokio::test]
async fn test_attestation() {
    let service = init_sas().await;

    let anchor_wallet = std::env::var("ANCHOR_WALLET").unwrap();
    let payer = read_keypair_file(&anchor_wallet).unwrap();
    let client = Client::new_with_options(Cluster::Localnet, &payer, CommitmentConfig::confirmed());

    let program = client.program(test_solana_program::ID).unwrap();

    let cred_pda = service.cred_pda;
    let scheme_pda = service.schema_pda;

    // Case A: valid attestation {age:true, country:true} -> should succeed
    let user_ok = Pubkey::new_unique();
    let _att_pda_created = service
        .create_attestation(
            user_ok,
            AttestationPayload {
                age: true,
                country: true,
            },
        )
        .await
        .expect("failed to create attestation for user_ok");
    let att_ok = AttestationService::attestation_pda(cred_pda, scheme_pda, user_ok);
    let res_ok = call_validate(&program, att_ok, cred_pda, scheme_pda, user_ok).await;
    assert!(
        res_ok.is_ok(),
        "validate should succeed for a valid attestation: {:?}",
        res_ok
    );

    // Case B: user without any attestation -> runtime should fail (account not found)
    let user_missing = Pubkey::new_unique();
    let att_missing = AttestationService::attestation_pda(cred_pda, scheme_pda, user_missing);
    let res_missing =
        call_validate(&program, att_missing, cred_pda, scheme_pda, user_missing).await;
    assert!(
        res_missing.is_err(),
        "validate should fail when attestation account is missing"
    );

    // Case C: PDA/user mismatch: pass valid attestation account but wrong user param -> error
    let wrong_user = Pubkey::new_unique();
    let res_mismatch = call_validate(&program, att_ok, cred_pda, scheme_pda, wrong_user).await;
    assert!(
        res_mismatch.is_err(),
        "validate should fail when the user param doesn't match the attestation PDA"
    );

    // Case D: wrong owner: pass system_program as 'attestation' -> error (WrongOwner)
    let res_wrong_owner = call_validate(
        &program,
        solana_program::system_program::ID,
        cred_pda,
        scheme_pda,
        user_ok,
    )
    .await;
    assert!(
        res_wrong_owner.is_err(),
        "validate should fail if attestation account is not owned by SAS program"
    );
}
