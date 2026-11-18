use std::{net::SocketAddr, path::Path, sync::Arc};

use axum::{Router, extract::ConnectInfo, routing::get};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::TlsConnector;

use crate::web::{
    signin::Caller,
    tls::{Certificate, TlsListener},
};

/// a fresh self-signed certificate for localhost, written to `dir`, and
/// the DER a client trusts it by
fn issue(dir: &Path) -> rustls::pki_types::CertificateDer<'static> {
    let issued = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    std::fs::write(dir.join("cert.pem"), issued.cert.pem()).unwrap();
    std::fs::write(dir.join("key.pem"), issued.signing_key.serialize_pem()).unwrap();
    issued.cert.der().clone()
}

/// what `GET /who` answers over TLS to a client trusting `trusted` alone
async fn ask(
    address: SocketAddr,
    trusted: &rustls::pki_types::CertificateDer<'static>,
) -> std::io::Result<String> {
    let mut roots = RootCertStore::empty();
    roots.add(trusted.clone()).unwrap();
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    let tcp = TcpStream::connect(address).await?;
    let name = ServerName::try_from("localhost").unwrap();
    let mut tls = connector.connect(name, tcp).await?;
    tls.write_all(b"GET /who HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
        .await?;
    let mut answer = String::new();
    tls.read_to_string(&mut answer).await?;
    Ok(answer)
}

async fn serve(dir: &Path) -> SocketAddr {
    let certificate = Certificate::load(&dir.join("cert.pem"), &dir.join("key.pem")).unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener = TlsListener::new(tcp, certificate.acceptor()).unwrap();
    let address = axum::serve::Listener::local_addr(&listener).unwrap();
    let router = Router::new().route(
        "/who",
        get(|ConnectInfo(Caller(peer)): ConnectInfo<Caller>| async move { peer.ip().to_string() }),
    );
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<Caller>(),
        )
        .await
    });
    address
}

#[tokio::test]
async fn it_serves_over_tls_and_knows_the_peer() {
    let dir = tempfile::tempdir().unwrap();
    let trusted = issue(dir.path());
    let address = serve(dir.path()).await;

    let answer = ask(address, &trusted).await.unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("127.0.0.1"), "{answer}");
}

#[tokio::test]
async fn a_renewed_certificate_is_served_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let old = issue(dir.path());
    let address = serve(dir.path()).await;
    assert!(ask(address, &old).await.is_ok());

    let new = issue(dir.path());
    assert!(
        ask(address, &new)
            .await
            .unwrap()
            .starts_with("HTTP/1.1 200")
    );
    assert!(ask(address, &old).await.is_err());

    // a half-written renewal keeps the one in use
    std::fs::write(dir.path().join("cert.pem"), "-----BEGIN CERTIFICATE-----\n").unwrap();
    assert!(ask(address, &new).await.is_ok());
}

#[tokio::test]
async fn a_stalled_handshake_holds_up_no_one() {
    let dir = tempfile::tempdir().unwrap();
    let trusted = issue(dir.path());
    let address = serve(dir.path()).await;

    let _silent = TcpStream::connect(address).await.unwrap();
    let answer = tokio::time::timeout(std::time::Duration::from_secs(5), ask(address, &trusted))
        .await
        .expect("a silent peer held up the handshake");
    assert!(answer.unwrap().starts_with("HTTP/1.1 200"));
}

#[test]
fn a_key_that_is_not_the_certificates_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    issue(dir.path());
    let other = rcgen::KeyPair::generate().unwrap();
    std::fs::write(dir.path().join("other.pem"), other.serialize_pem()).unwrap();

    assert!(
        Certificate::load(&dir.path().join("cert.pem"), &dir.path().join("other.pem")).is_err()
    );
    assert!(Certificate::load(&dir.path().join("cert.pem"), &dir.path().join("none.pem")).is_err());
}
