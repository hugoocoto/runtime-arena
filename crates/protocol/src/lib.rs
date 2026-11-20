//! Types shared by the server and the CLI.

use serde::{Deserialize, Serialize};

/// Response of `GET /api/info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: String,
    /// Where the source of the running server can be obtained (AGPL-3.0 section 13).
    pub source_url: String,
}
