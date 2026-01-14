use std::net::{IpAddr, Ipv4Addr};

use kalatori_client::types::ChainType;

pub const DEFAULT_CONFIG_DIR_PATH: &str = "configs";

pub const DEFAULT_POLKADOT_ASSET_HUB_ENDPOINTS: &[&str] = &[
    "wss://asset-hub-polkadot-rpc.n.dwellir.com",
    "wss://polkadot-asset-hub-rpc.polkadot.io",
];

pub const DEFAULT_INVOICE_LIFETIME_MILLIS: u64 = 86_400_000; // 24 hours

pub const DEFAULT_ALLOW_INSECURE_ENDPOINTS: bool = false;

pub const DEFAULT_CHAIN: ChainType = ChainType::PolkadotAssetHub;

pub const DEFAULT_ASSET_HUB_ASSET_ID: &str = "1337";

pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

pub const DEFAULT_PORT: u16 = 8080;

pub const DEFAULT_DATABASE_DIR: &str = "./database";
