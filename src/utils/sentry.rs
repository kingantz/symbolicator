// use std::ops::Deref;
use std::sync::Arc;

// use actix_web::{error, web, Error, FromRequest, HttpRequest};
use futures::{Future, Poll};
use sentry::{Hub, Scope};

// #[derive(Clone, Debug)]
// pub struct RequestHub(pub Arc<Hub>);

// impl RequestHub {
//     pub fn hub(&self) -> Arc<Hub> {
//         self.0.clone()
//     }
// }

// impl Deref for RequestHub {
//     type Target = Hub;

//     fn deref(&self) -> &Hub {
//         &self.0
//     }
// }

// impl Into<Arc<Hub>> for RequestHub {
//     fn into(self) -> Arc<Hub> {
//         self.0
//     }
// }

// impl FromRequest for RequestHub {
//     type Config = ();
//     type Error = Error;
//     type Future = Result<Self, Error>;

//     #[inline]
//     fn from_request(req: &HttpRequest, _: &mut web::Payload) -> Self::Future {
//         match req.extensions().get::<RequestHub>() {
//             Some(hub) => Ok(hub.clone()),
//             None => Err(error::ErrorInternalServerError(
//                 "Sentry hub is not configured, use Sentry middleware",
//             )),
//         }
//     }
// }

#[derive(Debug)]
pub struct SentryFuture<F> {
    pub(crate) hub: Arc<Hub>,
    pub(crate) inner: F,
}

impl<F> Future for SentryFuture<F>
where
    F: Future,
{
    type Item = F::Item;
    type Error = F::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Hub::run(self.hub.clone(), || self.inner.poll())
    }
}

pub trait SentryFutureExt: Sized {
    fn bind_hub<H>(self, hub: H) -> SentryFuture<Self>
    where
        H: Into<Arc<Hub>>,
    {
        SentryFuture {
            inner: self,
            hub: hub.into(),
        }
    }

    // TODO(ja): Remove this
    fn sentry_hub_current(self) -> SentryFuture<Self> {
        self.bind_hub(Hub::current())
    }

    // TODO(ja): Remove this
    fn sentry_hub_new_from_current(self) -> SentryFuture<Self> {
        self.bind_hub(Arc::new(Hub::new_from_top(Hub::current())))
    }
}

impl<F> SentryFutureExt for F {}

/// Write own data to Sentry scope, only the subset that is considered useful for debugging. Right
/// now this could've been a simple method, but the idea is that one day we want a custom derive
/// for this.
pub trait WriteSentryScope {
    fn write_sentry_scope(&self, scope: &mut Scope);
}
