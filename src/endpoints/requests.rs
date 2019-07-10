use actix_web::{error, web, Error};
use futures::Future;
use serde::Deserialize;

use crate::actors::symbolication::GetSymbolicationStatus;
use crate::app::ServiceState;
use crate::types::{RequestId, SymbolicationResponse};

/// Path parameters of the symbolication poll request.
#[derive(Deserialize)]
struct PollSymbolicationRequestPath {
    pub request_id: RequestId,
}

/// Query parameters of the symbolication poll request.
#[derive(Deserialize)]
struct PollSymbolicationRequestQueryParams {
    #[serde(default)]
    pub timeout: Option<u64>,
}

fn get_request(
    state: web::Data<ServiceState>,
    path: web::Path<PollSymbolicationRequestPath>,
    query: web::Query<PollSymbolicationRequestQueryParams>,
) -> Box<dyn Future<Item = web::Json<SymbolicationResponse>, Error = Error>> {
    let message = GetSymbolicationStatus {
        request_id: path.into_inner().request_id,
        timeout: query.into_inner().timeout,
    };

    let response = state
        .symbolication
        .get_symbolication_status(message)
        .map_err(error::ErrorInternalServerError)
        .and_then(|response_opt| match response_opt {
            Some(response) => Ok(web::Json(response)),
            None => Err(error::ErrorNotFound("Request does not exist")),
        });

    Box::new(response)
}

/// Adds the request poll endpoint to the app.
pub fn configure(config: &mut web::ServiceConfig) {
    config.route("/requests/{request_id}", web::get().to(get_request));
}
