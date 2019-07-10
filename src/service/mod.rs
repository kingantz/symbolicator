use std::sync::Arc;

use tokio_threadpool::ThreadPool;

pub mod cache;
pub mod cficaches;
pub mod objects;
pub mod symbolication;
pub mod symcaches;

pub struct Service {
    config: Arc<Config>,
    cpu_pool: Arc<ThreadPool>,
    io_pool: Arc<ThreadPool>,
    symbolication: Arc<SymbolicationActor>,
    objects: Arc<ObjectsActor>,
}
