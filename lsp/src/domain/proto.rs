use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Project {
    pub root: String,
    pub command: String,
    pub args: Vec<String>,
}
