//! the repository's age secrets, handled in process: a secret whose header
//! already names exactly its recipients is never touched, a new one is
//! encrypted straight from memory, and a stale one is decrypted with the
//! admin's identity and written back through a rename. no plaintext and no
//! private key reaches a disk or another process.

pub mod generate;
mod header;
mod identity;
mod seal;

#[cfg(test)]
mod test;

pub use generate::{Generated, generate};
pub use header::{Header, Stanza, Verdict};
pub use identity::AdminIdentity;
pub use seal::{Plan, Planned, create, plan};
