use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures::{future::IntoFuture, Future, Stream};
use parking_lot::Mutex;
use rusoto_core::Region;
use rusoto_s3::S3;
use tokio::codec::{BytesCodec, FramedRead};

use crate::actors::objects::common::prepare_download_paths;
use crate::actors::objects::{DownloadPath, DownloadStream, FileId, ObjectError, ObjectErrorKind};
use crate::types::{FileType, ObjectId, S3SourceConfig, S3SourceKey};

lazy_static::lazy_static! {
    static ref AWS_HTTP_CLIENT: rusoto_core::HttpClient = rusoto_core::HttpClient::new().unwrap();
    static ref S3_CLIENTS: Mutex<lru::LruCache<Arc<S3SourceKey>, Arc<rusoto_s3::S3Client>>> =
        Mutex::new(lru::LruCache::new(100));
}

struct SharedHttpClient;

impl rusoto_core::DispatchSignedRequest for SharedHttpClient {
    type Future = rusoto_core::request::HttpClientFuture;

    fn dispatch(
        &self,
        request: rusoto_core::signature::SignedRequest,
        timeout: Option<Duration>,
    ) -> Self::Future {
        AWS_HTTP_CLIENT.dispatch(request, timeout)
    }
}

fn get_client(key: &Arc<S3SourceKey>) -> Arc<rusoto_s3::S3Client> {
    let end_point = &key.end_point;
    match end_point {
        Some(end_point) => get_minio_clent(key, end_point.to_string()),
        None => get_s3_client(key),
    }
}

fn get_s3_client(key: &Arc<S3SourceKey>) -> Arc<rusoto_s3::S3Client> {
    let mut container = S3_CLIENTS.lock();
    if let Some(client) = container.get(&key) {
        client.clone()
    } else {
        let s3 = Arc::new(rusoto_s3::S3Client::new_with(
            SharedHttpClient,
            rusoto_credential::StaticProvider::new_minimal(
                key.access_key.clone(),
                key.secret_key.clone(),
            ),
            key.region.clone(),
        ));
        container.put(key.clone(), s3.clone());
        log::debug!("Connect Amazon S3 server!");
        s3
    }
}

fn get_minio_clent(key: &Arc<S3SourceKey>, end_point: String) -> Arc<rusoto_s3::S3Client> {
    let mut container = S3_CLIENTS.lock();
    if let Some(client) = container.get(&key) {
        client.clone()
    } else {
        let region = Region::Custom {
            name: key.region.name().to_string(),
            endpoint: end_point.clone(),
        };
        let s3 = Arc::new(rusoto_s3::S3Client::new_with(
            SharedHttpClient,
            rusoto_credential::StaticProvider::new_minimal(
                key.access_key.clone(),
                key.secret_key.clone(),
            ),
            region,
        ));
        container.put(key.clone(), s3.clone());
        log::debug!("Connect MinIO server! (URL: {})", end_point.clone());
        s3
    }
}

pub(super) fn prepare_downloads(
    source: &Arc<S3SourceConfig>,
    filetypes: &'static [FileType],
    object_id: &ObjectId,
) -> Box<Future<Item = Vec<FileId>, Error = ObjectError>> {
    let ids = prepare_download_paths(
        object_id,
        filetypes,
        &source.files.filters,
        source.files.layout,
    )
    .map(|download_path| FileId::S3(source.clone(), download_path))
    .collect();
    Box::new(Ok(ids).into_future())
}

pub(super) fn download_from_source(
    source: Arc<S3SourceConfig>,
    download_path: &DownloadPath,
) -> Box<Future<Item = Option<DownloadStream>, Error = ObjectError>> {
    let key = {
        let prefix = source.prefix.trim_matches(&['/'][..]);
        if prefix.is_empty() {
            download_path.0.clone()
        } else {
            format!("{}/{}", prefix, download_path.0)
        }
    };

    log::debug!("Fetching from s3: {} (from {})", &key, source.bucket);

    let bucket = source.bucket.clone();
    let source_key = &source.source_key;
    let response = get_client(&source_key).get_object(rusoto_s3::GetObjectRequest {
        key: key.clone(),
        bucket: bucket.clone(),
        ..Default::default()
    });

    let response = response.then(move |result| match result {
        Ok(mut result) => {
            let body_read = match result.body.take() {
                Some(body) => body.into_async_read(),
                None => {
                    log::debug!("Empty response from s3:{}{}", bucket, &key);
                    return Ok(None);
                }
            };

            let bytes = FramedRead::new(body_read, BytesCodec::new())
                .map(BytesMut::freeze)
                .map_err(|_err| ObjectError::from(ObjectErrorKind::Io));

            Ok(Some(DownloadStream::FutureStream(
                Box::new(bytes) as Box<dyn Stream<Item = _, Error = _>>
            )))
        }
        Err(err) => {
            log::debug!("Skipping response from s3:{}{}: {}", bucket, &key, err);
            Ok(None)
        }
    });

    Box::new(response)
}
