use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: String,
    pub domain: String,
    pub upstream: String,
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyEnvPreset {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Rule {
    Proxy {
        path: String,
        target: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        envs: Vec<ProxyEnvPreset>,
    },
    Static { path: String, root: String },
    Bypass { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub sites: Vec<Site>,
}
