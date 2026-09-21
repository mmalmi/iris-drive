//! Opaque backup bytes use the existing Hashtree blob wire and verification.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream};
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, StoreError};

#[derive(Default)]
pub(crate) struct LocalBackupRoute {
    routes: RwLock<Vec<Arc<dyn BlobRoute>>>,
}

impl LocalBackupRoute {
    pub fn set_routes(&self, routes: Vec<Arc<dyn BlobRoute>>) {
        *self
            .routes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = routes;
    }
}

#[async_trait]
impl BlobRoute for LocalBackupRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        let routes = self
            .routes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for route in routes {
            if let reply @ BlobReply::Data(_) =
                route.route(BlobRequest { htl: 0, ..request }).await?
            {
                return Ok(reply);
            }
        }
        Ok(BlobReply::NoResult)
    }
}

/// Reads from explicitly configured friends only, with bounded parallel work.
/// No local cache participates: an audit uses the same peer-specific routes.
#[derive(Default)]
pub(crate) struct FriendReadRoute {
    routes: RwLock<Vec<Arc<dyn BlobRoute>>>,
}

impl FriendReadRoute {
    pub fn set_routes(&self, routes: Vec<Arc<dyn BlobRoute>>) {
        *self
            .routes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = routes;
    }
}

#[async_trait]
impl BlobRoute for FriendReadRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        let routes = self
            .routes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut pending = Vec::with_capacity(routes.len());
        for route in routes {
            let call: futures::future::BoxFuture<'static, Result<BlobReply, StoreError>> =
                Box::pin(async move { route.route(BlobRequest { htl: 0, ..request }).await });
            pending.push(call);
        }
        tokio::time::timeout(Duration::from_secs(8), async move {
            let mut requests = stream::iter(pending).buffer_unordered(4);
            let mut failure = None;
            while let Some(result) = requests.next().await {
                match result {
                    Ok(reply @ BlobReply::Data(_)) => return Ok(reply),
                    Ok(BlobReply::NoResult) => {}
                    Err(error) => failure = Some(error),
                }
            }
            failure.map_or(Ok(BlobReply::NoResult), Err)
        })
        .await
        .map_err(|_| StoreError::Other("friend backup retrieval timed out".into()))?
    }
}
