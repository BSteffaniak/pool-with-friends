//! Native authoritative PWMTF process entry point.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use pwmtf_server::{
    CANONICAL_ORIGIN, GOOGLE_ISSUER, GoogleOidcClient, HttpState, http_router, migrate,
};
use thiserror::Error;

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_DATABASE_PATH: &str = "/data/pwmtf.db";
const DEFAULT_CALLBACK: &str = "https://pwmtf.hyperchad.dev/auth/google/callback";
const OIDC_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("server startup failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), StartupError> {
    let configuration = Configuration::from_environment()?;
    let db: Arc<dyn switchy_database::Database> = switchy_database_connection::builder()
        .turso()
        .with_path(&configuration.database_path)
        .with_busy_timeout(Duration::from_secs(5))
        .build()
        .await?
        .into();
    migrate(&*db).await?;
    let oidc = Arc::new(
        GoogleOidcClient::discover(
            &configuration.google_client_id,
            &configuration.google_client_secret,
            &configuration.google_callback,
        )
        .await?,
    );
    let state = Arc::new(HttpState::production(db, oidc).with_web_root(configuration.web_root));
    state.recover_all_matches().await?;
    let scheduler = tokio::spawn(scheduler_loop(Arc::clone(&state)));
    let listener = tokio::net::TcpListener::bind(configuration.bind).await?;
    axum::serve(listener, http_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    scheduler.abort();
    let _ = scheduler.await;
    Ok(())
}

async fn scheduler_loop(state: Arc<HttpState>) {
    let mut next_oidc_cleanup = 0_u64;
    loop {
        let now = unix_millis();
        if now >= next_oidc_cleanup {
            if let Err(error) = state.cleanup_oidc_attempts(now).await {
                eprintln!("OIDC attempt cleanup failed: {error}");
            }
            next_oidc_cleanup = now.saturating_add(
                u64::try_from(OIDC_CLEANUP_INTERVAL.as_millis()).unwrap_or(u64::MAX),
            );
        }
        if let Err(error) = state
            .poll_due_deadlines(pwmtf_server::DeadlineMillis::new(now))
            .await
        {
            eprintln!("authoritative deadline polling failed: {error}");
        }
        let sleep = state
            .next_deadline()
            .await
            .map_or(Duration::from_secs(1), |deadline| {
                Duration::from_millis(deadline.due_at.value().saturating_sub(unix_millis()).max(1))
            });
        tokio::time::sleep(sleep.min(Duration::from_secs(1))).await;
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

async fn shutdown_signal() {
    let control_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to install Ctrl-C handler: {error}");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                eprintln!("failed to install termination handler: {error}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}

struct Configuration {
    bind: SocketAddr,
    database_path: PathBuf,
    web_root: PathBuf,
    google_client_id: String,
    google_client_secret: String,
    google_callback: String,
}

impl Configuration {
    fn from_environment() -> Result<Self, StartupError> {
        let canonical_origin =
            environment("PWMTF_CANONICAL_ORIGIN").unwrap_or_else(|| CANONICAL_ORIGIN.to_owned());
        if canonical_origin != CANONICAL_ORIGIN {
            return Err(StartupError::Configuration);
        }
        let google_issuer =
            environment("PWMTF_GOOGLE_ISSUER").unwrap_or_else(|| GOOGLE_ISSUER.to_owned());
        if google_issuer != GOOGLE_ISSUER {
            return Err(StartupError::Configuration);
        }
        let callback =
            environment("PWMTF_GOOGLE_CALLBACK").unwrap_or_else(|| DEFAULT_CALLBACK.to_owned());
        if callback != DEFAULT_CALLBACK {
            return Err(StartupError::Configuration);
        }
        Ok(Self {
            bind: environment("PWMTF_BIND")
                .unwrap_or_else(|| DEFAULT_BIND.to_owned())
                .parse()
                .map_err(|_| StartupError::Configuration)?,
            database_path: environment("PWMTF_DATABASE_PATH")
                .unwrap_or_else(|| DEFAULT_DATABASE_PATH.to_owned())
                .into(),
            web_root: environment("PWMTF_WEB_ROOT")
                .unwrap_or_else(|| "/app/dist".to_owned())
                .into(),
            google_client_id: validate_secret_environment("PWMTF_GOOGLE_CLIENT_ID")?,
            google_client_secret: validate_secret_environment("PWMTF_GOOGLE_CLIENT_SECRET")?,
            google_callback: callback,
        })
    }
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn validate_secret_environment(name: &str) -> Result<String, StartupError> {
    let value = environment(name).ok_or(StartupError::Configuration)?;
    validate_secret_value(value)
}

fn validate_secret_value(value: String) -> Result<String, StartupError> {
    if value.len() > 4_096
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        Err(StartupError::Configuration)
    } else {
        Ok(value)
    }
}

#[derive(Debug, Error)]
enum StartupError {
    #[error("server configuration is invalid")]
    Configuration,
    #[error("database initialization failed")]
    Database(#[from] switchy_database_connection::InitTursoError),
    #[error("database migration failed")]
    Migration(#[from] switchy_schema::MigrationError),
    #[error("Google OIDC initialization failed")]
    Oidc(#[from] pwmtf_server::GoogleOidcError),
    #[error("transport recovery failed")]
    Transport(#[from] pwmtf_server::TransportError),
    #[error("server I/O failed")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_configuration_rejects_whitespace_controls_and_oversize_values() {
        assert_eq!(
            validate_secret_value("client-id".to_owned()).unwrap(),
            "client-id"
        );
        for value in [" secret", "secret ", "sec\nret", "two words"] {
            assert!(matches!(
                validate_secret_value(value.to_owned()),
                Err(StartupError::Configuration)
            ));
        }
        assert!(matches!(
            validate_secret_value("x".repeat(4_097)),
            Err(StartupError::Configuration)
        ));
    }

    #[test]
    fn production_constants_use_canonical_origin() {
        assert_eq!(CANONICAL_ORIGIN, "https://pwmtf.hyperchad.dev");
        assert_eq!(
            DEFAULT_CALLBACK,
            format!("{CANONICAL_ORIGIN}/auth/google/callback")
        );
        assert_eq!(GOOGLE_ISSUER, "https://accounts.google.com");
    }
}
