use ethers::prelude::*;
use revm::primitives::{ExecutionResult,TransactTo, B160 as rAddress};

use super::fork_db::fork_db::ForkDB;

use crate::{
    utils::{
        constants::get_weth_address,
    },
    stream::BlockInfo,    
    token::Token,
};
use super::{
    SimulationError,
};

use super::{
    helpers::{
        setup_block_state, get_balance_of_evm, sniper_wallet_1_address
    },
    inspectors::{AccessListInspector}
};

pub async fn simulate_profit(
    contract: Address,
    _token: &Token,
    test_txs: &Vec<Transaction>,
    target_block: &BlockInfo,
    fork_db: ForkDB,
) -> Result<(u64, U256), SimulationError> {
    /*
    #[cfg(feature = "dry")] 
    {   
        return Ok((0, U256::zero()));
    }
     */
    let mut evm = revm::EVM::new();
    evm.database(fork_db);

    
    // First apply the original block
    setup_block_state(&mut evm, &target_block);

    #[cfg(feature = "dry")] 
    {   
        let tx = test_txs[0].clone();
        evm.env.tx.caller = tx.from.0.clone().into();
        evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
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
    }
    // Get the current balance holding of the contract    
    let starting_balance = get_balance_of_evm(
        get_weth_address(),
        contract,
        &target_block,
        &mut evm
    )?;

    let mut total_gas_cost = 0;    
    #[cfg(feature = "dry")] 
    {   
        let tx = test_txs[1].clone();
        evm.env.tx.caller = tx.from.0.clone().into();
        evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
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
        total_gas_cost += result.gas_used();

        let starting_balance = get_balance_of_evm(
            get_weth_address(),
            contract,
            &target_block,
            &mut evm
        )?;
    }

   
    #[cfg(not(feature = "dry"))] 
    {
        for tx in test_txs.iter() {
            evm.env.tx.caller = tx.from.0.clone().into();
            evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
            evm.env.tx.data = tx.input.0.clone();
            evm.env.tx.value = tx.value.into();
            evm.env.tx.nonce = None;
            evm.env.tx.gas_limit = tx.gas.as_u64();
            evm.env.tx.gas_price = target_block.base_fee.into();
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
            total_gas_cost += result.gas_used();
        };
    }
  
    /*
    let pair_token_ending_balance = get_balance_of_evm(
        token.address,
        token.pool.unwrap().address,
        &target_block,
        &mut evm
    )?;
     */
    // TODO: Use this for x*y=k price impact calculation
    //let sell_amount = pair_token_ending_balance - pair_token_starting_balance;

    let ending_balance = get_balance_of_evm(
        get_weth_address(),
        contract,
        &target_block,
        &mut evm
    )?;
    Ok((total_gas_cost, ending_balance.checked_sub(starting_balance).unwrap_or_default()))
}

pub async fn simulate_rug(
    contract: Address,
    txs: &Vec<Transaction>,
    test_txs: &Vec<Transaction>,
    target_block: &BlockInfo,
    fork_db: ForkDB,
) -> Result<(u64, U256), SimulationError> {
    let mut total_gas_cost = 0; 
    /*
    #[cfg(feature = "dry")] 
    {   
        return Ok((0, U256::zero()));
    }
     */

    let mut evm = revm::EVM::new();

    evm.database(fork_db);
    
    // First apply the original block
    setup_block_state(&mut evm, &target_block);
    
    #[cfg(feature = "dry")] 
    {   
        let tx = test_txs[0].clone();
        evm.env.tx.caller = tx.from.0.clone().into();
        evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
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
    }
    // Get the current balance holding of the contract    
    let starting_balance = get_balance_of_evm(
        get_weth_address(),
        contract,
        &target_block,
        &mut evm
    )?;

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
    #[cfg(feature = "dry")] 
    {   
        let tx = test_txs[1].clone();
        evm.env.tx.caller = tx.from.0.clone().into();
        evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
        evm.env.tx.data = tx.input.0.clone();
        evm.env.tx.value = tx.value.into();
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
        total_gas_cost += result.gas_used();
    }
    #[cfg(not(feature = "dry"))] 
    {   
        for tx in test_txs.iter() {
            evm.env.tx.caller = tx.from.0.clone().into();
            evm.env.tx.transact_to = TransactTo::Call(tx.to.unwrap_or_default().0.clone().into());
            evm.env.tx.data = tx.input.0.clone();
            evm.env.tx.value = tx.value.into();
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
            total_gas_cost += result.gas_used();
        };
    }
    
    let ending_balance = get_balance_of_evm(
        get_weth_address(),
        contract,
        &target_block,
        &mut evm
    )?;
    Ok((total_gas_cost, ending_balance.checked_sub(starting_balance).unwrap_or_default()))

}

