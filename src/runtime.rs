//! Runtime interfaces for different Substrate chains (pure Subxt)

/// Subxt client wrapper used across the codebase
pub struct SubxtClient {
    pub subxt: subxt::OnlineClient<subxt::PolkadotConfig>,
}

impl SubxtClient {
    pub fn subxt(&self) -> &subxt::OnlineClient<subxt::PolkadotConfig> {
        &self.subxt
    }
}

/// Create a client for a given endpoint using Subxt
pub async fn create_client(
    endpoint: &str,
) -> Result<SubxtClient, Box<dyn std::error::Error + Send + Sync>> {
    let subxt_client = subxt::OnlineClient::<subxt::PolkadotConfig>::from_url(endpoint).await?;
    Ok(SubxtClient { subxt: subxt_client })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subxt_client_methods() {
        // Ensure type compiles
        let _ = std::mem::size_of::<SubxtClient>();
    }

    #[tokio::test]
    async fn test_create_client_error_handling() {
        // Invalid scheme should error
        let result = create_client("invalid://endpoint").await;
        assert!(result.is_err());
    }
}
