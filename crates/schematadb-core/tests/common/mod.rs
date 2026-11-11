#![allow(dead_code)]
use schematadb_core::{ConnectParams, Driver, Engine, connect};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt, ReuseDirective};

pub const ROOT_PASSWORD: &str = "schematadb";

/// A fresh, isolated database on one engine, plus the driver connected to it.
pub struct TestDb {
    pub driver: Arc<dyn Driver>,
    pub db: String,
    pub engine: Engine,
    pub server: Option<Server>,
    _dir: Option<tempfile::TempDir>,
}

#[derive(Clone, Copy, Debug)]
pub struct Server {
    pub engine: Engine,
    pub port: u16,
}

impl Server {
    pub fn params(&self, database: Option<&str>) -> ConnectParams {
        ConnectParams {
            engine: self.engine,
            host: Some("127.0.0.1".into()),
            port: Some(self.port),
            user: Some("root".into()),
            password: Some(ROOT_PASSWORD.into()),
            database: database.map(str::to_owned),
            file_path: None,
            create_file: false,
        }
    }
}

/// Starts (or reuses) a long-lived container for the engine and waits until it accepts connections.
fn server(engine: Engine) -> Server {
    static MYSQL: OnceLock<u16> = OnceLock::new();
    static MARIADB: OnceLock<u16> = OnceLock::new();
    let cell = if engine == Engine::MySql { &MYSQL } else { &MARIADB };
    let port = *cell.get_or_init(|| {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let (image, tag, name, pw_var) = match engine {
                    Engine::MySql => ("mysql", "8.4", "schematadb-test-mysql", "MYSQL_ROOT_PASSWORD"),
                    _ => ("mariadb", "11.4", "schematadb-test-mariadb", "MARIADB_ROOT_PASSWORD"),
                };
                let container = GenericImage::new(image, tag)
                    .with_exposed_port(ContainerPort::Tcp(3306))
                    .with_wait_for(WaitFor::Nothing)
                    .with_env_var(pw_var, ROOT_PASSWORD)
                    .with_container_name(name)
                    .with_reuse(ReuseDirective::Always)
                    .start()
                    .await
                    .expect("start database container (is Docker running?)");
                let port = container.get_host_port_ipv4(3306).await.unwrap();
                let probe = Server { engine, port };
                for _ in 0..120 {
                    if connect(&probe.params(None)).await.is_ok() {
                        return port;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                panic!("{image} container did not become ready");
            })
        })
        .join()
        .unwrap()
    });
    Server { engine, port }
}

impl TestDb {
    pub async fn sqlite() -> TestDb {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let driver = connect(&ConnectParams::sqlite(path.to_string_lossy().into_owned(), true)).await.unwrap();
        TestDb { driver, db: "main".into(), engine: Engine::Sqlite, server: None, _dir: Some(dir) }
    }

    pub async fn mysql() -> TestDb {
        Self::server_db(Engine::MySql).await
    }

    pub async fn mariadb() -> TestDb {
        Self::server_db(Engine::MariaDb).await
    }

    async fn server_db(engine: Engine) -> TestDb {
        let server = tokio::task::spawn_blocking(move || server(engine)).await.unwrap();
        let db = format!("t_{}", uuid_like());
        let admin = connect(&server.params(None)).await.unwrap();
        let r = admin.execute_script(None, &format!("CREATE DATABASE `{db}` CHARACTER SET utf8mb4"), 10, None).await.unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        let driver = connect(&server.params(Some(&db))).await.unwrap();
        TestDb { driver, db, engine, server: Some(server), _dir: None }
    }

    /// Runs SQL on the test database, panicking on failure.
    pub async fn exec(&self, sql: &str) {
        let result = self.driver.execute_script(Some(&self.db), sql, 1000, None).await.unwrap();
        if let Some(err) = result.error {
            panic!("setup SQL failed: {} ({})", err.error.message, err.statement);
        }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if let Some(server) = self.server {
            let db = self.db.clone();
            // Best-effort cleanup on a separate runtime; Drop cannot be async.
            let _ = std::thread::spawn(move || {
                tokio::runtime::Runtime::new().unwrap().block_on(async move {
                    if let Ok(admin) = connect(&server.params(None)).await {
                        let _ = admin.execute_script(None, &format!("DROP DATABASE IF EXISTS `{db}`"), 1, None).await;
                    }
                })
            })
            .join();
        }
    }
}

fn uuid_like() -> String {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{:x}{}{n}", t, std::process::id())
}

/// Generates one `#[tokio::test]` per engine for a scenario `async fn name(t: TestDb)`.
#[macro_export]
macro_rules! scenario {
    ($name:ident) => {
        mod $name {
            use super::*;
            #[tokio::test(flavor = "multi_thread")]
            async fn sqlite() {
                super::$name(TestDb::sqlite().await).await
            }
            #[tokio::test(flavor = "multi_thread")]
            async fn mysql() {
                super::$name(TestDb::mysql().await).await
            }
            #[tokio::test(flavor = "multi_thread")]
            async fn mariadb() {
                super::$name(TestDb::mariadb().await).await
            }
        }
    };
}
