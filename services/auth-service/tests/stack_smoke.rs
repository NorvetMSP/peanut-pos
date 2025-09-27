use std::io;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use portpicker::pick_unused_port;
use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::{Client, Response};
use serde::Deserialize;
use serde_json::json;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout, Instant};
use uuid::Uuid;

mod support;

use support::{seed_test_user, SeededUser, TestDatabase};

const DEV_PRIVATE_KEY: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/jwt-dev.pem"));
const DEV_PUBLIC_KEY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/jwt-dev.pub.pem"
));

fn log_step(message: impl AsRef<str>) {
    eprintln!("[stack-smoke] {}", message.as_ref());
}

fn env_flag_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key),
        Ok(value) if matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SessionEnvelope {
    token: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
    refresh_expires_in: i64,
    token_type: String,
    access_token_expires_at: String,
    refresh_token_expires_at: String,
    user: SessionUser,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SessionUser {
    id: Uuid,
    tenant_id: Uuid,
    name: String,
    email: String,
    role: String,
}

struct ProcessHandle {
    child: Option<Child>,
}

impl ProcessHandle {
    fn spawn(mut command: Command) -> Result<Self> {
        let child = command
            .spawn()
            .context("failed to spawn auth-service binary")?;
        Ok(Self { child: Some(child) })
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            match child.kill().await {
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::InvalidInput => {}
                Err(err) => return Err(err.into()),
            }
            let _ = child.wait().await;
        }
        Ok(())
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<String>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct TestEnvs {
    _vars: Vec<ScopedEnvVar>,
}

impl TestEnvs {
    fn configure() -> Self {
        let mut vars = Vec::new();

        if env_flag_truthy("AUTH_TEST_USE_EMBED")
            && std::env::var("AUTH_TEST_EMBED_CLEAR_CACHE").is_err()
        {
            log_step("configuring embedded Postgres cache clear");
            vars.push(ScopedEnvVar::set("AUTH_TEST_EMBED_CLEAR_CACHE", "1"));
        }

        Self { _vars: vars }
    }
}

struct ServiceHandle {
    process: ProcessHandle,
    host: String,
    port: u16,
}

impl ServiceHandle {
    async fn launch(database_url: &str, host: &str, port: u16) -> Result<Self> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_auth-service"));
        command.current_dir(env!("CARGO_MANIFEST_DIR"));
        command.env("DATABASE_URL", database_url);
        command.env("JWT_ISSUER", "https://auth.novapos.test");
        command.env("JWT_AUDIENCE", "novapos-api");
        command.env("JWT_DEV_PRIVATE_KEY_PEM", DEV_PRIVATE_KEY);
        command.env("JWT_DEV_PUBLIC_KEY_PEM", DEV_PUBLIC_KEY);
        command.env("HOST", host);
        command.env("PORT", port.to_string());
        let bootstrap =
            std::env::var("AUTH_TEST_KAFKA_BOOTSTRAP").unwrap_or_else(|_| "localhost:19092".into());
        command.env("KAFKA_BOOTSTRAP", bootstrap);
        command.env("AUTH_REQUIRE_MFA", "false");
        command.env("AUTH_MFA_REQUIRED_ROLES", "");
        command.env("AUTH_REFRESH_COOKIE_SECURE", "false");
        command.env("SECURITY_SUSPICIOUS_WEBHOOK_URL", "");
        command.env("SECURITY_SUSPICIOUS_WEBHOOK_BEARER", "");
        command.env("RUST_LOG", "info");

        let process = ProcessHandle::spawn(command)?;
        Ok(Self {
            process,
            host: host.into(),
            port,
        })
    }

    fn url(&self, path: &str) -> String {
        let trimmed = path.trim_start_matches('/');
        format!("http://{}:{}/{}", self.host, self.port, trimmed)
    }

    async fn wait_until_ready(&self, client: &Client) -> Result<()> {
        let health_url = self.url("healthz");
        wait_for_health(client, &health_url).await
    }

    async fn shutdown(self) -> Result<()> {
        self.process.shutdown().await
    }
}

struct StackFixture {
    _envs: TestEnvs,
    database: TestDatabase,
    user: SeededUser,
    client: Client,
    service: ServiceHandle,
}

impl StackFixture {
    async fn bootstrap() -> Result<Option<Self>> {
        let envs = TestEnvs::configure();

        log_step("initializing database for stack smoke test");
        let start = Instant::now();
        let ticker_start = start;
        let ticker = tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(5)).await;
                let elapsed = ticker_start.elapsed().as_secs();
                log_step(format!(
                    "still waiting for database setup ({}s elapsed)",
                    elapsed
                ));
            }
        });

        let setup_result = timeout(Duration::from_secs(120), TestDatabase::setup()).await;
        ticker.abort();

        let setup_outcome = match setup_result {
            Ok(result) => result?,
            Err(_) => {
                log_step("database setup timed out after 120 seconds");
                return Err(anyhow!("timed out waiting for TestDatabase::setup"));
            }
        };

        let Some(database) = setup_outcome else {
            log_step("database unavailable; skipping stack smoke test");
            return Ok(None);
        };
        log_step(format!(
            "database ready after {}s",
            start.elapsed().as_secs()
        ));

        let database_url = database.url().to_string();
        let pool = database.pool_clone();
        log_step("seeding test user");
        let user = seed_test_user(&pool, "manager").await?;
        log_step("test user seeded");
        drop(pool);

        let port = pick_unused_port().context("failed to find available port")?;
        log_step(format!("selected port {} for auth-service", port));
        let host = "127.0.0.1";

        log_step("launching auth-service binary");
        let service = ServiceHandle::launch(&database_url, host, port).await?;
        log_step(format!("auth-service spawned on {}:{}", host, port));

        log_step("building HTTP client");
        let client = Client::builder()
            .build()
            .context("failed to build HTTP client")?;
        log_step("HTTP client ready");

        Ok(Some(Self {
            _envs: envs,
            database,
            user,
            client,
            service,
        }))
    }

    async fn run_happy_path(&mut self) -> Result<()> {
        log_step("waiting for auth-service to report healthy");
        self.service.wait_until_ready(&self.client).await?;
        log_step("auth-service is healthy");

        log_step("executing login request");
        let login = self.perform_login().await?;
        log_step("login request succeeded");

        log_step("executing session refresh request");
        let refreshed = self.fetch_session(&login.refresh_cookie).await?;
        log_step("session refresh request succeeded");

        anyhow::ensure!(
            refreshed.user.id == login.envelope.user.id,
            "refreshed user does not match login user"
        );
        anyhow::ensure!(refreshed.refresh_expires_in > 0, "refresh expiry not set");

        Ok(())
    }

    async fn finish(self) -> Result<()> {
        log_step("shutting down auth-service process");
        let shutdown_result = self.service.shutdown().await;
        match &shutdown_result {
            Ok(_) => log_step("auth-service process stopped"),
            Err(err) => log_step(format!("auth-service shutdown error: {err}")),
        }

        log_step("tearing down database fixture");
        let teardown_result = self.database.teardown().await;
        match &teardown_result {
            Ok(_) => log_step("database teardown complete"),
            Err(err) => log_step(format!("database teardown error: {err}")),
        }

        shutdown_result.and_then(|_| teardown_result)
    }

    async fn perform_login(&self) -> Result<LoginArtifacts> {
        let url = self.service.url("login");
        log_step(format!("POST {}", url));
        let response = self
            .client
            .post(url)
            .json(&json!({
                "email": self.user.email.clone(),
                "password": self.user.password.clone(),
                "tenantId": self.user.tenant_id,
            }))
            .send()
            .await
            .context("failed to send login request")?;

        anyhow::ensure!(
            response.status().is_success(),
            "login request failed: {}",
            response.status()
        );
        log_step("received successful login response");

        let refresh_cookie = extract_refresh_cookie(&response)?;
        let envelope: SessionEnvelope = response
            .json()
            .await
            .context("failed to parse login response")?;

        anyhow::ensure!(!envelope.token.is_empty(), "missing access token");
        anyhow::ensure!(
            envelope.refresh_token.is_none(),
            "unexpected refresh token in body"
        );
        anyhow::ensure!(
            envelope.user.email == self.user.email,
            "unexpected user email"
        );

        Ok(LoginArtifacts {
            refresh_cookie,
            envelope,
        })
    }

    async fn fetch_session(&self, refresh_cookie: &str) -> Result<SessionEnvelope> {
        let url = self.service.url("session");
        log_step(format!("GET {}", url));
        let response = self
            .client
            .get(url)
            .header(COOKIE, refresh_cookie)
            .send()
            .await
            .context("failed to send session refresh request")?;

        anyhow::ensure!(
            response.status().is_success(),
            "refresh request failed: {}",
            response.status()
        );
        log_step("received successful session response");

        response
            .json()
            .await
            .context("failed to parse session response")
    }
}

struct LoginArtifacts {
    refresh_cookie: String,
    envelope: SessionEnvelope,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "launches the auth-service binary"]
async fn stack_smoke_happy_path() -> Result<()> {
    log_step("starting stack_smoke_happy_path");
    let Some(mut fixture) = StackFixture::bootstrap().await? else {
        log_step("stack smoke prerequisites unavailable; test skipped");
        return Ok(());
    };

    let run_result = fixture.run_happy_path().await;
    let cleanup_result = fixture.finish().await;

    match (run_result, cleanup_result) {
        (Ok(_), Ok(())) => {
            log_step("stack_smoke_happy_path completed successfully");
            Ok(())
        }
        (Ok(_), Err(cleanup_err)) => {
            log_step(format!("cleanup failed: {cleanup_err}"));
            Err(cleanup_err)
        }
        (Err(run_err), Ok(_)) => {
            log_step(format!("happy path run failed: {run_err}"));
            Err(run_err)
        }
        (Err(run_err), Err(cleanup_err)) => {
            log_step(format!("happy path run failed: {run_err}"));
            log_step(format!("cleanup failed after run error: {cleanup_err}"));
            Err(run_err)
        }
    }
}

fn extract_refresh_cookie(response: &Response) -> Result<String> {
    let raw = response
        .headers()
        .get(SET_COOKIE)
        .ok_or_else(|| anyhow!("missing refresh cookie header"))?
        .to_str()
        .context("invalid refresh cookie header")?;

    let cookie = raw
        .split(';')
        .next()
        .ok_or_else(|| anyhow!("malformed refresh cookie header"))?
        .to_string();

    anyhow::ensure!(cookie.contains('='), "refresh cookie missing payload");
    Ok(cookie)
}

async fn wait_for_health(client: &Client, url: &str) -> Result<()> {
    log_step(format!("polling {} for healthy status", url));
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut attempt: u32 = 0;
    loop {
        if Instant::now() > deadline {
            log_step("health polling timed out");
            return Err(anyhow!("auth-service did not become healthy in time"));
        }

        attempt += 1;
        match client.get(url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    log_step(format!(
                        "health endpoint returned success after {} attempts",
                        attempt
                    ));
                    return Ok(());
                }
                log_step(format!(
                    "health poll attempt {} returned status {}",
                    attempt,
                    response.status()
                ));
            }
            Err(err) => {
                log_step(format!("health poll attempt {} failed: {}", attempt, err));
            }
        }

        sleep(Duration::from_millis(200)).await;
    }
}
