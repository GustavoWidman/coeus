//! Endpoint resolution is separate from authentication and admission.
use eyre::{Result, WrapErr, bail, ensure, eyre};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

pub const DEFAULT_PORT: u16 = 45729;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub host: String,
    pub port: u16,
}
impl Target {
    pub fn parse(input: &str) -> Result<Self> {
        ensure!(
            !input.is_empty() && input.len() <= 512,
            "empty or oversized target"
        );
        ensure!(
            !input.starts_with('-') && !input.chars().any(char::is_whitespace),
            "invalid target"
        );
        let (host, port) = if let Some(rest) = input.strip_prefix('[') {
            let (host, rest) = rest
                .split_once(']')
                .ok_or_else(|| eyre!("unclosed IPv6 target"))?;
            ensure!(
                host.parse::<std::net::Ipv6Addr>().is_ok(),
                "invalid bracketed IPv6 target"
            );
            let port = if rest.is_empty() {
                DEFAULT_PORT
            } else {
                rest.strip_prefix(':')
                    .ok_or_else(|| eyre!("invalid target port"))?
                    .parse()?
            };
            (host.to_string(), port)
        } else if input.parse::<IpAddr>().is_ok() {
            (input.to_string(), DEFAULT_PORT)
        } else if let Some((host, port)) = input.rsplit_once(':') {
            ensure!(
                !host.contains(':'),
                "bracket IPv6 addresses when specifying a port"
            );
            (host.to_string(), port.parse()?)
        } else {
            (input.to_string(), DEFAULT_PORT)
        };
        ensure!(port != 0, "target port must be nonzero");
        ensure!(
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b)),
            "invalid target host"
        );
        Ok(Self {
            name: input.into(),
            host,
            port,
        })
    }
    pub fn is_local(&self) -> bool {
        matches!(self.host.as_str(), "local" | "localhost" | "me")
    }
    pub async fn resolve(&self) -> Result<Vec<SocketAddr>> {
        if self.is_local() {
            return Ok(vec![SocketAddr::new(Ipv4Addr::LOCALHOST.into(), self.port)]);
        }
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, self.port)]);
        }
        // -G evaluates the user's OpenSSH configuration locally; it never connects.
        // Ignore SSH Port/User/ProxyCommand: those describe SSH, not the Noise socket.
        let host = ssh_hostname(&self.host).await?;
        let addresses = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host.as_str(), self.port)),
        )
        .await
        .wrap_err("DNS deadline exceeded")??;
        let mut result: Vec<_> = addresses.take(16).collect();
        result.sort();
        result.dedup();
        ensure!(!result.is_empty(), "target did not resolve");
        Ok(result)
    }
}

async fn ssh_hostname(host: &str) -> Result<String> {
    let mut child = match Command::new("ssh")
        .args(["-G", "-o", "CanonicalizeHostname=no", "--", host])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(host.into()),
        Err(error) => return Err(error.into()),
    };
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| eyre!("ssh configuration output unavailable"))?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut output = Vec::new();
        (&mut stdout)
            .take(256 * 1024 + 1)
            .read_to_end(&mut output)
            .await?;
        ensure!(
            output.len() <= 256 * 1024,
            "SSH configuration output exceeded bound"
        );
        ensure!(
            child.wait().await?.success(),
            "OpenSSH configuration evaluation failed"
        );
        parse_ssh_hostname(&String::from_utf8(output)?, host)
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            bail!("OpenSSH configuration deadline exceeded");
        }
    }
}
fn parse_ssh_hostname(output: &str, fallback: &str) -> Result<String> {
    let hostname = output
        .lines()
        .find_map(|line| line.strip_prefix("hostname "))
        .unwrap_or(fallback);
    ensure!(
        !hostname.is_empty() && !hostname.chars().any(char::is_whitespace),
        "invalid SSH HostName"
    );
    Ok(hostname.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_aliases_never_consult_ssh_or_dns() -> Result<()> {
        for alias in ["me", "localhost", "local"] {
            let target = Target::parse(&format!("{alias}:321"))?;
            assert_eq!(target.resolve().await?, ["127.0.0.1:321".parse()?]);
        }
        Ok(())
    }
    #[test]
    fn ports_ipv4_ipv6_and_rejected_targets() -> Result<()> {
        assert_eq!(Target::parse("lab:123")?.port, 123);
        assert_eq!(Target::parse("127.0.0.1")?.port, DEFAULT_PORT);
        assert_eq!(Target::parse("[::1]:123")?.host, "::1");
        assert_eq!(Target::parse("::1")?.port, DEFAULT_PORT);
        for bad in [
            "", "-F", "x:0", "x:65536", "a b", "a;sh", "[bad]:1", "foo/bar",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_ssh_hostname("hostname 192.0.2.7\nport 2222\n", "lab")?,
            "192.0.2.7"
        );
        Ok(())
    }
}
