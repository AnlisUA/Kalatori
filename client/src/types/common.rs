use serde::{
    Deserialize,
    Serialize,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
pub enum ChainType {
    PolkadotAssetHub,
    Polygon,
}

impl ChainType {
    pub fn iter() -> impl Iterator<Item = ChainType> {
        [ChainType::PolkadotAssetHub, ChainType::Polygon]
            .iter()
            .copied()
    }
}

impl std::fmt::Display for ChainType {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        let s = match self {
            ChainType::PolkadotAssetHub => "PolkadotAssetHub",
            ChainType::Polygon => "Polygon",
        };

        write!(f, "{s}")
    }
}

impl std::str::FromStr for ChainType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PolkadotAssetHub" => Ok(ChainType::PolkadotAssetHub),
            "Polygon" => Ok(ChainType::Polygon),
            _ => Err(format!("Unknown ChainType: {s}")),
        }
    }
}
