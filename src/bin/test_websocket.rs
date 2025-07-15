use jsonrpsee::ws_client::WsClientBuilder;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
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
        
        match WsClientBuilder::default().build(endpoint).await {
            Ok(client) => {
                println!("✓ Successfully connected to {}", endpoint);
                
                // Test a simple RPC call
                match client.request::<Value, _>("system_name", rpc_params![]).await {
                    Ok(response) => {
                        println!("✓ RPC call successful: {:?}", response);
                    }
                    Err(e) => {
                        println!("✗ RPC call failed: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("✗ Failed to connect to {}: {}", endpoint, e);
            }
        }
    }
    
    Ok(())
}