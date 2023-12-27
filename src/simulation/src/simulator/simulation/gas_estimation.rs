use ethers::prelude::*;
use revm::primitives::{ExecutionResult, TransactTo, B160 as rAddress};

//use crate::prelude::access_list::AccessListInspector;
use super::fork_db::fork_factory::ForkFactory;

use crate::{
    stream::BlockInfo,
};
use super::{
    SimulationError,
};

use super::{
    helpers::{
        setup_block_state, convert_access_list
    },
    inspectors::{AccessListInspector}
};

pub async fn estimage_gas(
    txs: Vec<Transaction>,
    mut estimate_txs: Vec<Transaction>,
    target_block: &BlockInfo,
    fork_factory: &mut ForkFactory,
) -> Result<Vec<Transaction>, SimulationError> {
    #[cfg(feature = "dry")] 
    {   
        use super::helpers::inject_test_sniper;
        // Only the stubbed version of the contract works here, DO NOT USE THAT IN PROD!
        let owner = estimate_txs.last().unwrap().from.clone();
        let contract = estimate_txs.last().unwrap().to.clone().unwrap();

        inject_test_sniper(
            owner,
            contract,
            fork_factory,
        );
    }
    let fork_db = fork_factory.new_sandbox_fork();

    let mut evm = revm::EVM::new();

    evm.database(fork_db);

    // First apply the original block
    setup_block_state(&mut evm, &target_block);

    for tx in txs.iter() {
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

    for tx in estimate_txs.iter_mut() {
        evm.env.tx.caller = tx.from.0.clone().into();
        evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
        //evm.env.tx.nonce = Some(tx.nonce.as_u64());
        evm.env.tx.nonce = None;
        evm.env.tx.gas_limit = tx.gas.as_u64();
        evm.env.tx.gas_price = target_block.base_fee.into();
        evm.env.tx.gas_priority_fee = None;
        evm.env.tx.access_list = Vec::default();

        let mut access_list_inspector = AccessListInspector::new(
            tx.from,
            tx.to.unwrap_or_default()
        );
        evm.inspect_ref(&mut access_list_inspector)
            .map_err(|e| SimulationError::FrontrunEvmError(e))?;
        let access_list = access_list_inspector.into_access_list();       
        //println!("access_list: {:?}", access_list) ;
        evm.env.tx.access_list = access_list.clone();
    
        let result = match evm.transact_commit() {
            Ok(result) => result,
            Err(e) => {
                return Err(SimulationError::FrontrunEvmError(e))
            },
        };
        match result {
            ExecutionResult::Success { .. } => { /* continue */ }
            ExecutionResult::Revert { output, .. } => {
                return Err(SimulationError::FrontrunReverted(output))
            }
            ExecutionResult::Halt { reason, .. } => 
                return Err(SimulationError::FrontrunHalted(reason)),
        };
        tx.access_list = Some(convert_access_list(access_list));
        tx.gas = (U256::from(result.gas_used()) * 10) / 7; // gasused = 70% gaslimit
    }
    
    Ok(estimate_txs)
}

