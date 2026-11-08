use std::{
    env,
    error::Error,
    ffi::OsStr,
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use api::{
    AppState, app,
    auth::AuthPorts,
    broadcast::BroadcastPorts,
    comment::CommentPorts,
    config::{Config, listen_addr},
    reaper,
};
use application::reaper::Reaper;
use infrastructure::{
    auth::{Argon2Hasher, BreachedPasswordList},
    id::SnowflakeIdGenerator,
    media::{
        JwtPublishAuthorizer, MediaMtxControlApi, MediaMtxIngestRouter, MediaMtxPlaybackResolver,
    },
    persistence::{
        self, PgBroadcastQuery, PgBroadcastRepository, PgChatPolicyStore, PgCommentQuery,
        PgCommentRepository, PgPublishTicketLedger, PgReaperStore, PgSessionStore,
        PgUserRepository, PgWorkerIdAllocator,
    },
    rate_limit::InMemoryRateLimiter,
    realtime::PgEventBus,
};
use tokio::{net::TcpListener, sync::oneshot};
use tracing_subscriber::EnvFilter;

const DB_MAX_CONNECTIONS: u32 = 10;
const RATE_LIMIT_CLEANUP: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let command = env::args_os().nth(1);
    match command.as_deref().map(OsStr::to_str) {
        None => {
            let config = match Config::from_lookup(|key| env::var(key)) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("invalid configuration: {err}");
                    return ExitCode::FAILURE;
                }
            };
            match serve(config) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("server error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Some("healthcheck")) => match listen_addr(env::var("API_ADDR")) {
            Ok(addr) => healthcheck(addr),
            Err(err) => {
                eprintln!("invalid API_ADDR: {err}");
                ExitCode::FAILURE
            }
        },
        Some(_) => {
            eprintln!("unknown command: {:?}", command.unwrap_or_default());
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn serve(config: Config) -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    tracing::info!(?config, "starting");

    let listener = TcpListener::bind(config.listen_addr).await?;
    let pool = persistence::connect(&config.database_url, DB_MAX_CONNECTIONS)
        .await
        .map_err(|err| format!("database connection failed: {err}"))?;
    persistence::migrate(&pool)
        .await
        .map_err(|err| format!("database migration failed: {err}"))?;
    let hasher = tokio::task::spawn_blocking(Argon2Hasher::new)
        .await?
        .map_err(|err| format!("password hasher start failed: {err}"))?;
    let allocator = Arc::new(PgWorkerIdAllocator::new(
        pool.clone(),
        config.worker_lease_ttl,
    ));
    let (ids, heartbeat) = SnowflakeIdGenerator::start(allocator, config.worker_lease_ttl)
        .await
        .map_err(|err| format!("id generator start failed: {err}"))?;

    let ports = AuthPorts {
        ids,
        users: Arc::new(PgUserRepository::new(pool.clone())),
        sessions: Arc::new(PgSessionStore::new(pool.clone())),
        hasher: Arc::new(hasher),
        breached: Arc::new(BreachedPasswordList::load()),
    };
    let publish_jwk = config.publish_key.public_jwk();
    let publish_key = Arc::new(config.publish_key);
    let control_api = Arc::new(MediaMtxControlApi::new(
        config.mediamtx_api_url,
        publish_key.clone(),
    )?);
    let tokens = Arc::new(JwtPublishAuthorizer::new(publish_key));
    let broadcasts = Arc::new(PgBroadcastRepository::new(pool.clone()));
    let broadcast_ports = BroadcastPorts {
        repository: broadcasts.clone(),
        query: Arc::new(PgBroadcastQuery::new(pool.clone())),
        ingest: Arc::new(MediaMtxIngestRouter),
        playback: Arc::new(MediaMtxPlaybackResolver),
        authorizer: tokens.clone(),
        terminator: control_api.clone(),
        verifier: tokens,
        tickets: Arc::new(PgPublishTicketLedger::new(pool.clone())),
        presence: broadcasts,
    };
    let (events, event_listener) = match PgEventBus::start(pool.clone()).await {
        Ok(started) => started,
        Err(err) => {
            if let Err(release_err) = heartbeat.shutdown().await {
                tracing::warn!(%release_err, "could not release worker id lease");
            }
            return Err(format!("event listener start failed: {err}").into());
        }
    };
    let comment_limiter = Arc::new(InMemoryRateLimiter::default());
    let comment_ports = CommentPorts {
        repository: Arc::new(PgCommentRepository::new(pool.clone())),
        query: Arc::new(PgCommentQuery::new(pool.clone())),
        policies: Arc::new(PgChatPolicyStore::new(pool.clone())),
        limiter: comment_limiter.clone(),
        events,
    };
    let state = AppState::new(
        ports,
        broadcast_ports,
        comment_ports,
        config.allowed_origins,
        config.demo_login_enabled,
        publish_jwk,
        config.media_secret,
    );
    if let Err(err) = state.auth.demo_log_in.seed().await {
        event_listener.shutdown().await;
        if let Err(release_err) = heartbeat.shutdown().await {
            tracing::warn!(%release_err, "could not release worker id lease");
        }
        return Err(format!("demo account seeding failed: {err}").into());
    }
    let reaper = reaper::spawn(Reaper {
        store: Arc::new(PgReaperStore::new(pool.clone())),
        probe: control_api.clone(),
        ingest: Arc::new(MediaMtxIngestRouter),
        terminator: control_api,
    });
    let limits = Arc::clone(&state.limits);
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(RATE_LIMIT_CLEANUP);
        loop {
            ticks.tick().await;
            limits.forget_idle();
            comment_limiter.forget_idle();
        }
    });

    tracing::info!(addr = %config.listen_addr, "listening");
    let streams = Arc::clone(&state.streams);
    let lease_lost = heartbeat.lease_lost();
    let (lost_tx, mut lost_rx) = oneshot::channel();
    let served = axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::select! {
            () = shutdown_signal() => {}
            () = lease_lost => {
                tracing::error!("worker id lease lost, draining connections before exit");
                let _ = lost_tx.send(());
            }
        }
        // Open event streams never end on their own, and the drain waits
        // for every connection.
        streams.close_all();
    })
    .await;

    reaper.shutdown().await;
    event_listener.shutdown().await;
    match heartbeat.shutdown().await {
        Ok(()) => tracing::info!("worker id lease released"),
        Err(err) => tracing::warn!(%err, "could not release worker id lease"),
    }
    pool.close().await;
    served?;
    if lost_rx.try_recv().is_ok() {
        // Exit non-zero so the container restarts with a fresh lease.
        return Err("worker id lease lost".into());
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received, draining connections");
}

/// The runtime image is distroless (no shell, curl or wget), so the container
/// healthcheck runs this binary in a second mode instead.
fn healthcheck(addr: SocketAddr) -> ExitCode {
    match probe(probe_target(addr)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("healthcheck failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn probe_target(listen: SocketAddr) -> SocketAddr {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, listen.port())
}

fn probe(target: SocketAddr) -> io::Result<()> {
    // connect, write and read each get this budget; together they must stay under
    // the 3s Docker healthcheck timeout so the error message is not lost to a kill.
    let timeout = Duration::from_millis(900);
    let mut stream = TcpStream::connect_timeout(&target, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream
        .write_all(b"GET /api/healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut status_line = Vec::new();
    BufReader::new(stream.take(256)).read_until(b'\n', &mut status_line)?;
    if status_line.starts_with(b"HTTP/1.1 200 ") {
        Ok(())
    } else {
        let received = String::from_utf8_lossy(&status_line);
        Err(io::Error::other(format!(
            "unexpected status line: {:?}",
            received.trim_end()
        )))
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, routing::get};

    use super::*;

    #[tokio::test]
    async fn healthcheck_probe_reports_server_health() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let healthz = Router::new().route("/api/healthz", get(|| async { "ok" }));
        tokio::spawn(async move { axum::serve(listener, healthz).await });

        let healthy = tokio::task::spawn_blocking(move || probe(addr))
            .await
            .unwrap();
        assert!(
            healthy.is_ok(),
            "probe should see 200 from a running server"
        );

        let missing = Router::new();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, missing).await });

        let err = tokio::task::spawn_blocking(move || probe(addr))
            .await
            .unwrap()
            .expect_err("probe should reject a non-200 response");
        assert!(
            err.to_string().contains("HTTP/1.1 404 Not Found"),
            "error should carry the received status line, got: {err}"
        );
    }

    #[test]
    fn probe_target_maps_unspecified_to_loopback() {
        assert_eq!(
            probe_target("0.0.0.0:9000".parse().unwrap()),
            "127.0.0.1:9000".parse().unwrap()
        );
        assert_eq!(
            probe_target("[::]:9000".parse().unwrap()),
            "[::1]:9000".parse().unwrap()
        );
        assert_eq!(
            probe_target("10.0.0.5:9000".parse().unwrap()),
            "10.0.0.5:9000".parse::<SocketAddr>().unwrap()
        );
    }
}
