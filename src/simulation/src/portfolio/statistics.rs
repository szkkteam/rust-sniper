use serde::{Deserialize, Serialize};
use ethers::prelude::{
    U256
};
use async_trait::async_trait;
use crate::{
    types::{
        TraderId, deserialize_trader_id, serialize_trader_id
    },    
    simulator::{
        event::{
            SellSimulationEvent,
        }
    },
};
use super::error::PortfolioError;

#[async_trait]
pub trait StatisticsCalculator: Sync + Send {

    async fn get_trader_statistics(&mut self, trader_id: &TraderId, event: &SellSimulationEvent) -> Result<Option<Statistics>, PortfolioError>;
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Statistics
{
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub total_investment: U256,

    pub unralized_pnl: U256,
    pub realized_pnl: U256,
}

impl Statistics {

    pub fn new (
        trader_id: TraderId,
        total_investment: U256,
        unralized_pnl: U256,
        realized_pnl: U256,
    ) -> Self {
        Self {
            trader_id,
            total_investment,
            unralized_pnl,
            realized_pnl
        }
    }
}