use std::sync::Arc;
use std::time::Duration;

use actix_web::{client::Client, http::header};
use futures::{future, Future, IntoFuture, Stream};
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;

use crate::actors::objects::common::prepare_download_paths;
use crate::actors::objects::{DownloadPath, DownloadStream, FileId, ObjectError, USER_AGENT};
use crate::http;
use crate::types::{FileType, HttpSourceConfig, ObjectId};

const MAX_HTTP_REDIRECTS: usize = 10;

pub(super) fn prepare_downloads(
    source: &Arc<HttpSourceConfig>,
    filetypes: &'static [FileType],
    object_id: &ObjectId,
) -> Box<dyn Future<Item = Vec<FileId>, Error = ObjectError>> {
    let ids = prepare_download_paths(
        object_id,
        filetypes,
        &source.files.filters,
        source.files.layout,
    )
    .map(|download_path| FileId::Http(source.clone(), download_path))
    .collect();

    Box::new(Ok(ids).into_future())
}

pub(super) fn download_from_source(
    source: Arc<HttpSourceConfig>,
    download_path: &DownloadPath,
) -> Box<dyn Future<Item = Option<DownloadStream>, Error = ObjectError>> {
    // XXX: Probably should send an error if the URL turns out to be invalid
    let download_url = match source.url.join(&download_path.0) {
        Ok(x) => x,
        Err(_) => return Box::new(future::ok(None)),
    };

    log::debug!("Fetching debug file from {}", download_url);
    let client = Client::new(); // TODO(ja): Create global client.

    let try_download = clone!(download_url, source, || {
        http::follow_redirects(download_url, MAX_HTTP_REDIRECTS, move |url| {
            let mut request = client.get(url);

            for (key, value) in &source.headers {
                if let Ok(header) = header::HeaderName::from_bytes(key.as_bytes()) {
                    request = request.header(header, value.as_str());
                }
            }

            request
                .header(header::USER_AGENT, USER_AGENT)
                // This timeout is for the entire HTTP download *including* the response stream
                // itself, in contrast to what the Actix-Web docs say. We have tested this manually.
                //
                // The intent is to disable the timeout entirely, but there is no API for that.
                .timeout(Duration::from_secs(9999))
        })
    });

    let retries = ExponentialBackoff::from_millis(10).map(jitter).take(3);
    let response = Retry::spawn(retries, try_download)
        .map_err(|e| match e {
            tokio_retry::Error::OperationError(e) => e,
            tokio_retry::Error::TimerError(_) => unreachable!(),
        })
        .then(move |result| match result {
            Ok(response) => {
                if response.status().is_success() {
                    log::trace!("Success hitting {}", download_url);
                    let stream = Box::new(response.map_err(ObjectError::io));
                    Ok(Some(DownloadStream::FutureStream(stream)))
                } else {
                    log::trace!(
                        "Unexpected status code from {}: {}",
                        download_url,
                        response.status()
                    );
                    Ok(None)
                }
            }
            Err(e) => {
                log::trace!("Skipping response from {}: {}", download_url, e);
                Ok(None)
            }
        });

    Box::new(response)
}
