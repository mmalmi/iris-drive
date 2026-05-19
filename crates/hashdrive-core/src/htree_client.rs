use thiserror::Error;

#[derive(Debug, Error)]
pub enum HtreeClientError {
    #[error("hashtree daemon not reachable")]
    Unreachable,
}

pub struct HtreeClient {
    _endpoint: String,
}

impl HtreeClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            _endpoint: endpoint.into(),
        }
    }
}
