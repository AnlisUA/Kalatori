use codec::Decode;
use jsonrpsee::ws_client::WsClientBuilder;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Test both production and Chopsticks
    let endpoints = vec![
        ("Production", "wss://rpc.polkadot.io"),
        ("Chopsticks", "ws://localhost:8000"),
    ];
    
    for (name, endpoint) in endpoints {
        println!("\n=== Testing {} ({}) ===", name, endpoint);
        
        let client = match WsClientBuilder::default().build(endpoint).await {
            Ok(client) => client,
            Err(e) => {
                println!("Failed to connect to {}: {}", endpoint, e);
                continue;
            }
        };
        
        // Get metadata
        let metadata_request: Value = client
            .request(
                "state_call",
                rpc_params![
                    "Metadata_metadata_at_version",
                    "0x0f000000", // 15 in little-endian encoding
                    Option::<String>::None
                ],
            )
            .await?;
        
        match metadata_request {
            Value::String(x) => {
                println!("Raw metadata length: {}", x.len());
                println!("First 100 chars: {}", &x[..100.min(x.len())]);
                
                match hex::decode(x.trim_start_matches("0x")) {
                    Ok(metadata_raw) => {
                        println!("Decoded hex length: {}", metadata_raw.len());
                        println!("First 20 bytes: {:?}", &metadata_raw[..20.min(metadata_raw.len())]);
                        
                        // Try to decode as Option<Vec<u8>>
                        match Option::<Vec<u8>>::decode(&mut &metadata_raw[..]) {
                            Ok(maybe_metadata_raw) => {
                                match maybe_metadata_raw {
                                    Some(meta_v15_bytes) => {
                                        println!("Option decoded, inner length: {}", meta_v15_bytes.len());
                                        println!("First 20 bytes of inner: {:?}", &meta_v15_bytes[..20.min(meta_v15_bytes.len())]);
                                        
                                        // Check if it starts with "meta"
                                        if meta_v15_bytes.starts_with(b"meta") {
                                            println!("✓ Starts with 'meta' prefix");
                                            
                                            // Try to decode the actual metadata
                                            match frame_metadata::RuntimeMetadata::decode(&mut &meta_v15_bytes[4..]) {
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
                                        } else {
                                            println!("✗ Doesn't start with 'meta' prefix");
                                        }
                                    }
                                    None => {
                                        println!("✗ Option decoded to None");
                                    }
                                }
                            }
                            Err(e) => {
                                println!("✗ Failed to decode as Option<Vec<u8>>: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("✗ Failed to decode hex: {:?}", e);
                    }
                }
            }
            _ => {
                println!("✗ Metadata response is not a string: {:?}", metadata_request);
            }
        }
    }
    
    Ok(())
}