use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
pub enum ChainType {
    PolkadotAssetHub,
}

impl ChainType {
    pub fn iter() -> impl Iterator<Item = ChainType> {
        [
            ChainType::PolkadotAssetHub,
        ]
            .iter()
            .copied()
    }
}

impl std::fmt::Display for ChainType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChainType::PolkadotAssetHub => "PolkadotAssetHub",
        };

        write!(f, "{}", s)
    }
}

impl std::str::FromStr for ChainType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PolkadotAssetHub" => Ok(ChainType::PolkadotAssetHub),
            _ => Err(format!("Unknown ChainType: {}", s)),
        }
    }
}
