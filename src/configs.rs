use std::net::{IpAddr, Ipv4Addr};

use config::Config;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use subxt_signer::SecretString;

const DEFAULT_CONFIG_DIR_PATH: &str = "configs";

fn format_prefix(prefix: &str, config_prefix: &str) -> String {
    if prefix.is_empty() {
        config_prefix.to_string()
    } else {
        format!("{prefix}_{config_prefix}")
    }
}

fn format_config_path(config_dir_path: &str, config_name: &str) -> String {
    if config_dir_path.is_empty() {
        format!("{DEFAULT_CONFIG_DIR_PATH}/{config_name}")
    } else if config_dir_path.ends_with('/') {
        format!("{config_dir_path}{config_name}")
    } else {
        format!("{config_dir_path}/{config_name}")
    }
}

fn config_from_file_or_env<T: DeserializeOwned>(filename: &str, env_prefix: &str) -> T {
    let config = Config::builder()
        .add_source(config::File::with_name(filename).required(false))
        .add_source(
            config::Environment::with_prefix(env_prefix)
            .try_parsing(true)
            // allow set ChainConfig.endpoints over env vars
            .with_list_parse_key("endpoints")
            .list_separator(","),
        )
        .build()
        .unwrap_or_else(|err| panic!("Failed to read config file: {filename}. Error: {err}"));

    config
        .try_deserialize()
        .unwrap_or_else(|err| panic!("Failed to parse config file: {filename}. Error: {err}"))
}

// TODO: read it directly into SecretString
// TODO: perhaps will add optional password in future
#[derive(Deserialize)]
struct RawSeedConfig {
    seed: String,
}

pub struct SeedConfig {
    pub seed: SecretString,
}

impl From<RawSeedConfig> for SeedConfig {
    fn from(value: RawSeedConfig) -> Self {
        Self {
            seed: SecretString::from(value.seed),
        }
    }
}

// TODO: prefare use smallstrings?
pub type AssetName = String;
pub type AssetId = u32;

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct AssetConfig {
    pub name: AssetName,
    pub id: AssetId,
}

fn default_account_lifetime_millis() -> u64 {
    86_400_000
}

// TODO: add some docs for fields, their purpose might be not obvious
#[derive(Deserialize, Clone, Debug)]
pub struct ChainConfig {
    pub name: String,
    // TODO: try to parse into Url in order to intercept some errors on startup?
    pub endpoints: Vec<String>,
    /// false by default
    #[serde(default)]
    pub allow_insecure_endpoints: bool,
    pub assets: Vec<AssetConfig>,
}

// TODO: add some docs for fields, their purpose might be not obvious
#[derive(Deserialize, Debug)]
pub struct PaymentsConfig {
    // TODO: can we validate it somehow on startup?
    pub recipient: String,
    /// 1 day by default
    #[serde(default = "default_account_lifetime_millis")]
    pub account_lifetime_millis: u64,
    pub remark: Option<String>,
}

fn default_host() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

fn default_port() -> u16 {
    16726
}

// TODO: configure enable/disable health/metrics/etc handlers?
#[derive(Deserialize, Debug)]
pub struct WebServerConfig {
    /// By default use 0.0.0.0
    #[serde(default = "default_host")]
    pub host: IpAddr,
    /// By default use port 16726
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_database_path() -> String {
    "kalatori.db".to_string()
}

#[derive(Deserialize)]
pub struct DatabaseConfig {
    /// `kalatori.db` by default
    #[serde(default = "default_database_path")]
    pub path: String,
    #[serde(default)]
    pub temporary: bool,
}

// TODO: add logger config

pub fn seed_config_with_prefix(config_dir_path: &str, prefix: &str) -> SeedConfig {
    let config_path = format_config_path(config_dir_path, "seed.json");
    let env_prefix = format_prefix(prefix, "SEED");
    let config = config_from_file_or_env::<RawSeedConfig>(&config_path, &env_prefix);

    // Function is unsafe because of potential race conditions in multithreaded environment. We call it at very start
    // of the program before spawn any futures which might cause this error therefore can consider it safe. If you know
    // some better way to handle it (except of forbid to provide seed throgh env var) please let us know.
    unsafe {
        std::env::remove_var(format!("{env_prefix}_SEED"));
    }

    config.into()
}

pub fn chain_config_with_prefix(config_dir_path: &str, prefix: &str) -> ChainConfig {
    let config_path = format_config_path(config_dir_path, "chain.json");
    let env_prefix = format_prefix(prefix, "CHAIN");
    config_from_file_or_env(&config_path, &env_prefix)
}

pub fn payments_config_with_prefix(config_dir_path: &str, prefix: &str) -> PaymentsConfig {
    let config_path = format_config_path(config_dir_path, "payments.json");
    let env_prefix = format_prefix(prefix, "PAYMENTS");
    config_from_file_or_env(&config_path, &env_prefix)
}

pub fn web_server_config_with_prefix(config_dir_path: &str, prefix: &str) -> WebServerConfig {
    let config_path = format_config_path(config_dir_path, "web_server.json");
    let env_prefix = format_prefix(prefix, "WEB_SERVER");
    config_from_file_or_env(&config_path, &env_prefix)
}

pub fn database_config_with_prefix(config_dir_path: &str, prefix: &str) -> DatabaseConfig {
    let config_path = format_config_path(config_dir_path, "database.json");
    let env_prefix = format_prefix(prefix, "DATABASE");
    config_from_file_or_env(&config_path, &env_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serial_test::serial;
    use subxt_signer::ExposeSecret;

    // TODO: those tests suppose that `make copy-configs` was executed. Need somehow ensure that it happend

    #[test]
    #[serial]
    fn test_seed_config_with_prefix() {
        // load from default config dir without any overrides
        {
            let config = seed_config_with_prefix("", "");
            assert_eq!(
                config.seed.expose_secret(),
                "bottom drive obey lake curtain smoke basket hold race lonely fit walk"
            );
        }

        // override seed with env var and ensure this env var was removed after config load
        {
            let value = "test seed";
            unsafe {
                std::env::set_var("SEED_SEED", value);
            }
            let config = seed_config_with_prefix("", "");
            assert_eq!(config.seed.expose_secret(), value);

            let env_var = std::env::var("SEED_SEED");
            assert!(matches!(env_var, Err(std::env::VarError::NotPresent)));
        }

        // same as previous + override env var prefix. Also set some different dir which shouldn't affect anything in this case
        {
            let value = "test seed 2";
            unsafe {
                std::env::set_var("KALATORI_SUPER_PREFIX_SEED_SEED", value);
            }
            let config = seed_config_with_prefix("somewhere-nowhere", "KALATORI_SUPER_PREFIX");
            assert_eq!(config.seed.expose_secret(), value);

            let env_var = std::env::var("KALATORI_SUPER_PREFIX_SEED_SEED");
            assert!(matches!(env_var, Err(std::env::VarError::NotPresent)));
        }
    }

    #[test]
    #[serial]
    fn test_payments_config_with_prefix() {
        // load config from default config dir without any overrides
        {
            let config = payments_config_with_prefix("", "");
            assert_eq!(
                config.account_lifetime_millis,
                default_account_lifetime_millis()
            );

            assert_eq!(
                config.recipient,
                "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
            );

            assert_eq!(config.remark, Some("test".to_string()));
        }

        // override config dir and set `recipient` in env var
        {
            unsafe {
                // we don't validate it currenlty so can set any value
                std::env::set_var("PAYMENTS_RECIPIENT", "test_recipient");
            }

            let config = payments_config_with_prefix("somewhere-nowhere", "");

            assert_eq!(
                config.account_lifetime_millis,
                default_account_lifetime_millis()
            );

            assert_eq!(config.recipient, "test_recipient");
            assert!(config.remark.is_none());
        }

        // override config env prefix
        {
            unsafe {
                std::env::set_var("KALATORI_PAYMENTS_ACCOUNT_LIFETIME_MILLIS", "123");
            }

            let config = payments_config_with_prefix("", "KALATORI");

            assert_eq!(config.account_lifetime_millis, 123);

            assert_eq!(
                config.recipient,
                "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
            );

            assert_eq!(config.remark, Some("test".to_string()));
        }
    }

    #[test]
    #[serial]
    fn test_chain_config_with_prefix() {
        let mut expected_endpoints = vec!["ws://localhost:9000".to_string()];

        let expected_assets = vec![
            AssetConfig {
                name: "USDC".to_string(),
                id: 1337,
            },
            AssetConfig {
                name: "USDt".to_string(),
                id: 1984,
            },
        ];

        // load config from default config dir without any overrides
        {
            let config = chain_config_with_prefix("", "");

            assert_eq!(config.name, "statemint");
            assert_eq!(config.endpoints, expected_endpoints);
            assert_eq!(config.assets, expected_assets);
        }

        // override endpoints with env vars
        {
            unsafe {
                std::env::set_var("CHAIN_ENDPOINTS", "ws://localhost:9000,ws://localhost:9500");
            }

            expected_endpoints.push("ws://localhost:9500".to_string());
            let config = chain_config_with_prefix("", "");
            assert_eq!(config.name, "statemint");
            assert_eq!(config.endpoints, expected_endpoints);
            assert_eq!(config.assets, expected_assets);
        }

        // override env var prefix
        {
            unsafe {
                std::env::set_var("KALATORI_CHAIN_NAME", "kusama");
            }

            let _unused = expected_endpoints.pop();
            let config = chain_config_with_prefix("", "KALATORI");
            assert_eq!(config.name, "kusama");
            assert_eq!(config.endpoints, expected_endpoints);
            assert_eq!(config.assets, expected_assets);
        }
    }

    #[test]
    #[should_panic(
        expected = "Failed to parse config file: somewhere-nowhere/chain.json. Error: missing configuration field \"name\""
    )]
    #[serial]
    fn test_panic_on_unexisting_config() {
        let _config = chain_config_with_prefix("somewhere-nowhere", "");
    }

    #[test]
    #[serial]
    fn test_web_server_config_with_prefix() {
        // load config from default config dir without any overrides
        {
            let config = web_server_config_with_prefix("", "");
            assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert_eq!(config.port, 16726);
        }

        // override config dir to unexisting one but as long as all config fields are optional it should work
        {
            let config = web_server_config_with_prefix("somewhere-nowhere", "");
            assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert_eq!(config.port, 16726);
        }

        // override some parameter with env var
        {
            unsafe {
                std::env::set_var("WEB_SERVER_PORT", "12345");
            }

            let config = web_server_config_with_prefix("", "");
            assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert_eq!(config.port, 12345);
        }
    }

    #[test]
    #[serial]
    fn test_database_config_with_prefix() {
        // load config from default config dir without any overrides
        {
            let config = database_config_with_prefix("", "");
            assert_eq!(config.path, "kalatori.db".to_string());
            assert!(!config.temporary);
        }

        // override configs dir to unexisting one but as long as all config fields are optional it should work
        {
            let config = database_config_with_prefix("somewhere-nowhere", "");
            assert_eq!(config.path, "kalatori.db".to_string());
            assert!(!config.temporary);
        }

        // override some parameter with env var
        {
            unsafe {
                std::env::set_var("DATABASE_TEMPORARY", "true");
            }

            let config = database_config_with_prefix("", "");
            assert_eq!(config.path, "kalatori.db".to_string());
            assert!(config.temporary);
        }

        // override some parameter with env var with customized prefix
        {
            unsafe {
                std::env::set_var("MEGA_KALATORI_DATABASE_PATH", "mega_kalatori.db");
            }

            let config = database_config_with_prefix("", "MEGA_KALATORI");
            assert_eq!(config.path, "mega_kalatori.db");
            assert!(!config.temporary);
        }
    }
}
