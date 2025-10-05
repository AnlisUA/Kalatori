mod databases_configs;
#[cfg(feature = "exchanges")]
mod exchanges_configs;
mod queues_configs;
mod rest_api_configs;

pub use databases_configs::*;
#[cfg(feature = "exchanges")]
pub use exchanges_configs::*;
pub use queues_configs::*;
pub use rest_api_configs::*;

use std::borrow::Cow;

use config::Config;
use log::{Level, LevelFilter};
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::Logger;
use sentry::integrations::log::LogFilter;
use sentry::ClientInitGuard;
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub use log4rs;

pub type ConfigResult<T> = Result<T, config::ConfigError>;

#[derive(Debug, Deserialize, Clone, Eq, PartialEq)]
pub struct S3BucketConfig {
    pub access_key: String,
    pub secret_key: String,
    pub bucket_name: String,
    pub region: String,
    pub endpoint_url: Option<String>
}

#[derive(Debug, Deserialize, Clone, Eq, PartialEq)]
pub struct SentryConfig {
    pub dsn: String,
    pub environment: String,
}

#[derive(Debug, Deserialize, Clone, Eq, PartialEq)]
pub struct EncryptionConfig {
    pub encryption_key: String,
}

pub fn format_prefix(prefix: &str, config_prefix: &str) -> String {
    if prefix.is_empty() {
        config_prefix.to_string()
    } else {
        format!("{}_{}", prefix, config_prefix)
    }
}

pub fn config_from_file<T: DeserializeOwned>(filename: &str) -> T {
    let config = Config::builder()
        .add_source(config::File::with_name(filename))
        .build()
        .unwrap_or_else(|err| panic!("Failed to read config file: {}. Error: {}", filename, err));

    config.try_deserialize().unwrap_or_else(|err| panic!("Failed to parse config file: {}. Error: {}", filename, err))
}

pub fn config_from_file_result<T: DeserializeOwned>(filename: &str) -> ConfigResult<T> {
    let config = Config::builder()
        .add_source(config::File::with_name(filename))
        .build()?;

    config.try_deserialize()
}

pub fn config_from_file_or_env_result<T: DeserializeOwned>(filename: &str, env_prefix: &str) -> ConfigResult<T> {
    let config = Config::builder()
        .add_source(config::File::with_name(filename).required(false))
        .add_source(config::Environment::with_prefix(env_prefix))
        .build()?;

    config.try_deserialize()
}

pub fn config_from_file_or_env<T: DeserializeOwned>(filename: &str, env_prefix: &str) -> T {
    let config = Config::builder()
        .add_source(config::File::with_name(filename).required(false))
        .add_source(config::Environment::with_prefix(env_prefix))
        .build()
        .unwrap_or_else(|err| panic!("Failed to read config file: {}. Error: {}", filename, err));

    config.try_deserialize().unwrap_or_else(|err| panic!("Failed to parse config file: {}. Error: {}", filename, err))
}

/// Initializing s3bucket config from `configs/s3_bucket.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Allows to use additional prefix for environment variables.
/// If provided prefix is empty then only `S3_BUCKET_` prefix will be used otherwise `YOUR_PREFIX_S3_BUCKET_`.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `s3_bucket.json` file
///
/// ```json
#[doc = include_str!("../configs/s3_bucket.json")]
/// ```
pub fn s3_bucket_config_with_prefix(prefix: &str) -> S3BucketConfig {
    let env_prefix = format_prefix(prefix, "S3_BUCKET");
    config_from_file_or_env("configs/s3_bucket.json", &env_prefix)
}

/// Initializing s3bucket config from `configs/s3_bucket.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Use `S3_BUCKET_` prefix for environment variables.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `s3_bucket.json` file
///
/// ```json
#[doc = include_str!("../configs/s3_bucket.json")]
/// ```
pub fn s3_bucket_config() -> S3BucketConfig {
    s3_bucket_config_with_prefix("")
}

/// Initializing sentry config from `configs/sentry.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Allows to use additional prefix for environment variables.
/// If provided prefix is empty then only `SENTRY_` prefix will be used otherwise `YOUR_PREFIX_SENTRY_`.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `sentry.json` file
///
/// ```json
#[doc = include_str!("../configs/sentry.json")]
/// ```
pub fn sentry_config_with_prefix(prefix: &str) -> SentryConfig {
    let env_prefix = format_prefix(prefix, "SENTRY");
    config_from_file_or_env("configs/sentry.json", &env_prefix)
}

/// Initializing sentry config from `configs/sentry.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Use `SENTRY_` prefix for environment variables.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `sentry.json` file
///
/// ```json
#[doc = include_str!("../configs/sentry.json")]
/// ```
pub fn sentry_config() -> SentryConfig {
    sentry_config_with_prefix("")
}

/// Initializing logging config from `configs/logging.json`.
/// Environment variables can be used to override the values of the config.
/// Allows to use additional prefix for environment variables.
/// If provided prefix is empty then only `LOGGING_` prefix will be used otherwise `YOUR_PREFIX_LOGGING_`.
/// If file and/or environment variables are missing then default logger config will be used with `INFO` level.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `logging.json` file
///
/// ```json
#[doc = include_str!("../configs/logging.json")]
/// ```
pub fn logger_config_with_prefix(prefix: &str) -> log4rs::config::Config {
    let env_prefix = format_prefix(prefix, "LOGGING");

    let config = Config::builder()
        .add_source(config::File::with_name("configs/logging.json"))
        .add_source(config::Environment::with_prefix(&env_prefix))
        .build();

    if let Ok(config) = config {
        let config: log4rs::config::RawConfig = config.try_deserialize().unwrap_or_else(|err| panic!("Failed to parse config logging.json: {err:?}"));

        let (appenders, errors) = config.appenders_lossy(&log4rs::config::Deserializers::default());

        if !errors.is_empty() {
            panic!("Logging misconfiguration detected!");
        }

        log4rs::config::Config::builder()
            .appenders(appenders)
            .loggers(config.loggers())
            .build(config.root())
            .expect("Failed to build logger according to the dynamic config")
    } else {
        default_logger_config()
    }
}

/// Initializing logging config from `configs/logging.json`.
/// Environment variables can be used to override the values of the config.
/// Use `LOGGING_` prefix for environment variables.
/// If file and/or environment variables are missing then default logger config will be used with `INFO` level.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `logging.json` file
///
/// ```json
#[doc = include_str!("../configs/logging.json")]
/// ```
pub fn logger_config() -> log4rs::config::Config {
    logger_config_with_prefix("")
}

fn default_logger_config() -> log4rs::config::Config {
    let log_line_pattern = "{d(%Y-%m-%d %H:%M:%S.%f)(utc)} {({l})} {M}:{L} — {m}{n}";
    let stdout = log4rs::append::console::ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(log_line_pattern)))
        .build();

    log4rs::config::Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .build(Root::builder().appender("stdout").build(LevelFilter::Info))
        .unwrap()
}

/// Initialize sentry and loggers from given configs.
/// Sentry will use info or more sever log messages as breadcrumbs.
///
/// # Example of initialization of sentry and logger
/// ```
/// use zent_config::initialize_sentry_and_logger;
///
/// let _guard = initialize_sentry_and_logger(
///     zent_config::sentry_config(),
///     zent_config::logger_config()
/// );
/// ```
pub fn initialize_sentry_and_logger(sentry_config: SentryConfig, logger_config: log4rs::config::Config) -> ClientInitGuard {
    let logger = Logger::new(logger_config);
    log::set_max_level(logger.max_log_level());

    let sentry_logger =
        sentry::integrations::log::SentryLogger::with_dest(logger).filter(|md| match md.level() {
            Level::Debug | Level::Trace => LogFilter::Ignore,
            Level::Info | Level::Warn | Level::Error => LogFilter::Breadcrumb,
        });

    log::set_boxed_logger(Box::new(sentry_logger)).unwrap();
    initialize_sentry(sentry_config)
}

fn initialize_sentry(config: SentryConfig) -> ClientInitGuard {
    sentry::init((
        config.dsn.clone(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            max_breadcrumbs: 50,
            attach_stacktrace: true,
            environment: Some(Cow::from(config.environment.clone())),
            ..Default::default()
        },
    ))
}

/// Initializing encryption config from `configs/encryption.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Allows to use additional prefix for environment variables.
/// If provided prefix is empty then only `ENCRYPTION_` prefix will be used otherwise `YOUR_PREFIX_ENCRYPTION_`.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `encryption.json` file
///
/// ```json
#[doc = include_str!("../configs/encryption.json")]
/// ```
pub fn encryption_config_with_prefix(prefix: &str) -> EncryptionConfig {
    let env_prefix = format_prefix(prefix, "ENCRYPTION");
    config_from_file_or_env("configs/encryption.json", &env_prefix)
}

/// Initializing encryption config from `configs/encryption.json`.
/// Environment variables can be used to override the values of the config.
/// Allows file to be missing if environment variables override all required config values, otherwise panics.
/// Use `ENCRYPTION_` prefix for environment variables.
///
/// Folder `configs` must be located in the current working directory.
///
/// ## Example of `encryption.json` file
///
/// ```json
#[doc = include_str!("../configs/encryption.json")]
/// ```
pub fn encryption_config() -> EncryptionConfig {
    encryption_config_with_prefix("")
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    #[test]
    #[serial]
    fn test_initialize_sentry_config() {
        let config = sentry_config();

        assert_eq!(config.dsn, "");
        assert_eq!(config.environment, "dev");

        let cwd = std::env::current_dir().unwrap();
        // jump to configs directory so that we won't be able to read the `configs/sentry.json` file
        // instead we will read data from the env - the function should not panic at all
        let _ = std::env::set_current_dir("configs");
        std::env::set_var("SENTRY_DSN", "some_dsn");
        std::env::set_var("SENTRY_ENVIRONMENT", "some_env");
        let config = sentry_config_with_prefix("");

        assert_eq!(config.dsn, "some_dsn");
        assert_eq!(config.environment, "some_env");

        // restore env state
        std::env::remove_var("SENTRY_DSN");
        std::env::remove_var("SENTRY_ENVIRONMENT");
        let _ = std::env::set_current_dir(cwd);
    }

    #[test]
    #[serial]
    fn test_initialize_encryption_config() {
        let config = encryption_config();

        assert_eq!(config.encryption_key, "test_key".to_string());

        // test that custom prefix works
        std::env::set_var("MY_PREFIX_ENCRYPTION_ENCRYPTION_KEY", "another_key");
        let config = encryption_config_with_prefix("MY_PREFIX");
        assert_eq!(config.encryption_key, "another_key".to_string());

        // restore the state
        std::env::remove_var("MY_PREFIX_ENCRYPTION_ENCRYPTION_KEY");
    }

    #[test]
    #[serial]
    fn test_initialize_s3_bucket_config() {
        let config = s3_bucket_config();

        assert_eq!(config.access_key, "test-key");
        assert_eq!(config.secret_key, "secret-test-key");
        assert_eq!(config.bucket_name, "test");
        assert_eq!(config.region, "test");
        assert_eq!(config.endpoint_url, Some("http://192.168.207.3:9000".to_string()));

        // test that value has been overriden with env
        std::env::set_var("S3_BUCKET_REGION", "new-region");
        let config = s3_bucket_config();

        assert_eq!(config.region, "new-region");
        assert_eq!(config.access_key, "test-key");
        assert_eq!(config.secret_key, "secret-test-key");
        assert_eq!(config.bucket_name, "test");
        assert_eq!(config.endpoint_url, Some("http://192.168.207.3:9000".to_string()));

        // restore the state
        std::env::remove_var("S3_BUCKET_REGION");

        // test that custom prefix works
        std::env::set_var("MY_PREFIX_S3_BUCKET_BUCKET_NAME", "new-name");
        let config = s3_bucket_config_with_prefix("MY_PREFIX");

        assert_eq!(config.bucket_name, "new-name");
        assert_eq!(config.region, "test");
        assert_eq!(config.access_key, "test-key");
        assert_eq!(config.secret_key, "secret-test-key");
        assert_eq!(config.endpoint_url, Some("http://192.168.207.3:9000".to_string()));

        // restore the state
        std::env::remove_var("MY_PREFIX_S3_BUCKET_BUCKET_NAME");
    }
}
