use anyhow::Context;
use clap::Parser;
use kohiro::auth;
use kohiro::paths::Paths;
use kohiro::server::KohiroServer;
use kohiro::store::Store;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::Server as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

#[derive(Parser)]
struct Cli {
    #[arg(long = "admin-key")]
    admin_key: Option<PathBuf>,
    #[arg(long = "admin-user", default_value = "admin")]
    admin_user: String,
    #[arg(long = "set-public")]
    set_public: Option<String>,
    #[arg(long = "set-private")]
    set_private: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let paths = Arc::new(Paths::new("./data"));
    std::fs::create_dir_all(paths.ssh_dir()).context("create SSH data directory")?;
    std::fs::create_dir_all(paths.repos_dir()).context("create repos data directory")?;
    std::fs::create_dir_all(paths.data_dir.join("myque")).context("create myque data directory")?;

    let store = Arc::new(Store::open(&paths.db_path()).context("open store")?);

    if let Some(admin_key) = cli.admin_key.as_deref() {
        bootstrap_admin(&store, &cli.admin_user, admin_key).context("bootstrap admin")?;
    }

    if let Some(repo) = cli.set_public.as_deref() {
        let (owner, name) = auth::parse_repo(repo)
            .with_context(|| format!("--set-public: expected owner/name, got {repo:?}"))?;
        store.set_public(&owner, &name, true)?;
        log::info!("marked {repo} public");
        return Ok(());
    }

    if let Some(repo) = cli.set_private.as_deref() {
        let (owner, name) = auth::parse_repo(repo)
            .with_context(|| format!("--set-private: expected owner/name, got {repo:?}"))?;
        store.set_public(&owner, &name, false)?;
        log::info!("marked {repo} private");
        return Ok(());
    }

    let ci_db = Arc::new(chilin::Db::open(&paths.chilin_ci_db_path()).context("open ci db")?);
    ci_db.migrate()?;
    let agent_db =
        Arc::new(chilin::Db::open(&paths.chilin_agent_db_path()).context("open agent db")?);
    agent_db.migrate()?;
    tokio::spawn(chilin::run_worker(
        ci_db.clone(),
        build_runner(&env_image("KOHIRO_CI_IMAGE")),
        Duration::from_secs(2),
    ));
    tokio::spawn(chilin::run_worker(
        agent_db.clone(),
        build_runner(&env_image("KOHIRO_AGENT_IMAGE")),
        Duration::from_secs(2),
    ));

    let host_key = load_or_create_host_key(&paths.host_key_path()).context("load host key")?;
    let config = russh::server::Config {
        keys: vec![host_key],
        inactivity_timeout: Some(Duration::from_secs(3600)),
        methods: russh::MethodSet::PUBLICKEY,
        ..Default::default()
    };

    let mut srv = KohiroServer {
        store,
        paths,
        ci_db,
        agent_db,
    };
    log::info!("kohiro listening on 0.0.0.0:2222");
    srv.run_on_address(Arc::new(config), ("0.0.0.0", 2222))
        .await?;
    Ok(())
}

fn env_image(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| "alpine:3.19".into())
}

fn build_runner(image: &str) -> Arc<dyn chilin::Runner> {
    let runtime = std::env::var("KOHIRO_CI_RUNTIME").unwrap_or_else(|_| "docker".into());
    if runtime == "shell" {
        return Arc::new(chilin::ShellRunner);
    }
    let extra_args = std::env::var("KOHIRO_CI_EXTRA_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    Arc::new(chilin::ContainerRunner {
        runtime,
        image: image.to_owned(),
        extra_args,
    })
}

fn bootstrap_admin(store: &Store, username: &str, key_file: &Path) -> anyhow::Result<()> {
    let data = std::fs::read_to_string(key_file)
        .with_context(|| format!("read admin key {}", key_file.display()))?;
    let key = PublicKey::from_openssh(&data)?;
    let comment = if key.comment().is_empty() {
        username
    } else {
        key.comment()
    };
    let fp = auth::fingerprint_of(&key);
    store.bootstrap(username, &fp, comment)?;
    Ok(())
}

fn load_or_create_host_key(path: &Path) -> anyhow::Result<PrivateKey> {
    if path.exists() {
        return Ok(PrivateKey::read_openssh_file(path)?);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let key = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519)?;
    let pem = key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(pem.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, pem.as_bytes())?;
    }

    Ok(key)
}
