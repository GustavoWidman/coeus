use std::ops::{Deref, DerefMut};

use eyre::{Result, eyre};

pub struct Nonce(u64);

impl Nonce {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn advance(&mut self) -> Result<()> {
        self.0 = self
            .0
            .checked_add(1)
            .ok_or_else(|| eyre!("noise transport nonce exhausted"))?;

        Ok(())
    }
}

impl AsRef<u64> for Nonce {
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

impl AsMut<u64> for Nonce {
    fn as_mut(&mut self) -> &mut u64 {
        &mut self.0
    }
}

impl Deref for Nonce {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Nonce {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Default for Nonce {
    fn default() -> Self {
        Self::new()
    }
}
