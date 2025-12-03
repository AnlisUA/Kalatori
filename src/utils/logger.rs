use tracing_subscriber::EnvFilter;

use crate::error::Error;

pub fn initialize(directives: impl AsRef<str>) -> Result<(), Error> {
    let _filter = EnvFilter::try_new(directives)?;

    tracing_subscriber::fmt()
        // .with_timer(UtcTime::rfc_3339())
        // .with_env_filter(_filter)
        .json()
        .init();

    Ok(())
}
