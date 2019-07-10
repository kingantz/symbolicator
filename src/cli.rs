//! Exposes the command line application.
use std::io;
use std::path::{Path, PathBuf};

use failure::Fail;
use structopt::StructOpt;

use crate::cache::{Cache, CleanupError};
use crate::config::{Config, ConfigError};
use crate::logging;
use crate::server;

/// An enum representing a CLI error.
#[derive(Fail, Debug)]
pub enum CliError {
    /// Indicates a server startup error.
    #[fail(display = "Failed to start the server")]
    Startup(#[fail(cause)] io::Error),

    /// Indicates a config parsing error.
    #[fail(display = "Failed loading config")]
    ConfigParsing(#[fail(cause)] ConfigError),

    /// Indicates an IO error accessing the cache.
    #[fail(display = "Failed loading cache dirs")]
    CacheIo(#[fail(cause)] io::Error),

    /// Indicates an error while cleaning up caches.
    #[fail(display = "Failed cleaning up caches")]
    Cleanup(#[fail(cause)] CleanupError),
}

fn get_crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn get_long_crate_version() -> &'static str {
    concat!(
        "version: ",
        env!("CARGO_PKG_VERSION"),
        "\ngit commit: ",
        env!("SYMBOLICATOR_GIT_VERSION")
    )
}

/// Symbolicator commands.
#[derive(StructOpt)]
#[structopt(bin_name = "symbolicator")]
enum Command {
    /// Run the web server.
    #[structopt(name = "run")]
    Run,

    /// Clean local caches.
    #[structopt(name = "cleanup")]
    Cleanup,
}

/// Command line interface parser.
#[derive(StructOpt)]
#[structopt(raw(version = "get_crate_version()"))]
#[structopt(raw(long_version = "get_long_crate_version()"))]
struct Cli {
    /// Path to the configuration file.
    #[structopt(
        long = "config",
        short = "c",
        raw(global = "true"),
        value_name = "FILE"
    )]
    config: Option<PathBuf>,

    /// The command to execute.
    #[structopt(subcommand)]
    command: Command,
}

impl Cli {
    /// Returns the path to the configuration file.
    fn config(&self) -> Option<&Path> {
        self.config.as_ref().map(PathBuf::as_path)
    }
}

/// Runs the main application.
pub fn execute() -> Result<(), CliError> {
    let cli = Cli::from_args();
    let config = Config::get(cli.config())?;

    let _sentry = sentry::init(config.sentry_dsn.clone());
    logging::init_logging(&config);
    sentry::integrations::panic::register_panic_handler();

    match cli.command {
        Command::Run => server::run(config)?,
        Command::Cleanup => cleanup_caches(config)?,
    }

    Ok(())
}

struct Caches {
    objects: Cache,
    object_meta: Cache,
    symcaches: Cache,
    cficaches: Cache,
}

impl Caches {
    fn new(config: &Config) -> Self {
        Caches {
            objects: {
                let path = config.cache_dir.as_ref().map(|x| x.join("./objects/"));
                Cache::new("objects", path, config.caches.downloaded)
            },
            object_meta: {
                let path = config.cache_dir.as_ref().map(|x| x.join("./object_meta/"));
                Cache::new("object_meta", path, config.caches.derived)
            },
            symcaches: {
                let path = config.cache_dir.as_ref().map(|x| x.join("./symcaches/"));
                Cache::new("symcaches", path, config.caches.derived)
            },
            cficaches: {
                let path = config.cache_dir.as_ref().map(|x| x.join("./cficaches/"));
                Cache::new("cficaches", path, config.caches.derived)
            },
        }
    }
}

fn cleanup_caches(config: Config) -> Result<(), CliError> {
    let caches = Caches::new(&config);
    caches.objects.cleanup()?;
    caches.object_meta.cleanup()?;
    caches.symcaches.cleanup()?;
    caches.cficaches.cleanup()?;
    Ok(())
}
