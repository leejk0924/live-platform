use std::{
    env,
    ffi::OsStr,
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    process::ExitCode,
    time::Duration,
};

use api::config::listen_addr;
use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let addr = match listen_addr(env::var("API_ADDR")) {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("invalid API_ADDR: {err}");
            return ExitCode::FAILURE;
        }
    };
    let command = env::args_os().nth(1);
    match command.as_deref().map(OsStr::to_str) {
        None => match serve(addr) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("server error: {err}");
                ExitCode::FAILURE
            }
        },
        Some(Some("healthcheck")) => healthcheck(addr),
        Some(_) => {
            eprintln!("unknown command: {:?}", command.unwrap_or_default());
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn serve(addr: SocketAddr) -> io::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await
}

fn app() -> Router {
    Router::new().nest("/api", Router::new().route("/healthz", get(healthz)))
}

async fn healthz() -> &'static str {
    "ok"
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
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn healthz_returns_ok() {
        let response = app()
            .oneshot(Request::get("/api/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn healthcheck_probe_reports_server_health() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app()).await });

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
