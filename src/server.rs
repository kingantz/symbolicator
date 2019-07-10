use actix_rt::System;
use actix_web::{App, HttpServer};

use crate::config::Config;
use crate::endpoints;
use crate::middleware;

/// Starts all actors and HTTP server based on loaded config.
pub fn run(config: Config) -> Result<(), CliError> {
    let mut sys = System::new("symbolicator");

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Sentry)
            .wrap(middleware::RequestMetrics)
            // TODO: ErrorHandlers
            // TODO: data
            .configure(endpoints::configure)
    })
    .bind(&state.config.bind)?
    .start();

    log::info!("Started http server: {}", state.config.bind);

    sys.run()?;
    Ok(())
}
