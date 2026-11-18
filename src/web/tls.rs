//! TLS for the sign-in site: the machine's own `*.ts.net` certificate, read
//! again whenever its files change, so a renewal needs no restart

use std::{
    fmt, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use eyre::{Result, WrapErr, eyre};
use log::warn;
use rustls::{
    ServerConfig,
    crypto::{CryptoProvider, ring},
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

/// a certificate chain and its key, as PEM files
pub struct Certificate {
    cert: PathBuf,
    key: PathBuf,
    provider: Arc<CryptoProvider>,
    current: Mutex<Loaded>,
}

struct Loaded {
    /// both files' modification times when last read
    stamp: Option<(SystemTime, SystemTime)>,
    key: Arc<CertifiedKey>,
}

impl Certificate {
    pub fn load(cert: &Path, key: &Path) -> Result<Arc<Self>> {
        let provider = Arc::new(ring::default_provider());
        let stamp = stamp(cert, key);
        let loaded = read(cert, key, &provider)?;
        Ok(Arc::new(Self {
            cert: cert.to_owned(),
            key: key.to_owned(),
            provider,
            current: Mutex::new(Loaded { stamp, key: loaded }),
        }))
    }

    /// a server that presents this certificate
    pub fn acceptor(self: &Arc<Self>) -> TlsAcceptor {
        let mut config = ServerConfig::builder_with_provider(self.provider.clone())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default versions")
            .with_no_client_auth()
            .with_cert_resolver(self.clone());
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        TlsAcceptor::from(Arc::new(config))
    }
}

impl ResolvesServerCert for Certificate {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let stamp = stamp(&self.cert, &self.key);
        let mut current = self.current.lock().unwrap();
        if stamp != current.stamp {
            match read(&self.cert, &self.key, &self.provider) {
                Ok(key) => *current = Loaded { stamp, key },
                // likely half written: the next handshake reads it again
                Err(error) => warn!("keeping the TLS certificate in use: {error:#}"),
            }
        }
        Some(current.key.clone())
    }
}

impl fmt::Debug for Certificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Certificate")
            .field("cert", &self.cert)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

fn stamp(cert: &Path, key: &Path) -> Option<(SystemTime, SystemTime)> {
    let modified = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    Some((modified(cert)?, modified(key)?))
}

fn read(cert: &Path, key: &Path, provider: &CryptoProvider) -> Result<Arc<CertifiedKey>> {
    let chain = CertificateDer::pem_file_iter(cert)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| eyre!("invalid certificate {}: {error}", cert.display()))?;
    if chain.is_empty() {
        return Err(eyre!("no certificate in {}", cert.display()));
    }
    let private = PrivateKeyDer::from_pem_file(key)
        .map_err(|error| eyre!("invalid key {}: {error}", key.display()))?;
    let certified = CertifiedKey::from_der(chain, private, provider)
        .wrap_err_with(|| format!("{} is not {}'s key", key.display(), cert.display()))?;
    Ok(Arc::new(certified))
}

/// connections that finished their TLS handshake. each handshake runs on its
/// own, so a peer that stalls in one holds up nobody else
pub struct TlsListener {
    accepted: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl TlsListener {
    /// how long a peer may take over its handshake
    const HANDSHAKE: Duration = Duration::from_secs(10);

    pub fn new(listener: TcpListener, acceptor: TlsAcceptor) -> io::Result<Self> {
        let local = listener.local_addr()?;
        let (sender, accepted) = mpsc::channel(16);
        tokio::spawn(async move {
            loop {
                let (stream, peer) = tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok(accepted) => accepted,
                        Err(error) => {
                            warn!("failed to accept on {local}: {error}");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    },
                    () = sender.closed() => return,
                };
                let acceptor = acceptor.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    if let Ok(Ok(stream)) = timeout(Self::HANDSHAKE, acceptor.accept(stream)).await
                    {
                        let _ = sender.send((stream, peer)).await;
                    }
                });
            }
        });
        Ok(Self { accepted, local })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.accepted.recv().await {
            Some(accepted) => accepted,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}
