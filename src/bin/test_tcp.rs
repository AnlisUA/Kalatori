use std::net::TcpStream;
use std::time::Duration;

fn main() {
    println!("Testing TCP connections to Chopsticks containers...");
    
    let endpoints = vec![
        ("chopsticks-polkadot", 8000),
        ("chopsticks-statemint", 9000),
        ("172.18.0.3", 8000),
        ("172.18.0.2", 9000),
    ];
    
    for (host, port) in endpoints {
        println!("\nTesting TCP connection to {}:{}", host, port);
        
        match TcpStream::connect_timeout(
            &format!("{}:{}", host, port).parse().unwrap(),
            Duration::from_secs(5)
        ) {
            Ok(_) => {
                println!("✓ TCP connection successful to {}:{}", host, port);
            }
            Err(e) => {
                println!("✗ TCP connection failed to {}:{}: {}", host, port, e);
            }
        }
    }
}