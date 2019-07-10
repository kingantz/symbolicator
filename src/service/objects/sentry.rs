use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_web::{client::Client, http::header};

use futures::future::Either;
use futures::{future, Future, Stream};
use parking_lot::Mutex;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;
use url::Url;

use crate::actors::objects::{DownloadStream, FileId, ObjectError, SentryFileId, USER_AGENT};
use crate::types::{FileType, ObjectId, SentrySourceConfig};

lazy_static::lazy_static! {
    static ref SENTRY_SEARCH_RESULTS: Mutex<lru::LruCache<SearchQuery, (Instant, Vec<SearchResult>)>> =
        Mutex::new(lru::LruCache::new(2000));
}

// #[derive(Debug, Fail, Clone, Copy)]
// pub enum SentryErrorKind {
//     #[fail(display = "failed parsing JSON response from Sentry")]
//     Parsing,

//     #[fail(display = "bad status code from Sentry")]
//     BadStatusCode,

//     #[fail(display = "failed sending request to Sentry")]
//     SendRequest,
// }

#[derive(Clone, Debug, serde::Deserialize)]
struct SearchResult {
    id: String,
    // TODO: Add more fields
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SearchQuery {
    index_url: Url,
    token: String,
}

// TODO(ja): Bring back?
// symbolic::common::derive_failure!(
//     SentryError,
//     SentryErrorKind,
//     doc = "Errors happening while fetching data from Sentry"
// );

fn perform_search(
    query: SearchQuery,
) -> Box<dyn Future<Item = Vec<SearchResult>, Error = ObjectError>> {
    if let Some((created, entries)) = SENTRY_SEARCH_RESULTS.lock().get(&query) {
        if created.elapsed() < Duration::from_secs(3600) {
            return Box::new(future::ok(entries.clone()));
        }
    }

    let index_url = query.index_url.clone();
    let token = query.token.clone();

    log::debug!("Fetching list of Sentry debug files from {}", index_url);
    let index_request = move || {
        let client = Client::new(); // TODO(ja): Get a shared client somehow.
        client
            .get(index_url.as_str())
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::AUTHORIZATION, format!("Bearer {}", token.clone()))
            .send()
            .map_err(ObjectError::io)
            .and_then(move |response| {
                if response.status().is_success() {
                    log::trace!("Success fetching index from Sentry");
                    Either::A(
                        response
                            .json::<Vec<SearchResult>>()
                            .map_err(ObjectError::io),
                    )
                } else {
                    let message = format!("Sentry returned status code {}", response.status());
                    log::warn!("{}", message);
                    Either::B(future::err(ObjectError::io(message)))
                }
            })
    };

    let retries = ExponentialBackoff::from_millis(10).map(jitter).take(3);
    let index_request = Retry::spawn(retries, index_request)
        .map_err(|e| match e {
            tokio_retry::Error::OperationError(e) => e,
            tokio_retry::Error::TimerError(_) => unreachable!(),
        })
        .inspect(move |entries| {
            SENTRY_SEARCH_RESULTS
                .lock()
                .put(query, (Instant::now(), entries.clone()));
        });

    Box::new(future_metrics!(
        "downloads.sentry.index",
        None,
        index_request
    ))
}

pub(super) fn prepare_downloads(
    source: &Arc<SentrySourceConfig>,
    _filetypes: &'static [FileType],
    object_id: &ObjectId,
) -> Box<dyn Future<Item = Vec<FileId>, Error = ObjectError>> {
    let index_url = {
        let mut url = source.url.clone();
        if let Some(ref debug_id) = object_id.debug_id {
            url.query_pairs_mut()
                .append_pair("debug_id", &debug_id.to_string());
        }

        if let Some(ref code_id) = object_id.code_id {
            url.query_pairs_mut()
                .append_pair("code_id", &code_id.to_string());
        }

        url
    };

    let query = SearchQuery {
        index_url,
        token: source.token.clone(),
    };

    let entries = perform_search(query.clone()).map(clone!(source, |entries| {
        entries
            .into_iter()
            .map(move |api_response| FileId::Sentry(source.clone(), SentryFileId(api_response.id)))
            .collect()
    }));

    Box::new(entries)
}

pub(super) fn download_from_source(
    source: Arc<SentrySourceConfig>,
    file_id: &SentryFileId,
) -> Box<dyn Future<Item = Option<DownloadStream>, Error = ObjectError>> {
    let download_url = {
        let mut url = source.url.clone();
        url.query_pairs_mut().append_pair("id", &file_id.0);
        url
    };

    log::debug!("Fetching debug file from {}", download_url);
    let token = &source.token;
    let client = Client::default(); // TODO(ja): Get a shared client somehow
    let try_download = move || {
        client
            .get(download_url.as_str())
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            // TODO(ja): Verify the below assumption.
            // This timeout is for the entire HTTP download *including* the response stream
            // itself, in contrast to what the Actix-Web docs say. We have tested this
            // manually.
            //
            // The intent is to disable the timeout entirely, but there is no API for that.
            .timeout(Duration::from_secs(9999))
            .send()
    };

    let response = Retry::spawn(
        ExponentialBackoff::from_millis(10).map(jitter).take(3),
        try_download,
    );

    let response = response.map_err(|e| match e {
        tokio_retry::Error::OperationError(e) => e,
        e => panic!("{}", e),
    });

    let response = response.then(move |result| match result {
        Ok(response) => {
            if response.status().is_success() {
                log::trace!("Success hitting {}", download_url);
                let stream = Box::new(response.map_err(ObjectError::io));
                Ok(Some(DownloadStream::FutureStream(stream)))
            } else {
                log::debug!(
                    "Unexpected status code from {}: {}",
                    download_url,
                    response.status()
                );
                Ok(None)
            }
        }
        Err(e) => {
            log::warn!("Skipping response from {}: {}", download_url, e);
            Ok(None)
        }
    });

    Box::new(response)
}
