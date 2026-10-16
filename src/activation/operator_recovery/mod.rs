//! Local stopped-daemon recovery. Trust is explicit operator attestation, not ps/ACK.
mod audit;
#[cfg(test)]
mod tests;
use super::{ActivationState, Receipt, ReceiptStore, ServiceIdentity, directory_for_ledger};
use crate::{
    common::proto::DeployPhase,
    jobs::{ExistingJob, existing_job},
};
pub(crate) use audit::validate_audit;
pub(crate) const RESOLUTION_MESSAGE: &str =
    "Partial activation resolved by operator attestation. Not verified success or rollback.";
pub use audit::{OperatorResolution, RecoveryBinding};
use audit::{challenge, reason_valid, sha256};
use clap::Args;
use eyre::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Args, Debug, Clone)]
pub struct RecoverArgs {
    /// Existing server config. Paths are derived from the installed service, not client settings.
    #[arg(long)]
    pub server_config: PathBuf,
    /// Exact challenge from a prior preview. Omit for read-only preview.
    #[arg(long, requires_all = ["reason", "attest_no_writers", "accept_partial_state"])]
    pub confirm: Option<String>,
    /// Explain how the partial outcome was inspected and writers stopped (1..512 bytes).
    #[arg(long, requires = "confirm")]
    pub reason: Option<String>,
    /// Attest all prior activation writers/descendants stopped and cannot resume.
    #[arg(long, requires = "confirm")]
    pub attest_no_writers: bool,
    /// Accept possible partial system changes and allow later queued/new work.
    #[arg(long, requires = "confirm")]
    pub accept_partial_state: bool,
}

trait Environment {
    fn installed(&self) -> Result<ServiceIdentity>;
    fn retained(&self, receipt: &Receipt) -> Result<()>;
}
struct Native;
impl Environment for Native {
    fn installed(&self) -> Result<ServiceIdentity> {
        ServiceIdentity::read(Path::new("/run/current-system"))
    }
    fn retained(&self, receipt: &Receipt) -> Result<()> {
        super::switch::validate_store_path(&receipt.closure)?;
        ensure!(
            ServiceIdentity::read(&receipt.closure)? == receipt.expected,
            "retained closure service identity changed"
        );
        Ok(())
    }
}

pub async fn run(job_id: u64, args: RecoverArgs) -> Result<()> {
    // This command never connects to a daemon, launches a deployer, or changes a service.
    let output = tokio::task::spawn_blocking(move || recover(job_id, &args, &Native)).await??;
    print!("{output}");
    Ok(())
}

fn recover(job_id: u64, args: &RecoverArgs, environment: &dyn Environment) -> Result<String> {
    ensure!(job_id != 0, "nonzero job ID required");
    ensure!(
        args.server_config
            .to_str()
            .is_some_and(|path| !path.chars().any(char::is_control)),
        "server config path must be UTF-8 without control characters"
    );
    let installed = environment.installed()?;
    installed.validate()?;
    let uid = nix::unistd::geteuid().as_raw();
    let actor = nix::unistd::User::from_uid(nix::unistd::geteuid())?
        .ok_or_else(|| eyre::eyre!("service account unavailable"))?;
    ensure!(
        uid != 0
            && installed.user == actor.name
            && installed.account.uid.is_none_or(|id| id == uid),
        "run recovery locally as the configured non-root service user"
    );
    let (config_path, config) = crate::config::server::load(&args.server_config)?;
    ensure!(
        fs::canonicalize(&installed.config_file)? == config_path,
        "server config does not belong to the installed service"
    );
    let ledger_path = if config.state_directory.is_absolute() {
        config.state_directory.clone()
    } else {
        fs::canonicalize(&installed.state_directory)?.join(&config.state_directory)
    };
    let activation_path = directory_for_ledger(&ledger_path)?;
    // Existing-only acquisition cannot initialize/migrate/create even on preview/error.
    // Ledger lease first excludes a running/restarting daemon for this entire operation.
    let job = existing_job(ledger_path.clone(), config.repo_url.clone(), job_id)?;
    let store = ReceiptStore::existing(&activation_path)?;
    let _operation = store.operation()?;
    ensure!(
        !store.directory().join("receipt.pending").try_exists()?,
        "activation receipt has pending writes; recovery refused"
    );
    let mut receipt = store.load(job_id)?;
    ensure!(
        receipt.expected.host == installed.host
            && receipt.expected.platform == installed.platform
            && receipt.expected.user == installed.user
            && receipt.expected.account == installed.account
            && fs::canonicalize(&receipt.expected.state_directory)?
                == fs::canonicalize(&installed.state_directory)?
            && fs::canonicalize(&receipt.expected.config_file)? == config_path,
        "receipt does not belong to the installed service layout"
    );
    ensure!(
        fs::symlink_metadata(&receipt.root)?
            .file_type()
            .is_symlink()
            && fs::canonicalize(&receipt.root)? == receipt.closure,
        "retained activation root changed"
    );
    environment.retained(&receipt)?;
    let config_hash = sha256(&fs::read(&config_path)?);
    let canonical_ledger = fs::canonicalize(&ledger_path)?;
    let canonical_activation = fs::canonicalize(&activation_path)?;
    if let Some(audit) = &receipt.operator_resolution {
        // read() has validated the complete audit, not just its enum. Exact repeats
        // only inspect/report the original immutable attestation; never overwrite.
        check_current_binding(
            &audit.binding,
            &job,
            &installed,
            &config_path,
            &config_hash,
            &canonical_ledger,
            &canonical_activation,
        )?;
        if let Some(confirm) = &args.confirm {
            ensure!(
                confirm == &audit.challenge
                    && args.reason.as_deref() == Some(audit.reason.as_str())
                    && args.attest_no_writers
                    && args.accept_partial_state,
                "operator resolution already exists with different confirmation or reason"
            );
        } else {
            ensure!(
                args.reason.is_none() && !args.attest_no_writers && !args.accept_partial_state,
                "preview cannot include partial confirmation flags"
            );
        }
        return Ok(format!(
            "Operator resolution already recorded for job {job_id}.\nOriginal attestation inspected. No new write or activation.\n"
        ));
    }
    ensure!(
        job.status.phase == DeployPhase::Interrupted && job.started,
        "recovery requires an Interrupted job with prior execution"
    );
    ensure!(
        matches!(
            receipt.state,
            ActivationState::Activating | ActivationState::Uncertain
        ),
        "recovery requires attempted Activating or Uncertain receipt; Ready is not eligible"
    );
    let original = fs::read_to_string(store.directory().join(format!("{job_id}.toml")))?;
    ensure!(
        original.len() <= 128 * 1024,
        "original receipt exceeds recovery audit bound"
    );
    let parsed: Receipt =
        toml::from_str(&original).map_err(|_| eyre::eyre!("original receipt changed"))?;
    ensure!(
        parsed == receipt,
        "original receipt changed during recovery"
    );
    let binding = RecoveryBinding {
        job_id,
        request_nonce: job.request.request_id,
        revision: job.request.rev.clone(),
        dry_run: job.request.dry_run,
        clean_substituters: job.request.clean_substituters,
        commit: job.status.commit.clone(),
        sequence: job.sequence,
        started: job.started,
        prior_job_sha256: sha256(&job.snapshot),
        high_water: job.high_water,
        server_config: config_path.clone(),
        server_config_sha256: config_hash.clone(),
        ledger: canonical_ledger,
        activation_directory: canonical_activation,
        installed: installed.clone(),
        expected: receipt.expected.clone(),
        closure: receipt.closure.clone(),
        root: receipt.root.clone(),
    };
    let prior_hash = sha256(original.as_bytes());
    let digest = challenge(&prior_hash, &binding)?;
    let Some(confirm) = &args.confirm else {
        ensure!(
            args.reason.is_none() && !args.attest_no_writers && !args.accept_partial_state,
            "preview cannot include partial confirmation flags"
        );
        return Ok(preview(job_id, args, &receipt, &binding, &digest));
    };
    ensure!(
        confirm == &digest,
        "recovery challenge is stale; inspect a new preview"
    );
    let reason = args
        .reason
        .as_deref()
        .ok_or_else(|| eyre::eyre!("recovery requires an inspection reason"))?;
    ensure!(
        reason_valid(reason) && args.attest_no_writers && args.accept_partial_state,
        "recovery requires a bounded reason and both explicit operator attestations"
    );
    // Revalidate config and installed immutable identity immediately before commit.
    let (fresh_path, fresh_config) = crate::config::server::load(&args.server_config)?;
    ensure!(
        fresh_path == config_path
            && fresh_config == config
            && sha256(&fs::read(fresh_path)?) == config_hash
            && environment.installed()? == installed,
        "service configuration changed during recovery; preview again"
    );
    receipt.state = ActivationState::OperatorResolved;
    receipt.message = RESOLUTION_MESSAGE.into();
    receipt.operator_resolution = Some(OperatorResolution {
        format: 1,
        actor_uid: uid,
        unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        reason: reason.into(),
        no_writers_attested: true,
        partial_state_accepted: true,
        prior_receipt_toml: original,
        prior_receipt_sha256: prior_hash,
        binding,
        challenge: digest,
    });
    store.save(&receipt)?; // receipt first; ledger stays Interrupted until daemon observation.
    Ok(format!(
        "Recovery recorded for job {job_id}. Operator attestation saved.\nActivation was not rerun. Restart the daemon to apply the job result.\n"
    ))
}

fn check_current_binding(
    binding: &RecoveryBinding,
    job: &ExistingJob,
    installed: &ServiceIdentity,
    config: &Path,
    config_hash: &str,
    ledger: &Path,
    activation: &Path,
) -> Result<()> {
    ensure!(
        binding.job_id == job.status.job_id
            && binding.request_nonce == job.request.request_id
            && binding.revision == job.request.rev
            && binding.dry_run == job.request.dry_run
            && binding.clean_substituters == job.request.clean_substituters
            && binding.commit == job.status.commit
            && binding.sequence == job.sequence
            && binding.started == job.started
            && binding.installed == *installed
            && binding.server_config == config
            && binding.server_config_sha256 == config_hash
            && binding.ledger == ledger
            && binding.activation_directory == activation,
        "existing operator resolution no longer matches this job or service layout"
    );
    Ok(())
}
fn preview(
    id: u64,
    args: &RecoverArgs,
    receipt: &Receipt,
    binding: &RecoveryBinding,
    digest: &str,
) -> String {
    let config = super::remote::quote(&args.server_config.to_string_lossy());
    format!(
        "Recovery preview\nTarget: {}\nJob: {id} ({})\nClosure: {}\nState: Activation outcome unknown. New deployments blocked.\nAction: Record operator resolution. Mark job failed when the daemon resumes.\nRisk: Partial system changes may remain. No activation replay or rollback.\nRequired: Confirm no deployment writers remain and accept the partial system state.\nChallenge: {digest}\nConfirm: coeus recover {id} --server-config {config} --confirm {digest} --reason '<explain inspection>' --attest-no-writers --accept-partial-state\n",
        receipt.expected.host,
        crate::utils::plain_text::bounded(&binding.revision, 256),
        crate::utils::plain_text::bounded(&receipt.closure.to_string_lossy(), 4096)
    )
}
