use subxt::OnlineClient;
use subxt::PolkadotConfig;
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing WebSocket connection to Chopsticks containers...");
    
    let endpoints = vec![
        "ws://localhost:8000",
        "ws://localhost:9000",
    ];
    
    for endpoint in endpoints {
        println!("\nTesting connection to: {}", endpoint);
        
        match OnlineClient::<PolkadotConfig>::from_url(endpoint).await {
            Ok(client) => {
                println!("✓ Successfully connected to {}", endpoint);
                let name = client.backend().legacy_rpc_methods().system_chain().await;
                match name {
                    Ok(n) => println!("✓ RPC call successful: {:?}", n.0),
                    Err(e) => println!("✗ RPC call failed: {}", e),
                }
            }
            Err(e) => {
                println!("✗ Failed to connect to {}: {}", endpoint, e);
            }
        }
    }
    
    Ok(())
}