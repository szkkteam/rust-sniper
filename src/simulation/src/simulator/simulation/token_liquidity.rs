use ethers::prelude::*;
use num_bigfloat::BigFloat;
use revm::primitives::{TransactTo, B160 as rAddress};

//use crate::prelude::access_list::AccessListInspector;
use super::fork_db::fork_db::ForkDB;

use crate::{
    
    stream::BlockInfo,
    
    token::Token,
    utils::constants::get_weth_address,
};
use super::{
    SimulationError,
};

use super::helpers::{
    setup_block_state, get_balance_of_evm, get_total_supply_of_evm
};
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
pub fn liquidity_ratio(
    token: &Token,
    txs: &Vec<Transaction>,
    start_block: &BlockInfo,
    mut fork_db: ForkDB,
) -> Result<(BigFloat, U256, U256), SimulationError> {
    let mut evm = revm::EVM::new();
    evm.database(fork_db.clone());
    // First apply the original block    
    setup_block_state(&mut evm, &start_block);

    apply_transactions(&mut evm, txs);

    let total_supply = get_total_supply_of_evm(token.address, start_block, &mut evm)?;
    let pool_balance = get_balance_of_evm(
        token.address,
        token.pool.unwrap().address,
        start_block,
        &mut evm
    )?;
    let pool_weth_balance = get_balance_of_evm(
        token.get_paired_with().unwrap_or(get_weth_address()),
        token.pool.unwrap().address,
        start_block,
        &mut evm
    )?;
    let ratio = BigFloat::from_u128(pool_balance.as_u128()) / BigFloat::from_u128(total_supply.as_u128()) * BigFloat::from(100);
    Ok((ratio, pool_balance, pool_weth_balance))
}