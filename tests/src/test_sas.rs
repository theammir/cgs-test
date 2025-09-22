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
use anchor_lang::{solana_program, InstructionData, ToAccountMetas};
use sas_client::{AttestationPayload, AttestationService};

use test_solana_program::accounts::Validate as ValidateAccounts;
use test_solana_program::instruction::Validate as ValidateIx;

async fn init_sas() -> AttestationService {
    let anchor_wallet = std::env::var("ANCHOR_WALLET").unwrap();

    let payer = read_keypair_file(&anchor_wallet).unwrap();
    let issuer = read_keypair_file(&anchor_wallet).unwrap();
    let signer = read_keypair_file(&anchor_wallet).unwrap();
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

/// I totally vibecoded it.
#[tokio::test]
async fn test_attestation() {
    // 0) SAS init: credential + schema
    let service = init_sas().await;

    // 1) Anchor program client
    let anchor_wallet = std::env::var("ANCHOR_WALLET").unwrap();
    let payer = read_keypair_file(&anchor_wallet).unwrap();
    let client = Client::new_with_options(Cluster::Localnet, &payer, CommitmentConfig::confirmed());

    let program = client.program(test_solana_program::ID).unwrap();

    // 2) Derive SAS credential/schema PDAs
    let issuer = payer.pubkey();
    let cred_pda = AttestationService::credential_pda(issuer);
    let sch_pda = AttestationService::schema_pda(cred_pda);

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
    let att_ok = AttestationService::attestation_pda(cred_pda, sch_pda, user_ok);
    let res_ok = call_validate(&program, att_ok, cred_pda, sch_pda, user_ok).await;
    assert!(
        res_ok.is_ok(),
        "validate should succeed for a valid attestation: {:?}",
        res_ok
    );

    // Case B: user without any attestation -> runtime should fail (account not found)
    let user_missing = Pubkey::new_unique();
    let att_missing = AttestationService::attestation_pda(cred_pda, sch_pda, user_missing);
    let res_missing = call_validate(&program, att_missing, cred_pda, sch_pda, user_missing).await;
    assert!(
        res_missing.is_err(),
        "validate should fail when attestation account is missing"
    );

    // Case C: invalid payload {age:true, country:false} -> program error (PayloadInvalid)
    let user_bad = Pubkey::new_unique();
    let _att_pda_bad = service
        .create_attestation(
            user_bad,
            AttestationPayload {
                age: true,
                country: false,
            },
        )
        .await
        .expect("failed to create attestation for user_bad");
    let att_bad = AttestationService::attestation_pda(cred_pda, sch_pda, user_bad);
    let res_bad = call_validate(&program, att_bad, cred_pda, sch_pda, user_bad).await;
    assert!(
        res_bad.is_err(),
        "validate should fail for payload {{age:true, country:false}}"
    );

    // Case D: PDA/user mismatch: pass valid attestation account but wrong user param -> error
    let wrong_user = Pubkey::new_unique();
    let res_mismatch = call_validate(&program, att_ok, cred_pda, sch_pda, wrong_user).await;
    assert!(
        res_mismatch.is_err(),
        "validate should fail when the user param doesn't match the attestation PDA"
    );

    // Case E: wrong owner: pass system_program as 'attestation' -> error (WrongOwner)
    let res_wrong_owner = call_validate(
        &program,
        solana_program::system_program::ID,
        cred_pda,
        sch_pda,
        user_ok,
    )
    .await;
    assert!(
        res_wrong_owner.is_err(),
        "validate should fail if attestation account is not owned by SAS program"
    );
}
