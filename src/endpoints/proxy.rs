use std::io::Cursor;

use actix_web::{error, http, web, Error, HttpRequest, HttpResponse};
use bytes::BytesMut;
use failure::Fail;
use futures::{future, Future, Stream};
use serde::Deserialize;
use tokio::codec::{BytesCodec, FramedRead};

use crate::actors::objects::{FindObject, ObjectPurpose};
use crate::app::ServiceState;
use crate::types::Scope;
use crate::utils::paths::parse_symstore_path;

/// Symstore proxy error.
#[derive(Fail, Debug, Clone, Copy)]
pub enum ProxyErrorKind {
    #[fail(display = "failed to write object")]
    Io,

    #[fail(display = "failed to download object")]
    Fetching,
}

symbolic::common::derive_failure!(
    ProxyError,
    ProxyErrorKind,
    doc = "Errors happening while proxying to a symstore"
);

/// Path parameters of the symstore proxy request.
#[derive(Deserialize)]
struct ProxyPath {
    pub path: String,
}

fn get_symstore_proxy(
    state: web::Data<ServiceState>,
    path: web::Path<ProxyPath>,
    request: HttpRequest,
) -> Box<dyn Future<Item = HttpResponse, Error = Error>> {
    let is_head = request.method() == http::Method::HEAD;

    if !state.config.symstore_proxy {
        return Box::new(future::ok(HttpResponse::NotFound().finish()));
    }

    let (filetypes, object_id) = match parse_symstore_path(&path.path) {
        Some((filetypes, object_id)) => (filetypes, object_id),
        None => return Box::new(future::ok(HttpResponse::NotFound().finish())),
    };

    log::debug!("Searching for {:?} ({:?})", object_id, filetypes);

    let objects = state.objects.clone();
    let response = objects
        .find(FindObject {
            filetypes,
            identifier: object_id,
            sources: state.config.sources.clone(),
            scope: Scope::Global,
            purpose: ObjectPurpose::Debug,
        })
        .and_then(|meta_opt| match meta_opt {
            Some(meta) => future::Either::A(objects.fetch(meta).map(Some)),
            None => future::Either::B(future::ok(None)),
        })
        .map_err(error::ErrorInternalServerError)
        .and_then(|object_opt| match object_opt {
            Some(object) if object.has_object() => Ok(object),
            _ => Err(error::ErrorNotFound("File does not exist")),
        })
        .and_then(move |object_file| {
            let mut response = HttpResponse::Ok();
            response
                .content_length(object_file.len() as u64)
                .set(http::header::ContentType::octet_stream());

            if is_head {
                Ok(response.finish())
            } else {
                let bytes = Cursor::new(object_file.data());
                let async_bytes = FramedRead::new(bytes, BytesCodec::new())
                    .map(BytesMut::freeze)
                    .map_err(Error::from);
                Ok(response.streaming(async_bytes))
            }
        });

    Box::new(response)
}

pub fn configure(config: &mut web::ServiceConfig) {
    config.route(
        "/symbols/{path:.+}",
        web::get().method(http::Method::HEAD).to(get_symstore_proxy),
    );
}
