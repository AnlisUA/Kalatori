use alloy::primitives::{address, Address};

pub const CHAIN_ID: u64 = 137; // Polygon Mainnet
pub const BUNDLER_RPC: &str = "https://public.pimlico.io/v2/137/rpc"; // Note: CHAIN ID included in URL

pub const ENTRYPOINT: Address = address!("0x4337084D9E255Ff0702461CF8895CE9E3b5Ff108");
pub const PAYMASTER: Address = address!("0x0578cFB241215b77442a541325d6A4E6dFE700Ec");
pub const USDC: Address = address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359");
pub const ACCOUNT_IMPL: Address = address!("0xe6Cae83BdE06E4c305530e199D7217f42808555B");
