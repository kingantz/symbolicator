//! Helpers for testing the web server and service.
//!
//! When writing tests, keep the following points in mind:
//!
//!  - In every test, call `test::setup`. This will set up the logger so that all console output is
//!    captured by the test runner.
//!
//!  - Do not use `test::block_on`. This function only initializes a runtime and executor once it
//!    starts polling the future. However, many services require an ambient runtime to spawn
//!    additional futures when they are called. Instead, wrap the entire call to the service into
//!    `test::block_fn`, and return the future from that closure.
//!
//!  - When using `test::tempdir`, make sure that the handle to the temp directory is held for the
//!    entire lifetime of the test. When dropped too early, this might silently leak the temp
//!    directory, since symbolicator will create it again lazily after it has been deleted. To avoid
//!    this, assign it to a variable in the test function (e.g. `let _cache_dir = test::tempdir()`).
//!
//!  - When using `test::symbol_server`, make sure that the server is held until all requests to the
//!    server have been made. If the server is dropped, the ports remain open and all connections
//!    to it will time out. To avoid this, assign it to a variable: `let (_server, source) =
//!    test::symbol_server();`. Alternatively, use `test::local_source()` to test without HTTP
//!    connections.

use std::path::PathBuf;
use std::sync::Arc;

use actix_http::Request;
use actix_service::Service as ActixService;
use actix_web::dev::{Body, ServiceResponse};
use actix_web::{App, Error};
use log::LevelFilter;

use crate::config::Config;
use crate::server::create_app;
use crate::service::Service;
use crate::types::{FilesystemSourceConfig, HttpSourceConfig, SourceConfig};

pub use actix_web::test::*;
pub use tempfile::TempDir;

const SYMBOLS_PATH: &str = "tests/fixtures/symbols";

/// Setup the test environment.
///
///  - Initializes logs: The logger only captures logs from the `symbolicator` crate and mutes all
///    other logs (such as actix or symbolic).
///  - Switches threadpools into test mode: In this mode, threadpools do not actually spawn threads,
///    but instead return the futures that are spawned. This allows to capture console output logged
///    from spawned tasks.
pub(crate) fn setup() {
    env_logger::builder()
        .filter(Some("symbolicator"), LevelFilter::Trace)
        .is_test(true)
        .try_init()
        .ok();

    crate::utils::futures::enable_test_mode();
}

/// Creates a temporary directory.
///
/// The directory is deleted when the `TempDir` instance is dropped, unless `into_path()` is called.
/// Use it as a guard to automatically clean up after tests.
pub(crate) fn tempdir() -> TempDir {
    TempDir::new().unwrap()
}

/// Creates a test service running the full application.
///
/// This service has equivalent behavior to the HTTP server, minus the HTTP part. All middlewares,
/// endpoints and services are registered in the same way as with the production service.
pub(crate) fn test_service(
    config: Config,
) -> impl ActixService<Request = Request, Response = ServiceResponse<Body>, Error = Error> {
    let service = Service::create(config);
    actix_web::test::init_service(create_app(service))
}

/// Get bucket configuration for the local fixtures.
///
/// Files are served directly via the local file system without the indirection through a HTTP
/// symbol server. This is the fastest way for testing, but also avoids certain code paths.
pub(crate) fn local_source() -> SourceConfig {
    SourceConfig::Filesystem(Arc::new(FilesystemSourceConfig {
        id: "local".to_owned(),
        path: PathBuf::from(SYMBOLS_PATH),
        files: Default::default(),
    }))
}

/// Spawn an actual HTTP symbol server for local fixtures.
///
/// The symbol server serves static files from the local symbols fixture location under the
/// `/download` prefix. The layout of this folder is `Native`. This function returns the test server
/// as well as a source configuration, which can be used to access the symbol server in
/// symbolication requests.
///
/// **Note**: The symbol server runs on localhost. By default, connections to local host are not
/// permitted, and need to be activated via `Config::connect_to_reserved_ips`.
pub(crate) fn symbol_server() -> (actix_http_test::TestServerRuntime, SourceConfig) {
    let server = actix_http_test::TestServer::new(|| {
        actix_http::HttpService::new(
            App::new().service(actix_files::Files::new("/download/", SYMBOLS_PATH)),
        )
    });

    // The source uses the same identifier ("local") as the local file system source to avoid
    // differences when changing the bucket in tests.

    let source = SourceConfig::Http(Arc::new(HttpSourceConfig {
        id: "local".to_owned(),
        url: server.url("/download/").parse().unwrap(),
        headers: Default::default(),
        files: Default::default(),
    }));

    (server, source)
}
