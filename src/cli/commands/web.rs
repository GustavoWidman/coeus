use std::{
    future::Future,
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::Router;
use clap::Args;
use eyre::{Result, WrapErr, bail, eyre};
use log::{info, warn};
use tokio::{
    net::{TcpListener, UnixListener},
    signal::unix::{SignalKind, signal},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

use crate::{
    cli::{exit::Exit, local_hostname},
    client::{Fleet, Tailnet, Targets},
    common::model::Hostname,
    config::{Endpoint, Key, SigninSettings, WebSettings},
    web::{
        connector::{self, Timing},
        fixture::{Fixture, Scenario},
        push::Push,
        ready,
        session::Sessions,
        signin::{Caller, Signin},
        site::{Access, Settings, Site},
        source::Source,
        tls::{Certificate, TlsListener},
        whois::{Policy, Whois},
    },
};

#[derive(Args, Debug)]
pub struct WebArgs {
    /// The built web app, served for every path outside /api
    #[arg(long, value_name = "DIR", env = "COEUS_WEB_DIR")]
    pub web_dir: Option<PathBuf>,

    /// Serve a simulated fleet instead of the real one, signing anyone in on
    /// the same address; POST /fixture/seek?t=<seconds>[&speed=<x>] moves its
    /// clock, and without t holds it or sets its speed where it stands
    #[arg(long, value_name = "SCENARIO")]
    pub fixture: Option<Scenario>,

    /// Where a simulated fleet is served
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "127.0.0.1:7979",
        requires = "fixture"
    )]
    pub listen: SocketAddr,

    /// Let anyone read a simulated fleet without signing in
    #[arg(long, requires = "fixture")]
    pub open: bool,

    /// How fast a simulated fleet's clock runs; 0 holds it
    #[arg(long, default_value_t = 1.0, requires = "fixture")]
    pub speed: f64,
}

impl WebArgs {
    /// serves the fleet named in `config`, or in /etc/coeus/web.toml, on its
    /// origin socket and signing in on this machine's tailnet addresses, or
    /// a simulated one on a local port
    pub async fn run(self, config: Option<&Path>) -> Result<Exit> {
        if let Some(scenario) = self.fixture {
            return self.simulate(scenario).await;
        }

        let settings = WebSettings::load(config.unwrap_or(Path::new(WebSettings::PATH)))?;
        let web = &settings.web;
        let sessions = Arc::new(Sessions::load(&web.session_key_file, web.session_days)?);
        let signin = Signin {
            sessions: sessions.clone(),
            whois: Whois::tailscale(),
            policy: Policy {
                capability: web.signin.capability.clone(),
                logins: web.signin.logins.clone(),
            },
            origin: web.public_origin.clone(),
        }
        .router()?;
        let key = settings.client.key.load()?;
        let targets = Targets::select(&["@all".to_owned()], &settings.client)?;
        if targets.hosts().is_empty() {
            bail!("no machines in the inventory to show");
        }

        let served_by = local_hostname()
            .parse::<Hostname>()
            .wrap_err("this machine's hostname is not a valid name")?;

        let fleet = Fleet::new(settings.client.clone(), key.clone(), targets);
        let source = Source::Live(fleet.clone());
        let site = Site::start(
            fleet.hosts().to_vec(),
            source.clone(),
            Settings {
                web_dir: self.web_dir,
                served_by: served_by.clone(),
                access: Access::Sessions {
                    sessions,
                    signin: web.signin.urls.clone(),
                },
                push: web.push.clone().map(Push::Live),
            },
        );
        source.feed(site.inputs.clone());

        let terminated = terminated()?;
        let socket = &web.origin_socket;
        let listener = bind(socket)?;
        info!("serving on {}", socket.display());

        let stop = CancellationToken::new();
        let tunnel = match &web.tunnel {
            Some(tunnel) => {
                let config = connector::configure(tunnel, socket)?;
                let ready = watch_readiness(&settings, key, &served_by);
                let program = tunnel.cloudflared.clone();
                let stop = stop.clone();
                Some(tokio::spawn(async move {
                    connector::run(&program, &config, Timing::DEFAULT, ready, stop).await;
                }))
            }
            None => {
                info!("no tunnel configured, so nothing reaches the site");
                None
            }
        };

        // cloudflared finishes the requests it carries, which need the
        // origin, before the origin closes
        let done = async {
            terminated.await;
            stop.cancel();
            if let Some(tunnel) = tunnel {
                let _ = tunnel.await;
            }
        };
        let public = async { axum::serve(listener, site.router).await };
        let signing_in = sign_in(signin, &web.signin);
        until(
            async {
                tokio::select! {
                    served = public => served.wrap_err("the site stopped"),
                    served = signing_in => served,
                }
            },
            done,
        )
        .await?;
        let _ = std::fs::remove_file(socket);
        Ok(Exit::Success)
    }

    /// serves a simulated fleet, which never dials a coeusd
    async fn simulate(self, scenario: Scenario) -> Result<Exit> {
        let fixture = Fixture::new(scenario, self.speed);
        let origin = format!("http://{}", self.listen);
        let router = fixture.site(self.web_dir, &origin, self.open)?;

        let listener = TcpListener::bind(self.listen)
            .await
            .wrap_err_with(|| format!("failed to listen on {}", self.listen))?;
        info!("serving the {scenario:?} fixture on {origin}");
        until(
            async {
                axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<Caller>(),
                )
                .await
                .wrap_err("the site stopped")
            },
            terminated()?,
        )
        .await?;
        Ok(Exit::Success)
    }
}

/// serves `router` on each of this machine's tailnet addresses, once
/// tailscale has them and the certificate exists, trying again until then
async fn sign_in(router: Router, settings: &SigninSettings) -> Result<()> {
    let listeners = loop {
        match listen_on_tailnet(settings).await {
            Ok(listeners) => break listeners,
            Err(error) => warn!("not signing in yet: {error:#}"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    };
    let served = listeners.into_iter().map(|listener| {
        let router = router.clone();
        async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<Caller>(),
            )
            .await
        }
    });
    futures_util::future::try_join_all(served)
        .await
        .wrap_err("signing in stopped")?;
    Ok(())
}

/// TLS listeners on every tailnet address of this machine that binds
async fn listen_on_tailnet(settings: &SigninSettings) -> Result<Vec<TlsListener>> {
    let tailnet = Tailnet::read()
        .await
        .ok_or_else(|| eyre!("tailscale did not answer"))?;
    let certificate = Certificate::load(&settings.cert, &settings.key)?;
    let mut listeners = Vec::new();
    for address in tailnet.addresses() {
        let address = SocketAddr::new(address, settings.port);
        match TcpListener::bind(address).await {
            Ok(tcp) => {
                listeners.push(TlsListener::new(tcp, certificate.acceptor())?);
                info!("signing in on https://{address}");
            }
            Err(error) => warn!("failed to listen on {address}: {error}"),
        }
    }
    match listeners.is_empty() {
        true => Err(eyre!("no tailnet address to listen on")),
        false => Ok(listeners),
    }
}

/// binds `socket` for its owner alone, replacing a socket a previous run left
fn bind(socket: &Path) -> Result<UnixListener> {
    if std::fs::symlink_metadata(socket).is_ok_and(|meta| meta.file_type().is_socket()) {
        std::fs::remove_file(socket)
            .wrap_err_with(|| format!("failed to remove {}", socket.display()))?;
    }
    let listener = UnixListener::bind(socket)
        .wrap_err_with(|| format!("failed to listen on {}", socket.display()))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .wrap_err_with(|| format!("failed to restrict {}", socket.display()))?;
    Ok(listener)
}

/// probes whether this machine can serve, its own coeusd reached over IPv4
/// loopback with the read-only key, and says so in the receiver
fn watch_readiness(
    settings: &WebSettings,
    key: Key,
    served_by: &Hostname,
) -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    let client = settings.client.clone();
    let readiness = settings.web.readiness.clone();
    let daemon = Endpoint {
        host: "127.0.0.1".to_owned(),
        port: client.endpoint(served_by).port,
    };
    tokio::spawn(async move {
        ready::watch(readiness, || ready::probe(&client, &daemon, &key), sender).await;
    });
    receiver
}

/// resolves on SIGTERM or SIGINT, whose handlers are installed at once
fn terminated() -> Result<impl Future<Output = ()>> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {},
            _ = interrupt.recv() => {},
        }
    })
}

/// runs `serving` until it stops, or until `done` resolves
async fn until(
    serving: impl Future<Output = Result<()>>,
    done: impl Future<Output = ()>,
) -> Result<()> {
    tokio::select! {
        served = serving => served,
        () = done => Ok(()),
    }
}
