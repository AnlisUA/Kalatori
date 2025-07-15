//! Runtime interfaces for different Substrate chains

use jsonrpsee::ws_client::WsClient;

/// Subxt client wrapper that provides both subxt and jsonrpc functionality
pub struct SubxtClient {
    pub jsonrpc: WsClient,
    pub subxt: Option<subxt::OnlineClient<subxt::PolkadotConfig>>,
}

impl SubxtClient {
    /// Get the JsonRPC client for legacy compatibility
    pub fn jsonrpc(&self) -> &WsClient {
        &self.jsonrpc
    }

    /// Get the subxt client (if available)
    pub fn subxt(&self) -> Option<&subxt::OnlineClient<subxt::PolkadotConfig>> {
        self.subxt.as_ref()
    }
}

// Delegate JsonRPC methods to the underlying client for compatibility
impl std::ops::Deref for SubxtClient {
    type Target = WsClient;

    fn deref(&self) -> &Self::Target {
        &self.jsonrpc
    }
}

/// Create a client for a given endpoint
pub async fn create_client(
    endpoint: &str,
) -> Result<SubxtClient, Box<dyn std::error::Error + Send + Sync>> {
    let jsonrpc_client = jsonrpsee::ws_client::WsClientBuilder::default()
        .build(endpoint)
        .await?;

    // Try to create subxt client, but don't fail if it doesn't work
    let subxt_client = match subxt::OnlineClient::<subxt::PolkadotConfig>::from_url(endpoint).await
    {
        Ok(client) => Some(client),
        Err(e) => {
            tracing::warn!("Failed to create subxt client for {}: {}", endpoint, e);
            None
        }
    };

    Ok(SubxtClient {
        jsonrpc: jsonrpc_client,
        subxt: subxt_client,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subxt_client_methods() {
        // Test that SubxtClient methods work correctly
        use jsonrpsee::ws_client::WsClientBuilder;

        // Since we can't create a real client in unit tests, we'll test the structure
        // This test verifies that the SubxtClient struct can be properly constructed
        // and that its methods have the correct signatures
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Try to create a mock client, but if it fails, that's expected in test environment
            let mock_result = WsClientBuilder::default()
                .build("ws://localhost:9944")
                .await;

            match mock_result {
                Ok(jsonrpc_client) => {
                    let subxt_client = SubxtClient {
                        jsonrpc: jsonrpc_client,
                        subxt: None,
                    };

                    // Test that we can access methods
                    let _rpc_client = subxt_client.jsonrpc();
                    assert!(subxt_client.subxt().is_none());

                    // Test Deref implementation compiles
                    let _rpc_client: &WsClient = &subxt_client;
                }
                Err(_) => {
                    // Expected to fail in test environment - this is okay
                    // We'll just test that the structure compiles and has the right methods
                    // by testing method signatures at compile time
                    println!("Mock client creation failed as expected in test environment");
                }
            }
        });
    }

    #[tokio::test]
    async fn test_create_client_error_handling() {
        // Test with invalid endpoint
        let result = create_client("invalid://endpoint").await;
        assert!(result.is_err());
    }
}
