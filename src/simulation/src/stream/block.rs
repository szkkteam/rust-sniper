use tokio::sync::watch;
use crate::{
    utils
};
use serde::{Deserialize, Serialize};
use eyre::Result;
use ethers::prelude::*;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockInfo {
    pub number: U64,
    pub timestamp: U256,
    pub base_fee: U256,
}

impl BlockInfo {
    // Create a new `BlockInfo` instance
    pub fn new(number: U64, timestamp: U256, base_fee: U256) -> Self {
        Self {
            number,
            timestamp,
            base_fee,
        }
    }

    // Find the next block ahead of `prev_block`
    pub fn find_next_block_info(prev_block: &Block<TxHash>) -> Self {
        let number = prev_block.number.unwrap_or_default() + 1;
        let timestamp = prev_block.timestamp + 12;
        let base_fee = utils::calculate_next_block_base_fee(prev_block);

        Self {
            number,
            timestamp,
            base_fee,
        }
    }
    
    pub fn roll(&self, step: U256) -> Self {
        let number = self.number + step.as_u64();
        let timestamp = self.timestamp + (step * 12);
        let base_fee = self.base_fee;
        Self {
            number,
            timestamp,
            base_fee
        }
    }

}

impl PartialEq for BlockInfo {
    fn eq(&self, other: &Self) -> bool {
        self.number == other.number
    }
}

impl Eq for BlockInfo {}

#[derive(Debug, Clone, Default)]
pub struct BlockOracle {
    pub latest: BlockInfo,
    pub next: BlockInfo,
    pub block: Block<H256>,
}

impl From<Block<H256>> for BlockOracle {
    fn from(block: Block<H256>) -> Self {
        let latest = BlockInfo::new(
            block.number.unwrap_or_default(),
            block.timestamp,
            block.base_fee_per_gas.unwrap_or_default()
        );
        Self {
            latest,
            next: BlockInfo::find_next_block_info(&block),
            block,
        }
    }
}

pub async fn stream_block_notification() -> Result<watch::Receiver<BlockOracle>, ProviderError> {
    let client = utils::create_websocket_client().await.unwrap();
    let latest_block = match client.get_block(BlockNumber::Latest).await {
        Ok(b) => b,
        Err(e) => return Err(e),
    };
    
    //println!("Latest Block txs: {:?}\n", latest_block.clone().unwrap().transactions);
    
    let lb = if let Some(b) = latest_block {
        BlockOracle::from(b)
    } else {
        return Err(ProviderError::CustomError("Block not found".to_string()));
    };
    log::info!("{}", format!("Starting from latest block: {:?}", lb.latest.number));
    let (tx, rx )= watch::channel(lb);

    tokio::spawn(async move {
        let mut block_stream = if let Ok(stream) = client.subscribe_blocks().await {
            stream
        } else {
            panic!("Failed to create new block stream");
        };

        while let Some(block) = block_stream.next().await {
            // lock the RwLock for write access and update the variable
            {   
                
                let b = match client.get_block(BlockNumber::Number(block.number.unwrap())).await {
                    Ok(v) => {
                        match v {
                            Some(b) => b,
                            None => {continue;}                        
                        }
                    },
                    Err(_) => { continue;}
                };
                //println!("Block txs: {:?}\n", b.transactions);
                match tx.send(BlockOracle::from(b)) {
                    Ok(_) => {continue;},
                    Err(_) => {continue;}
                };
            }
        }
    });

    Ok(rx)
}