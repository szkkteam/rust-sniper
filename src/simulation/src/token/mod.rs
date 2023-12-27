use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};
use serde::{Deserialize, Serialize};
use crate::{
    dex::{
        Pool, Dex
    }
};

use ethers::prelude::*;



#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub address: Address,
    pub pool: Option<Pool>,
}

impl Token {

    pub fn new(
        address: Address,
        pool: Option<Pool>,
    ) -> Self {
        Self {
            address,
            pool,
        }
        
    }

    pub async fn create(
        address: Address,
        dexes: &Vec<Dex>,
        client: Arc<Provider<Ws>>
    ) -> Token {
        for dex in dexes {
            match dex.new_from_token_weth(address, client.clone()).await {
                Some(pool) => {
                    return Token::new(address, Some(pool));
                }
                None => {
                    
                }
            };
        }
        return Token::new(address, None);
    }

    pub fn get_paired_with(&self) -> Option<Address> {
        match &self.pool {
            Some(pair) => {
                Some(if pair.token_0 == self.address {
                    pair.token_1.clone()
                } else {
                    pair.token_0.clone()
                })
            },
            None => { None }
        }
    }

}


impl Hash for Token {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}
