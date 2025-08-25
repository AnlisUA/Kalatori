use codec::Decode;
use subxt::OnlineClient;
use subxt::PolkadotConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Test both production and Chopsticks
    let endpoints = vec![
        ("Production", "wss://rpc.polkadot.io"),
        ("Chopsticks", "ws://localhost:8000"),
    ];
    
    for (name, endpoint) in endpoints {
        println!("\n=== Testing {} ({}) ===", name, endpoint);
        
        let client = match OnlineClient::<PolkadotConfig>::from_url(endpoint).await {
            Ok(client) => client,
            Err(e) => {
                println!("Failed to connect to {}: {}", endpoint, e);
                continue;
            }
        };
        let raw = client.metadata().into_inner();
        println!("Raw metadata bytes: {}", raw.len());
        match frame_metadata::RuntimeMetadata::decode(&mut &raw[..]) {
            Ok(frame_metadata::RuntimeMetadata::V15(runtime_metadata_v15)) => {
                println!("✓ Successfully decoded metadata v15");
                println!("Number of pallets: {}", runtime_metadata_v15.pallets.len());
            }
            Ok(other) => {
                println!("✗ Got different metadata version: {:?}", other);
            }
            Err(e) => {
                println!("✗ Failed to decode metadata: {:?}", e);
            }
        }
    }
    
    Ok(())
}