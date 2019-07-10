use std::sync::Arc;

use actix_web::{error, web, Error, FromRequest};
use futures::Future;
use sentry::configure_scope;
use serde::Deserialize;

use crate::actors::symbolication::{GetSymbolicationStatus, SymbolicateStacktraces};
use crate::app::ServiceState;
use crate::sentry::WriteSentryScope;
use crate::types::{
    RawObjectInfo, RawStacktrace, Scope, Signal, SourceConfig, SymbolicationResponse,
};

/// Query parameters of the symbolication request.
#[derive(Deserialize)]
pub struct SymbolicationRequestQueryParams {
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub scope: Scope,
}

impl WriteSentryScope for SymbolicationRequestQueryParams {
    fn write_sentry_scope(&self, scope: &mut sentry::Scope) {
        scope.set_tag("request.scope", &self.scope);
        if let Some(timeout) = self.timeout {
            scope.set_tag("request.timeout", timeout);
        } else {
            scope.set_tag("request.timeout", "none");
        }
    }
}

/// JSON body of the symbolication request.
#[derive(Deserialize)]
struct SymbolicationRequestBody {
    #[serde(default)]
    pub signal: Option<Signal>,
    #[serde(default)]
    pub sources: Option<Vec<SourceConfig>>,
    #[serde(default)]
    pub stacktraces: Vec<RawStacktrace>,
    #[serde(default)]
    pub modules: Vec<RawObjectInfo>,
}

fn post_payload(
    state: web::Data<ServiceState>,
    params: web::Query<SymbolicationRequestQueryParams>,
    body: web::Json<SymbolicationRequestBody>,
) -> Box<dyn Future<Item = web::Json<SymbolicationResponse>, Error = Error>> {
    let params = params.into_inner();
    configure_scope(|scope| params.write_sentry_scope(scope));

    let body = body.into_inner();
    let message = SymbolicateStacktraces {
        signal: body.signal,
        sources: match body.sources {
            Some(sources) => Arc::new(sources),
            None => state.config.sources.clone(),
        },
        stacktraces: body.stacktraces,
        modules: body.modules.into_iter().map(From::from).collect(),
        scope: params.scope,
    };

    let symbolication = state.symbolication;
    let request_id = tryf!(symbolication.symbolicate_stacktraces(message));
    let timeout = params.timeout;

    let future = symbolication
        .get_symbolication_status(GetSymbolicationStatus {
            request_id,
            timeout,
        })
        .then(|result| match result {
            Ok(Some(response)) => Ok(web::Json(response)),
            Ok(None) => Err(error::ErrorInternalServerError(
                "symbolication request did not start",
            )),
            Err(error) => Err(error::ErrorInternalServerError(error)),
        });

    Box::new(future)
}

/// Adds the payload symbolication endpoint to the app.
pub fn configure(config: &mut web::ServiceConfig) {
    let body_config = web::Json::<SymbolicationRequestBody>::configure(|cfg| cfg.limit(5_000_000));

    let resource = web::resource("/symbolicate")
        .route(web::post().to(post_payload))
        .data(body_config);

    config.service(resource);
}
