//! Core domain definitions (non-API-specific)

use std::ops::{
    Deref,
    Sub,
};

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Balance(pub u128);

impl Deref for Balance {
    type Target = u128;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Sub for Balance {
    type Output = Self;

    fn sub(
        self,
        r: Self,
    ) -> Self {
        // TODO: it's better to replace u128 with Decimal
        #[expect(clippy::arithmetic_side_effects)]
        Balance(self.0 - r.0)
    }
}

impl Balance {
    pub fn parse(
        float: f64,
        decimals: crate::legacy_types::Decimals,
    ) -> Self {
        let parsed_float = (float * decimal_exponent_product(decimals)).round();

        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Self(parsed_float as _)
    }
}

pub fn decimal_exponent_product(decimals: crate::legacy_types::Decimals) -> f64 {
    10f64.powi(decimals.into())
}
