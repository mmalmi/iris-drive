use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareInvite {
    pub share_id: String,
    pub root_pubkey: String,
    pub name: String,
}
