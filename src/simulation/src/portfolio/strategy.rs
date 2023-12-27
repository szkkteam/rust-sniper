use ethers::prelude::{
    U256
};
use async_trait::async_trait;
use crate::{
    types::{
        TraderId
    },    
    simulator::{
        event::{
            SellSimulationEvent,
        }
    },
};
use super::{
    OrderEvent,
    error::PortfolioError,
    statistics::{
        Statistics
    }
};


#[async_trait]
pub trait StrategyGenerator: Sync + Send {

    async fn generate_strategy_order(&mut self, trader_id: &TraderId, statistics: &Statistics) -> Result<Option<OrderEvent>, PortfolioError>;
}