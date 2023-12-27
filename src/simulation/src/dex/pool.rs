use std::{
    hash::{Hash, Hasher},
    str::FromStr,
};
use serde::{Deserialize, Serialize};
use crate::utils;
use ethers::prelude::*;

pub trait PoolWethPair {
    fn is_weth_pair(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pool {
    pub address: Address,
    pub token_0: Address,
    pub token_1: Address,
    pub pool_variant: PoolVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PoolVariant {
    UniswapV2,
    //UniswapV3
}

impl Pool {
    pub fn new(
        address: Address,
        token_a: Address,
        token_b: Address,
        pool_variant: PoolVariant
    ) -> Pool {
        let (token_0, token_1 ) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        Pool {
            address,
            token_0,
            token_1,
            pool_variant
        }
    }
}

impl PoolWethPair for Pool {
    fn is_weth_pair(&self) -> bool {
        [self.token_0, self.token_1].contains(&utils::constants::get_weth_address())
    }
}

impl Hash for Pool {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

impl PoolVariant {
    pub fn pool_created_event_signature(&self) -> H256 {
        match self {
            PoolVariant::UniswapV2 => {
                H256::from_str("0x0d3648bd0f6ba80134a33ba9275ac585d9d315f0ad8355cddefde31afa28d0e9").unwrap()
            }
            /*
            PoolVariant::UniswapV3 => {
                H256::from_str("0x783cca1c0412dd0d695e784568c96da2e9c22ff989357a2e8b1d9b2b4e6b7118").unwrap()
            }
             */
        }
    } 
}