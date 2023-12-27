use serde::{Deserialize, Serialize};
use ethers::prelude::{
    Address,
    Transaction,
    U256,
    I256,
    Provider,
    Ws,
    Middleware,
    H256
};
use crate::{
    executor::{
        TransactionEvent
    },
    types::{
        PositionId,
        TraderId,
        deserialize_position_id,
        serialize_position_id
    },
    utils::{
        create_websocket_client,
        calcualte_transaction_cost,
        state_diff,
    },
    portfolio::builder::{get_wallets_balances, get_wallets},
};
use std::sync::Arc;
use async_trait::async_trait;
use super::error::PortfolioError;

#[async_trait]
pub trait PositionEnterer {
    async fn enter(trader_id: &TraderId, contract: Address, transaction: &TransactionEvent) -> Result<Position, PortfolioError>;
}

pub trait PositionUpdater {

}

#[async_trait]
pub trait PositionExiter {

}


#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position 
{
    #[serde(deserialize_with = "deserialize_position_id", serialize_with = "serialize_position_id")]
    pub position_id: PositionId,
    pub investment: U256,
    pub fee: U256,
    pub balances: Vec<(Address, U256)>,

    pub realized_pnl: U256,
}

impl Position {

    pub fn is_closed(&self) -> bool {
        self.balances.iter().all(|(_, balance)| balance < &U256::from(1))
    }

    pub fn total_investment_cost(&self) -> U256 {
        self.investment + self.fee
    }

    // This is a sniper, only 1 buy than scaling out, so every other tx are just sells
    pub async fn update_from_transaction(
        mut self,
        contract: Address,
        transaction: &TransactionEvent
    ) -> Position {
        #[cfg(not(feature = "dry"))] 
        {
            let client = create_websocket_client().await.unwrap();

            let balances = get_wallets_balances(
                client.clone(),
                transaction.order.token,
                contract
            ).await;
    
            let wallets = get_wallets(
                client.clone(),
                contract
            ).await;
    
            let balances = wallets.into_iter().zip(balances.into_iter()).collect::<Vec<_>>();
    
            let (gross_realized_profit, fee) = calculate_cost(&client, &transaction.hashes).await;
            let net_realized_profit = U256::try_from(gross_realized_profit).unwrap_or_default().checked_sub(fee).unwrap_or_default();
    
            //self.unrealized_pnl -= I256::try_from(net_realized_profit).unwrap_or_default();
            self.realized_pnl += net_realized_profit;
            self.balances = balances;
        }
        self
    }
}

#[async_trait]
impl PositionEnterer for Position
{
    async fn enter(
        trader_id: &TraderId,
        contract: Address,
        transaction: &TransactionEvent
    ) -> Result<Position, PortfolioError> {

        #[cfg(feature = "dry")] 
        {
            Ok(Position {
                position_id: PositionId::from(trader_id),     
                investment: U256::zero(),
                fee: U256::zero(),
                balances: vec![],
                //unrealized_pnl: investment.checked_sub(I256::try_from(fee).unwrap_or_default()).unwrap(),
                realized_pnl: U256::zero()
            })
        }
        #[cfg(not(feature = "dry"))] 
        {
            let client = create_websocket_client().await.unwrap();
            let (investment, fee) = calculate_cost(&client, &transaction.hashes).await;
    
            let balances = get_wallets_balances(
                client.clone(),
                transaction.order.token,
                contract
            ).await;
    
            let wallets = get_wallets(
                client.clone(),
                contract
            ).await;
    
            let balances = wallets.into_iter().zip(balances.into_iter()).collect::<Vec<_>>();
            // TODO: Calculate costs, fee, gained amount, etc etc
            Ok(Position {
                position_id: PositionId::from(trader_id),     
                investment: U256::try_from(investment.abs()).unwrap_or_default(),
                fee,
                balances,
                //unrealized_pnl: investment.checked_sub(I256::try_from(fee).unwrap_or_default()).unwrap(),
                realized_pnl: U256::zero()
            })
        }
      
    }
}

async fn calculate_weth_difference(
    client: &Arc<Provider<Ws>>,
    tx: &Transaction,
) -> Option<I256> {
    let state_diff = state_diff::get_from_hash(&client, tx.hash.clone()).await?;
    let (from, to) = state_diff::get_weth_balance_change(tx.to.unwrap(), &state_diff)?;
    Some(I256::try_from(to).unwrap() - I256::try_from(from).unwrap())
    
}

async fn calculate_cost(
    client: &Arc<Provider<Ws>>,
    hashes: &Vec<H256>,
) -> (I256, U256) {
    let mut balance_change = I256::zero();
    let mut transaction_cost = U256::zero();
    for hash in hashes {
        let tx = client.get_transaction(hash.clone()).await.unwrap().unwrap();

        match calculate_weth_difference(&client, &tx).await {
            Some(cost) => {
                balance_change += cost;
            },
            None => {}
        };
        let (gas_price, gas) = calcualte_transaction_cost(&tx);
        // GasPrice will return the gas limit, but the actual gas usage is always 70% of the gas limit
        let gas_price= (U256::from(gas_price) * 7) / 10;
        transaction_cost += gas_price * gas;
    }
    (balance_change, transaction_cost)
}