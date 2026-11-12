use eyre::Result;

use crate::common::model::{HostInfo, Hostname, Platform};

impl HostInfo {
    /// this machine, named by the configuration or else by its hostname
    /// without any domain (`altar.local` is `altar`)
    pub fn detect(configured: Option<Hostname>) -> Result<Self> {
        let hostname = match configured {
            Some(hostname) => hostname,
            None => {
                let uname = rustix::system::uname();
                let nodename = uname.nodename().to_string_lossy();
                nodename.split('.').next().unwrap_or_default().parse()?
            }
        };

        let platform = Platform::current();
        let os = match platform {
            Platform::NixOs => "linux",
            Platform::Darwin => "darwin",
        };

        Ok(Self {
            hostname,
            platform,
            system: format!("{}-{os}", std::env::consts::ARCH),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        })
    }
}
