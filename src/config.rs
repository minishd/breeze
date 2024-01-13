use std::{path::PathBuf, time::Duration};

use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr, DurationSeconds};
use tracing_subscriber::filter::LevelFilter;

#[derive(Deserialize)]
pub struct Config {
    pub engine: EngineConfig,
    pub http: HttpConfig,
    pub logger: LoggerConfig,
}

fn default_motd() -> String {
    "breeze file server (v%version%) - currently hosting %uplcount% files".to_string()
}

#[derive(Deserialize)]
pub struct EngineConfig {
    /// The url that the instance of breeze is meant to be accessed from.
    ///
    /// ex: https://picture.wtf would generate links like https://picture.wtf/p/abcdef.png
    pub base_url: String,

    /// Location on disk the uploads are to be saved to
    pub save_path: PathBuf,

    /// Authentication key for new uploads, will be required if this is specified. (optional)
    #[serde(default)]
    pub upload_key: String,

    /// Configuration for cache system
    pub cache: CacheConfig,

    /// Motd displayed when the server's index page is visited.
    ///
    /// This isn't explicitly engine-related but the engine is what gets passed to routes,
    /// so it is here for now.
    #[serde(default = "default_motd")]
    pub motd: String,
}

#[serde_as]
#[derive(Deserialize)]
pub struct CacheConfig {
    /// The maximum length in bytes that a file can be
    /// before it skips cache (in seconds)
    pub max_length: usize,

    /// The amount of time a file can last inside the cache (in seconds)
    #[serde_as(as = "DurationSeconds")]
    pub upload_lifetime: Duration,

    /// How often the cache is to be scanned for
    /// expired entries (in seconds)
    #[serde_as(as = "DurationSeconds")]
    pub scan_freq: Duration,

    /// How much memory the cache is allowed to use (in bytes)
    pub mem_capacity: usize,
}

#[derive(Deserialize)]
pub struct HttpConfig {
    pub listen_on: String,
}

fn default_level_filter() -> LevelFilter {
    LevelFilter::WARN
}

#[serde_as]
#[derive(Deserialize)]
pub struct LoggerConfig {
    /// Minimum level a log must be for it to be shown.
    /// This defaults to "warn" if not specified.
    #[serde_as(as = "DisplayFromStr")]
    // yes... kind of a hack but serde doesn't have anything better
    #[serde(default = "default_level_filter")]
    pub level: LevelFilter,
}
