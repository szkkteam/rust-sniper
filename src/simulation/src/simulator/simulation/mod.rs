pub mod error;
pub mod fork_db;
pub mod helpers;
pub mod tx_builder;
pub mod inspectors;

pub use error::*;
pub mod token_simulation;
pub mod gas_estimation;
pub mod token_liquidity;
pub mod sell_simulation;

pub use gas_estimation::estimage_gas;

use crate::{
    stream::BlockInfo,
    dex::{Pool},
    utils::{
        state_diff::{
            StateDiff,
            to_cache_db,
            empty_db
        },
    },
    token::Token,
};
use num_bigfloat::BigFloat;
use std::fmt;
use serde::{Deserialize, Serialize};
use std::{sync::Arc};
//use super::simulation::SimulationError;
use ethers::{prelude::*, utils::{parse_ether}};
use fork_db::fork_factory::ForkFactory;
use helpers::{
    attach_braindance_module,
};
use token_simulation::{
    simulate_token_max_buy,
    simulate_token_trade,
    SimulationData
};
use sell_simulation::{
    simulate_rug,
    simulate_profit,
};

#[derive(Debug, Clone)]
pub struct SimulatorInput {
    pub input_amount: U256,
    pub pool: Pool,
    pub startend_token: Address,
    pub intermediary_token: Address,
    pub caller_txs: Vec<Transaction>,
}

impl SimulatorInput {

    pub fn new(
        token_address: Address,
        pool: Pool,
        caller_txs: Vec<Transaction>,
    ) -> Self {        
        
        let startend_token = if pool.token_0 == token_address {
            pool.token_1
        } else {
            pool.token_0
        };
        
        Self {
            input_amount: parse_ether("0.011").unwrap(), // 0.001 ether, TODO: make this better,
            pool: pool,
            startend_token,
            intermediary_token: token_address,
            caller_txs
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SimulationResult {
    pub block: BlockInfo,
    pub first_valid_block: Option<BlockInfo>,
    pub tx: Option<Transaction>,

    pub buy_fee: BigFloat,
    pub sell_fee: BigFloat,

    pub buy_gas: U256,
    pub sell_gas: U256,

    pub liquidity_ratio: BigFloat,
    pub token_liquidity: U256,
    pub paired_with_liquidity: U256,

    pub max_tx: Option<U256>,

    pub reason: Option<String>,
}

impl SimulationResult {

    pub fn buy_valid(&self) -> bool {
        // TODO: 
        //self.buy_fee < 90 && (self.max_tx.unwrap_or(U256::from(1)) > U256::zero())
        self.buy_fee < BigFloat::from(90) &&
        !self.is_reverted() &&
        !self.sell_fee.is_inf() &&
        self.liquidity_ratio < BigFloat::from(100) &&
        self.liquidity_ratio > BigFloat::parse("1e-6").unwrap()
    }

    pub fn sell_valid(&self) -> bool {
        // TODO: 
        //self.sell_fee <= 99 && (self.max_tx.unwrap_or(U256::from(1)) > U256::zero())
        self.sell_fee <= BigFloat::from(99) && !self.is_reverted() && !self.sell_fee.is_inf()
    }

    pub fn is_reverted(&self) -> bool {
        self.reason.is_some()
    }
}


impl PartialEq for SimulationResult {
    fn eq(&self, other: &Self) -> bool {
        self.buy_fee == other.buy_fee &&
        self.sell_fee == other.sell_fee &&
        self.max_tx == other.max_tx
    }
}

impl From<Vec<SimulationData>> for SimulationResult 
where
    SimulationError: fmt::Display
{
    fn from(value: Vec<SimulationData>) -> Self {
        let block = value.first().unwrap().block.clone();
        let buy_fee = value
            .iter()
            .max_by(|a, b| 
                a
                .buy_tax
                .partial_cmp(&b.buy_tax).unwrap()
            ).unwrap()
            .buy_tax;
        let sell_fee = value
            .iter()
            .max_by(|a, b| 
                a
                .sell_tax
                .partial_cmp(&b.sell_tax).unwrap()
            )
            .unwrap()
            .sell_tax;

        let buy_gas = value
            .iter()
            .max_by(|a, b| a.buy_gas.cmp(&b.buy_gas))
            .unwrap()
            .buy_gas;

        let sell_gas = value
            .iter()
            .max_by(|a, b| a.sell_gas.cmp(&b.sell_gas))
            .unwrap()
            .sell_gas;

        let first_valid = value.iter().find(|sim| !sim.is_failed());        
        let first_valid_block = match first_valid {
            Some(b) => Some(b.block.clone()),
            None => None,
        };
        let reason = match first_valid {
            Some(b) => b.sim_result.clone(),
            None => value.first().unwrap().sim_result.clone()
        };

        let reason: Option<String> = match reason {
            Some(v) => Some(v.to_string()),
            None => None,
        };

        Self {
            block,
            first_valid_block,
            tx: None,
            buy_fee,
            buy_gas: buy_gas.into(),
            sell_fee,
            sell_gas: sell_gas.into(),
            liquidity_ratio: BigFloat::from(0),
            token_liquidity: U256::zero(),
            paired_with_liquidity: U256::zero(),
            max_tx: None,
            reason
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SellSimulationResult {
    pub frontrun: SellBalanceChange,
    pub backrun: SellBalanceChange,
}

impl SellSimulationResult {
    fn new(
        frontrun: SellBalanceChange,
        backrun: SellBalanceChange,
    ) -> Self {
        Self {
            frontrun,
            backrun
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SellBalanceChange {
    pub gross_balance_change: U256,
    pub gas_used: u64,
    pub error: Option<String>
}

impl SellBalanceChange {
    pub fn is_sell_failed(&self) -> bool {
        #[cfg(feature = "dry")] 
        {
            self.error.is_some()
        }
        #[cfg(not(feature = "dry"))] 
        {
            self.error.is_some() || self.gross_balance_change == U256::zero()
        }
    }
}

impl From<Result<(u64, U256), SimulationError>> for SellBalanceChange {
    fn from(value: Result<(u64, U256), SimulationError>) -> Self {
        value.map_or_else(|e| {
            let mut data = Self::default();
            data.error = Some(e.to_string());
            data    
        }, |(gas_used, gross_balance_change)| {
            Self {
                gross_balance_change,
                gas_used,
                error: None,
            }
        })
    }
}


pub async fn simulate_token(
    token: &Token,
    txs: &Vec<Transaction>,
    fork_block: &BlockInfo,

    fork_factory: &mut ForkFactory,
) -> Result<SimulationResult, SimulationError> {

    let request = SimulatorInput::new(token.address, token.pool.ok_or(SimulationError::TokenHasNoPool)?,  txs.to_vec());

    // TODO: Use the other values too
    let (liquidity_ratio, token_pair_balance, other_pair_balance) = token_liquidity::liquidity_ratio(
        token,
        &request.caller_txs,
        fork_block,
        fork_factory.new_sandbox_fork()
    )?;

    
    let (max_result, buy_result) = tokio::join!(
        simulate_token_max_buy(
            &request,
            &fork_block,
            fork_factory.new_sandbox_fork())
        , simulate_token_trade(
            &request,
            &fork_block,
            fork_factory.new_sandbox_fork()
        ));

    let max_result = max_result?;
    let buy_result = buy_result?;
    
    let mut sim_result = SimulationResult::from(buy_result);
    sim_result.tx = txs.last().cloned();
    sim_result.max_tx = max_result;
    sim_result.liquidity_ratio = liquidity_ratio;
    Ok(sim_result)
}




pub async fn simulate_sell(
    token: Token,
    txs: Vec<Transaction>,
    test_txs: Vec<Transaction>,
    target_block: BlockInfo,
    mut fork_factory: ForkFactory,
) -> Result<SellSimulationResult, SimulationError> {

    let contract = test_txs.last().unwrap().to.clone().unwrap();
    // TODO: Can we do this outside somehow?
    #[cfg(feature = "dry")] 
    {   
        use helpers::inject_test_sniper;
        // Only the stubbed version of the contract works here, DO NOT USE THAT IN PROD!
        let owner = test_txs.last().unwrap().from.clone();

        inject_test_sniper(
            owner,
            contract,
            &mut fork_factory,
        );
    }
   
    let (frontrun, backrun) = tokio::join!(
        simulate_profit(
            contract.clone(),
            &token,
            &test_txs,
            &target_block,
            fork_factory.new_sandbox_fork())
        , simulate_rug(
            contract.clone(),
            &txs,
            &test_txs,
            &target_block,
            fork_factory.new_sandbox_fork())
        );
    let frontrun = SellBalanceChange::from(frontrun);
    let backrun = SellBalanceChange::from(backrun);

    Ok(SellSimulationResult::new(frontrun, backrun))
    
}


pub async fn prepare_database(
    client: Arc<Provider<Ws>>,
    fork_block: BlockInfo,
    state_diff: Option<StateDiff>
) -> Result<ForkFactory, SimulationError> {
    
    let fork_block = Some(BlockId::Number(BlockNumber::Number(
        fork_block.number,
    )));
    
    let initial_db = match state_diff {
        Some(state_diff) => { to_cache_db(&state_diff, fork_block, &client).await.unwrap() }
        None => { empty_db() }
    };

    let mut fork_factory = ForkFactory::new_sandbox_factory(client, initial_db, fork_block);


    attach_braindance_module(&mut fork_factory);


    Ok(fork_factory)
}
