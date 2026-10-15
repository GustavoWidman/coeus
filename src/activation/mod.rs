//! Exact-closure activation owned by an independent operator process, never coeusd.
mod build;
mod identity;
mod local;
mod receipt;
mod remote;
mod switch;

pub use identity::{
    AccountPolicy, BinaryIdentity, DaemonIdentity, ServiceIdentity, daemon_identity,
};
pub use local::run_local;
pub use receipt::read as read_receipt;
pub use receipt::{ActivationState, Receipt, ReceiptStore, directory_for_ledger};
pub use remote::{LocalArgs, RemoteArgs, run_remote};
pub use switch::{ActivationExecutor, NativeExecutor, activate, prepare, reconcile};

#[cfg(test)]
mod tests;

pub fn build_attribute(identity: &ServiceIdentity) -> String {
    if identity.platform == "darwin" {
        format!(".#darwinConfigurations.{}.system", identity.host)
    } else {
        format!(
            ".#nixosConfigurations.{}.config.system.build.toplevel",
            identity.host
        )
    }
}
