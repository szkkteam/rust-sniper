use std::{sync::Arc, str::FromStr};

use ethers::{prelude::*};
use crate::abi::*;
use crate::utils;
pub mod pool;
pub use pool::*;

#[derive(Clone, Copy)]
pub struct Dex {
    pub address: Address,
    pub pool_variant: pool::PoolVariant,
}


impl Dex {

    pub fn new(address: H160, pool_variant: pool::PoolVariant) -> Dex {
        Dex {
            address,
            pool_variant
        }
    }

    pub fn new_pool_from_event(&self, log: Log, provider: Arc<Provider<Ws>>) -> Option<pool::Pool> {
        match self.pool_variant {
            pool::PoolVariant::UniswapV2 => {
                let uniswap_v2_factory = UniswapV2Factory::new(self.address, provider);

                let (token_0, token_1, address, _)  = if let Ok(pair) = uniswap_v2_factory.decode_event::<(Address, Address, Address, U256)>(
                    "PairCreated",
                    log.topics,
                    log.data
                ) {
                    pair 
                } else {
                    return None;
                };

                // TODO: Ignore pools, which does not have WETH as one of their pair
                if ![token_0, token_1].contains(&utils::constants::get_weth_address()) {
                    return None;
                }

                Some(pool::Pool::new(
                    address,
                    token_0,
                    token_1,
                    pool::PoolVariant::UniswapV2
                ))
            }
           
        }
    }

    pub async fn new_from_token_weth(&self, token_address: Address, provider: Arc<Provider<Ws>>) -> Option<pool::Pool> {
        match self.pool_variant {
            pool::PoolVariant::UniswapV2 => {
                let uniswap_v2_factory = UniswapV2Factory::new(self.address, provider.clone());
                let pair_address = if let Ok(pair) = uniswap_v2_factory.get_pair(token_address, utils::constants::get_weth_address()).call().await {
                    pair
                } else {
                    return None;
                };
                println!("pair: {:?} token: {:?} factory: {:?} weth: {:?}", pair_address, token_address, self.address, utils::constants::get_weth_address());
                log::info!( "{}", format!("Pair {:?} found for token {:?}", pair_address, token_address));
                // In case of pair does not exists
                if pair_address == Address::from_str("0x0000000000000000000000000000000000000000").unwrap() {
                    return None
                }
                let uniswap_v2_pair = UniswapV2Pair::new(pair_address, provider);

                let token_0 = uniswap_v2_pair.token_0().call().await.unwrap();
                let token_1 = uniswap_v2_pair.token_1().call().await.unwrap();

                Some(pool::Pool::new(
                    pair_address,
                    token_0,
                    token_1,
                    pool::PoolVariant::UniswapV2
                ))
            }
            
        }
    }

}