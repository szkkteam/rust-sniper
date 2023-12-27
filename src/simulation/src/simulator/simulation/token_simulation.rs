use ethers::prelude::*;
use num_bigfloat::BigFloat;
use revm::primitives::{ExecutionResult, Output, TransactTo, B160 as rAddress, U256 as rU256};

//use crate::prelude::access_list::AccessListInspector;
use super::fork_db::fork_db::ForkDB;

use crate::{
    stream::BlockInfo,
    dex::{
        PoolVariant
    }
};
use super::{
    SimulationError,
    SimulatorInput,
};
use super::tx_builder;

use super::helpers::{
    braindance_address, braindance_controller_address,
    setup_block_state, get_balance_of_evm
};

#[derive(Debug, Clone, Default)]
pub struct SimulationData {
    pub block: BlockInfo,
    pub buy_tax: BigFloat,
    pub sell_tax: BigFloat,
    pub sim_result: Option<SimulationError>,
    pub buy_gas: u64,
    pub sell_gas: u64,
}

impl SimulationData {
    pub fn new(block: BlockInfo) -> Self {
        Self {
            block,
            buy_tax: BigFloat::from(100),
            sell_tax: BigFloat::from(100),
            sim_result: None,
            buy_gas: 0,
            sell_gas: 0
        }
    }

    pub fn construct_buy_tax(&mut self, buy_real_out_amount: U256, buy_out_amount: U256) {
        let mut tax = BigFloat::from_u128(buy_real_out_amount.as_u128()) / BigFloat::from_u128(buy_out_amount.as_u128()) * BigFloat::from(100);
        if tax.is_nan() || tax.is_inf() {
            tax = BigFloat::from(0);
        }
        self.buy_tax = BigFloat::from(100) - tax;
    }
   
    pub fn construct_sell_tax(&mut self, sell_real_out_amount: U256, sell_out_amount: U256) {
        let mut tax = BigFloat::from_u128(sell_real_out_amount.as_u128()) / BigFloat::from_u128(sell_out_amount.as_u128()) * BigFloat::from(100);
        if tax.is_nan() || tax.is_inf() {
            tax = BigFloat::from(0);
        }
        self.sell_tax = BigFloat::from(100) - tax;
    }
    
    pub fn is_failed(&self) -> bool {
        self.sim_result.is_some() || self.buy_tax > BigFloat::from(90) || self.sell_tax > BigFloat::from(90)
    }
}

pub async fn simulate_token_trade(
    request: &SimulatorInput,
    start_block: &BlockInfo,
    fork_db: ForkDB,
) -> Result<Vec<SimulationData>, SimulationError> {
    // Start simulating +10 blocks in advance
    let mut blocks = vec![];
    for i in 0..10 {
    //for i in 0..1 {
        blocks.push(     
            start_block.roll(i.into())
        );
    };
    let mut buy_result = Vec::new();
    
    //let mut sell_result = Vec::new();
    for block in &blocks {
        let sim = tokio::task::spawn(simulate_token_buy(
            request.clone(),
            start_block.clone(),
            block.clone(),
            fork_db.clone()
        ));
        buy_result.push(sim);
    }
   
    let buy_result = futures::future::join_all(buy_result).await;
   
    let buy_result = buy_result
            .into_iter()
            .map(|r| r.unwrap().unwrap())
            .collect::<Vec<_>>();

    Ok(buy_result)
}

pub async fn simulate_token_max_buy(    
    request: &SimulatorInput,
    start_block: &BlockInfo,
    fork_db: ForkDB,
) -> Result<Option<U256>, SimulationError> {

    let mut evm = revm::EVM::new();
    evm.database(fork_db.clone());
    // First apply the original block
    setup_block_state(&mut evm, &start_block);

    // Apply transactions - in case if addliq is also creating the pair
    apply_transactions(&mut evm, &request.caller_txs);

    let upper_limit = match get_balance_of_evm(request.intermediary_token, request.pool.address, &start_block, &mut evm) {
        Ok(v) => v,
        Err(e) => { 
            log::error!("{}", format!("Max TX failed to fetch pairs token balance: {:?} reason: {:?}", request.intermediary_token, e));
            return Ok(Some(U256::zero())) 
        }
    };
    if upper_limit == U256::zero() {
        return Ok(Some(upper_limit));
    }
    let mut lower_bound: U256 = U256::zero();
    let mut upper_bound: U256 = upper_limit;
    // setup values for search termination
    let base = U256::from(1000000u64);
    let tolerance = U256::from(1u64);
    
    let tolerance = (tolerance * ((upper_bound + lower_bound) / 2)) / base;

    // initialize variables for search
    let left_interval_lower = |i: usize, intervals: &Vec<U256>| intervals[i - 1].clone() + 1;
    let right_interval_upper = |i: usize, intervals: &Vec<U256>| intervals[i + 1].clone() - 1;
    let should_loop_terminate = |lower_bound: U256, upper_bound: U256| -> bool {
        let search_range = match upper_bound.checked_sub(lower_bound) {
            Some(range) => range,
            None => return true,
        };
        // produces negative result
        if lower_bound > upper_bound {
            return true;
        }
        // tolerance condition not met
        if search_range < tolerance {
            return true;
        }
        false
    };
    let mut highest_amount_out = U256::zero();
    let number_of_intervals = 15;
    let mut counter = 0;

    // continue search until termination condition is met (no point seraching down to closest wei)
    loop {
        counter += 1;
        if should_loop_terminate(lower_bound, upper_bound) {
            break;
        }

        // split search range into intervals
        let mut intervals = Vec::new();
        for i in 0..=number_of_intervals {
            intervals.push(lower_bound + (((upper_bound - lower_bound) * i) / number_of_intervals));
        }
        let mut amount_outs = Vec::new();
        for bound in &intervals {
            let sim: tokio::task::JoinHandle<Result<U256, SimulationError>> = tokio::task::spawn(simulate_max_buy(
                *bound,
                request.clone(),
                start_block.clone(),
                fork_db.clone()
            ));
            amount_outs.push(sim);
        }

        let amount_outs = futures::future::join_all(amount_outs).await;

        let amount_outs = amount_outs
            .into_iter()
            .map(|r| r.unwrap().unwrap_or_default())
            .collect::<Vec<_>>();
        // find interval that produces highest revenue
        let (highest_amount_out_index, _highest_revenue) = amount_outs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.cmp(&b))
            .unwrap();

        highest_amount_out = intervals[highest_amount_out_index];

        // enhancement: find better way to increase finding opps incase of all rev=0
        if amount_outs[highest_amount_out_index] == U256::zero() {
            // most likely there is no sandwich possibility
            if counter == 10 {
                return Ok(Some(U256::zero()));
            }
            // no revenue found, most likely small optimal so decrease range
            upper_bound = intervals[intervals.len() / 3];
            continue;
        }

        // if highest revenue is produced at last interval (upper bound stays fixed)
        if highest_amount_out_index == intervals.len() - 1 {
            lower_bound = left_interval_lower(highest_amount_out_index, &intervals);
            continue;
        }

        // if highest revenue is produced at first interval (lower bound stays fixed)
        if highest_amount_out_index == 0 {
            upper_bound = right_interval_upper(highest_amount_out_index, &intervals);
            continue;
        }

        // set bounds to intervals adjacent to highest revenue index and search again
        lower_bound = left_interval_lower(highest_amount_out_index, &intervals);
        upper_bound = right_interval_upper(highest_amount_out_index, &intervals);
    }

    let result = if highest_amount_out.checked_mul(U256::from(100)).unwrap_or(U256::MAX).checked_div(upper_limit).unwrap_or(U256::MAX) >= U256::from(90) {
        None
    } else {
        Some(highest_amount_out)
    };

    Ok(result)
}

fn apply_transactions(evm: &mut revm::EVM<ForkDB>, transactions: &Vec<Transaction>) {
    for tx in transactions.iter() {
        evm.env.tx.caller = rAddress::from_slice(&tx.from.0);
        evm.env.tx.transact_to =
            TransactTo::Call(rAddress::from_slice(&tx.to.unwrap_or_default().0));
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
        evm.env.tx.chain_id = tx.chain_id.map(|id| id.as_u64());
        evm.env.tx.nonce = Some(tx.nonce.as_u64());
        evm.env.tx.gas_limit = tx.gas.as_u64();
        match tx.transaction_type {
            Some(ethers::types::U64([0])) => {
                // legacy tx
                evm.env.tx.gas_price = tx.gas_price.unwrap_or_default().into();
            }
            Some(_) => {
                // type 2 tx
                evm.env.tx.gas_priority_fee = tx.max_priority_fee_per_gas.map(|mpf| mpf.into());
                evm.env.tx.gas_price = tx.max_fee_per_gas.unwrap_or_default().into();
            }
            None => {
                // legacy tx
                evm.env.tx.gas_price = tx.gas_price.unwrap_or_default().into();
            }
        }

        let _res = evm.transact_commit();      
        //println!("Tx result: {:?}", _res)  ;
    }
}

fn apply_braindance_buy_transaction(
    evm: &mut revm::EVM<ForkDB>,
    block: &BlockInfo,
    data: Bytes,
    pool_variant: PoolVariant
) -> Result<(U256, U256, u64), SimulationError> {

    evm.env.tx.caller = braindance_controller_address();
    evm.env.tx.transact_to = TransactTo::Call(braindance_address().0.into());
    evm.env.tx.data = data.0;
    evm.env.tx.gas_limit = 700000;
    evm.env.tx.nonce = None;
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.gas_price = block.base_fee.into();
    evm.env.tx.value = rU256::ZERO;

    //println!("Sim result frontrun: {:?}", evm.env);

    let result = match evm.transact_commit() {
        Ok(result) => result,
        Err(e) => {
            return Err(SimulationError::FrontrunEvmError(e))
        },
    };

    let buy_gas_used = result.gas_used();
    let output = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o,
            Output::Create(o, _) => o,
        },
        ExecutionResult::Revert { output, .. } => {
            return Err(SimulationError::FrontrunReverted(output))
        }
        ExecutionResult::Halt { reason, .. } => {
            return Err(SimulationError::FrontrunHalted(reason))
        }
    };

    let (buy_amount_out, buy_real_amount_out) = match pool_variant {
        PoolVariant::UniswapV2 => {
            match tx_builder::decode_swap_v2_result(output.into()) {
                Ok(output) => output,
                //Err(e) => return Err(SimulationError::FailedToDecodeOutput(e)),
                Err(_) => return Err(SimulationError::ZeroOptimal()),
            }
        }
    };

    Ok((buy_amount_out, buy_real_amount_out, buy_gas_used))
}

fn apply_braindance_sell_transaction(
    evm: &mut revm::EVM<ForkDB>,
    block: &BlockInfo,
    data: Bytes,
    pool_variant: PoolVariant
) -> Result<(U256, U256, u64), SimulationError> {
    evm.env.tx.caller = braindance_controller_address();
    evm.env.tx.transact_to = TransactTo::Call(braindance_address().0.into());
    evm.env.tx.data = data.0;
    evm.env.tx.gas_limit = 700000;
    //evm.env.tx.nonce = Some(1);
    evm.env.tx.nonce = None;
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.gas_price = block.base_fee.into();
    evm.env.tx.value = rU256::ZERO;

    let result = match evm.transact_commit() {
        Ok(result) => result,
        Err(e) => {
            return Err(SimulationError::BackrunEvmError(e))
        },
    };
    let sell_gas_used = result.gas_used();
    let output = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o,
            Output::Create(o, _) => o,
        },
        ExecutionResult::Revert { output, .. } => {
            return Err(SimulationError::BackrunReverted(output))
        }
        ExecutionResult::Halt { reason, .. } => return Err(SimulationError::BackrunHalted(reason)),
    };
    let (sell_amount_out, sell_real_amount_out) = match pool_variant {
        PoolVariant::UniswapV2 => {
            match tx_builder::decode_swap_v2_result(output.into()) {
                Ok(output) => output,
                //Err(e) => return Err(SimulationError::FailedToDecodeOutput(e)),
                Err(_) => return Err(SimulationError::ZeroOptimal()),
            }
        }
        
    };  
    
    //println!("sell_amount_out: {:?} | sell_real_amount_out: {:?} | post_balance: {:?}", sell_amount_out, sell_real_amount_out, post_balance);

    Ok((sell_amount_out, sell_real_amount_out, sell_gas_used))
}

fn apply_braindance_max_buy(
    evm: &mut revm::EVM<ForkDB>,
    block: &BlockInfo,
    data: Bytes,
    pool_variant: PoolVariant
) -> Result<U256, SimulationError> {

    evm.env.tx.caller = braindance_controller_address();
    evm.env.tx.transact_to = TransactTo::Call(braindance_address().0.into());
    evm.env.tx.data = data.0;
    evm.env.tx.gas_limit = 700000;
    evm.env.tx.nonce = None;
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.gas_price = block.base_fee.into();
    evm.env.tx.value = rU256::ZERO;

    //println!("Sim result frontrun: {:?}", evm.env);

    let result = match evm.transact_commit() {
        Ok(result) => result,
        Err(e) => {
            return Err(SimulationError::FrontrunEvmError(e))
        },
    };

    let output = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o,
            Output::Create(o, _) => o,
        },
        ExecutionResult::Revert { output, .. } => {
            return Err(SimulationError::FrontrunReverted(output))
        }
        ExecutionResult::Halt { reason, .. } => {
            return Err(SimulationError::FrontrunHalted(reason))
        }
    };

    let buy_amount_out = match pool_variant {
        PoolVariant::UniswapV2 => {
            match tx_builder::decode_maxbuy_swap_v2_result(output.into()) {
                Ok(output) => output,
                //Err(e) => return Err(SimulationError::FailedToDecodeOutput(e)),
                Err(_) => return Err(SimulationError::ZeroOptimal()),
            }
        }
    };

    Ok(buy_amount_out)
}

async fn simulate_max_buy(
    amout_out: U256,
    data: SimulatorInput,
    target_block: BlockInfo,
    fork_db: ForkDB,
) -> Result<U256, SimulationError> {

    let mut evm = revm::EVM::new();
    evm.database(fork_db);
    // First apply the original block
    setup_block_state(&mut evm, &target_block);

    // Apply transactions
    apply_transactions(&mut evm, &data.caller_txs);

    let buy_data = match data.pool.pool_variant {
        PoolVariant::UniswapV2 => {
            tx_builder::build_maxbuy_swap_v2_data(
                amout_out,
                data.pool.address,
                data.startend_token,
                data.intermediary_token
            )
        }
    };
    
    let buy_amount_out = match apply_braindance_max_buy(&mut evm, &target_block, buy_data, data.pool.pool_variant) {
        Ok(v) => v,
        Err(e) => return Err(e)
    };
    Ok(buy_amount_out)
    // I need the token info
}

async fn simulate_token_buy(
    data: SimulatorInput,
    original_block: BlockInfo,
    target_block: BlockInfo,
    fork_db: ForkDB,
) -> Result<SimulationData, SimulationError> {

    //let mut sim_result = SimulationData::default();
    let mut sim_result = SimulationData::new(target_block.clone());

    let mut evm = revm::EVM::new();

    evm.database(fork_db);

    // First apply the original block
    setup_block_state(&mut evm, &original_block);

    // Apply transactions
    apply_transactions(&mut evm, &data.caller_txs);
    
    // Second apply the target block
    setup_block_state(&mut evm, &target_block);

    /**
     * Prepare the buy side backrun transaction
     */
    //log::info!( "{}", format!("Pair address: {:?}", data.pool.address));
    let buy_data = match data.pool.pool_variant {
        PoolVariant::UniswapV2 => {
            tx_builder::build_swap_v2_data(
                data.input_amount,
                data.pool.address,
                data.startend_token,
                data.intermediary_token
            )
        }
    };
    

    let (buy_amount_out, buy_real_amount_out, buy_gas) = match apply_braindance_buy_transaction(&mut evm, &original_block, buy_data, data.pool.pool_variant) {
        Ok(v) => v,
        Err(e) => {
            sim_result.sim_result = Some(e);
            return Ok(sim_result);
        }
    };
    sim_result.construct_buy_tax(buy_real_amount_out, buy_amount_out);
    sim_result.buy_gas = buy_gas;

    let sell_data = match data.pool.pool_variant {
        PoolVariant::UniswapV2 => tx_builder::build_swap_v2_data(
            buy_real_amount_out,
            data.pool.address,
            data.intermediary_token,
            data.startend_token,
        ),
        
    };

    let (sell_amount_out, sell_real_amount_out, sell_gas) = match apply_braindance_sell_transaction(&mut evm, &original_block, sell_data, data.pool.pool_variant) {
        Ok(v) => v,
        Err(e) => {
            sim_result.sim_result = Some(e);
            return Ok(sim_result);
        }
    };
    sim_result.construct_sell_tax(sell_real_amount_out, sell_amount_out);
    sim_result.sell_gas = sell_gas;

    Ok(sim_result)   
    
    // I need the token info
}
