use std::{
    env,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    process::ExitCode,
    time::Duration,
};

use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_ADDR: &str = "0.0.0.0:8080";

fn main() -> ExitCode {
    let addr = match listen_addr() {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("invalid API_ADDR: {err}");
            return ExitCode::FAILURE;
        }
    };
    match env::args().nth(1).as_deref() {
        None => match serve(addr) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("server error: {err}");
                ExitCode::FAILURE
            }
        },
        Some("healthcheck") => healthcheck(addr),
        Some(other) => {
            eprintln!("unknown command: {other}");
            ExitCode::FAILURE
        }
    }
}

fn listen_addr() -> Result<SocketAddr, std::net::AddrParseError> {
    env::var("API_ADDR")
        .unwrap_or_else(|_| DEFAULT_ADDR.to_owned())
        .parse()
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
    let target = SocketAddr::from((Ipv4Addr::LOCALHOST, addr.port()));
    match probe(target) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("healthcheck failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn probe(target: SocketAddr) -> io::Result<bool> {
    let timeout = Duration::from_secs(2);
    let mut stream = TcpStream::connect_timeout(&target, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream
        .write_all(b"GET /api/healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut status_line = [0u8; 12];
    stream.read_exact(&mut status_line)?;
    Ok(&status_line == b"HTTP/1.1 200")
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
            .unwrap()
            .unwrap();
        assert!(healthy, "probe should see 200 from a running server");

        let missing = Router::new();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, missing).await });

        let healthy = tokio::task::spawn_blocking(move || probe(addr))
            .await
            .unwrap()
            .unwrap();
        assert!(!healthy, "probe should reject a non-200 response");
    }
}
