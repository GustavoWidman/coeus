//! Versioned operator provenance. An attestation is not verified activation.
use super::super::{ActivationState, Receipt, ServiceIdentity};
use eyre::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryBinding {
    pub job_id: u64,
    pub request_nonce: u64,
    pub revision: String,
    pub dry_run: bool,
    pub clean_substituters: bool,
    pub commit: Option<String>,
    pub sequence: u64,
    pub started: bool,
    pub prior_job_sha256: String,
    pub high_water: u64,
    pub server_config: PathBuf,
    pub server_config_sha256: String,
    pub ledger: PathBuf,
    pub activation_directory: PathBuf,
    pub installed: ServiceIdentity,
    pub expected: ServiceIdentity,
    pub closure: PathBuf,
    pub root: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorResolution {
    pub format: u8,
    pub actor_uid: u32,
    pub unix_seconds: u64,
    pub reason: String,
    pub no_writers_attested: bool,
    pub partial_state_accepted: bool,
    pub prior_receipt_toml: String,
    pub prior_receipt_sha256: String,
    pub binding: RecoveryBinding,
    pub challenge: String,
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn challenge(prior: &str, binding: &RecoveryBinding) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"coeus-operator-recovery-v1\0");
    for part in [prior.as_bytes().to_vec(), postcard::to_stdvec(binding)?] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn reason_valid(reason: &str) -> bool {
    !reason.trim().is_empty() && reason.len() <= 512 && !reason.chars().any(char::is_control)
}

pub(crate) fn validate_audit(receipt: &Receipt) -> Result<()> {
    let Some(audit) = &receipt.operator_resolution else {
        ensure!(
            receipt.state != ActivationState::OperatorResolved,
            "operator resolution is missing its audit"
        );
        return Ok(());
    };
    ensure!(
        receipt.state == ActivationState::OperatorResolved
            && receipt.message == super::RESOLUTION_MESSAGE,
        "operator audit on a non-resolved or modified receipt"
    );
    ensure!(
        audit.format == 1
            && audit.unix_seconds > 0
            && audit.unix_seconds <= i64::MAX as u64
            && reason_valid(&audit.reason)
            && audit.no_writers_attested
            && audit.partial_state_accepted,
        "invalid operator recovery attestations"
    );
    ensure!(
        audit.actor_uid == nix::unistd::geteuid().as_raw(),
        "operator audit belongs to another service user"
    );
    ensure!(
        audit.prior_receipt_toml.len() <= 128 * 1024
            && sha256(audit.prior_receipt_toml.as_bytes()) == audit.prior_receipt_sha256,
        "invalid prior receipt provenance"
    );
    let prior: Receipt = toml::from_str(&audit.prior_receipt_toml)
        .map_err(|_| eyre::eyre!("invalid original operator receipt"))?;
    ensure!(
        prior.operator_resolution.is_none()
            && matches!(
                prior.state,
                ActivationState::Activating | ActivationState::Uncertain
            ),
        "operator recovery did not acknowledge an attempted activation"
    );
    let mut original = receipt.clone();
    original.operator_resolution = None;
    original.state = prior.state;
    original.message = prior.message.clone();
    ensure!(
        original == prior,
        "operator resolution changed original activation identity"
    );
    let binding = &audit.binding;
    binding.installed.validate()?;
    binding.expected.validate()?;
    ensure!(
        binding.job_id == receipt.job_id
            && binding.request_nonce != 0
            && binding.started
            && binding.sequence > 0
            && binding.high_water >= binding.job_id
            && !binding.revision.is_empty()
            && binding.revision.len() <= 256
            && !binding.revision.chars().any(char::is_control)
            && binding.expected == receipt.expected
            && binding.closure == receipt.closure
            && binding.root == receipt.root
            && binding.server_config.is_absolute()
            && binding.ledger.is_absolute()
            && binding.activation_directory.is_absolute()
            && super::super::directory_for_ledger(&binding.ledger)? == binding.activation_directory
            && binding.commit.as_ref().is_none_or(
                |c| matches!(c.len(), 40 | 64) && c.bytes().all(|b| b.is_ascii_hexdigit())
            )
            && binding.installed.host == binding.expected.host
            && binding.installed.platform == binding.expected.platform
            && binding.installed.user == binding.expected.user
            && binding.installed.account == binding.expected.account
            && digest_valid(&binding.prior_job_sha256)
            && digest_valid(&binding.server_config_sha256)
            && digest_valid(&audit.prior_receipt_sha256)
            && audit.challenge == challenge(&audit.prior_receipt_sha256, binding)?,
        "operator recovery binding is inconsistent"
    );
    Ok(())
}
