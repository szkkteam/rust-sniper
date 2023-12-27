use crate::{
    utils::{
        dotenv,
        constants,
        create_websocket_client,
        sign_eip1559,
    },
    stream::BlockInfo,
    portfolio::{
        OrderEvent,
        OrderType,
        TransactionSigner,
        profile::Priority,

    },
    executor::{
        TransactionEvent,
        TransactionEventBuilder,
        OrderEventWithResponse
    },
    types::{
        OrderId,
        TransactionId
    }
};
use super::error::SenderError;
use std::{sync::Arc, vec};
use ethers::{
    prelude::{
        Address,
        Transaction,
        U256,
        H256,
        U64,
        Bytes,
        BlockNumber,
        Provider,
        ProviderError,
        NameOrAddress,
        LocalWallet,
        Eip1559TransactionRequest,
        Ws,
        Http,
        QuorumProvider,
        Quorum,
        Middleware,
        WeightedProvider,
        JsonRpcClient
    },
    utils::keccak256
};
use tokio;
use std::str::FromStr;
use std::collections::{
    HashSet,
    HashMap
};
use reqwest::Url;



pub async fn get_provider() -> Vec<Provider<Http>> {
    let mut list = vec![];

    list.push(Provider::<Ws>::connect("ws://localhost:8545").await.unwrap());
    /*
    let provider: QuorumProvider = QuorumProvider::dyn_rpc()
        .add_provider(WeightedProvider::new(Box::new(Http::from_str("https://eth.llamarpc.com").unwrap())))
        .add_provider(WeightedProvider::new(Box::new(Ws::connect("ws://localhost:8545").await.unwrap())))
        .quorum(Quorum::ProviderCount(1))
        .build();
    provider
     */
    list
}

enum CalculatedGasType {
    Eip1559(U256, U256),
    Legacy(U256),
}

fn calculate_gas(
    order_type: &OrderType,
    priority: &Priority,
    base_fee: U256,
) -> CalculatedGasType {
    // 1) Check if priority is configured if yes use it to send a high prio transaction
    if priority.max_prio_fee_per_gas > U256::zero() {
        CalculatedGasType::Eip1559(
            base_fee * 2 + priority.max_prio_fee_per_gas, // max_fee_per_gas
            priority.max_prio_fee_per_gas // max_prio_fee_per_gas
        ) 
    } else {
        match order_type {
            OrderType::Normal => {
                CalculatedGasType::Eip1559(
                    base_fee * 2 + U256::from(1000000000), // max_fee_per_gas
                    U256::from(1000000000) // 1 gwei extra
                ) 
            },
            OrderType::Backrun(tx) => {
                match tx.transaction_type {
                    Some(ethers::types::U64([0])) => {
                        // legacy tx
                        CalculatedGasType::Legacy(tx.gas_price.unwrap_or_default())
                    }
                    Some(_) => {
                        CalculatedGasType::Eip1559(
                            tx.max_priority_fee_per_gas.unwrap_or_default(),
                            tx.max_fee_per_gas.unwrap_or_default()
                        )                        
                    }
                    None => {
                        // legacy tx
                        CalculatedGasType::Legacy(tx.gas_price.unwrap_or_default())
                    }
                }
            },
            OrderType::Frontrun(tx) => {
                match tx.transaction_type {
                    Some(ethers::types::U64([0])) => {
                        // legacy tx
                        CalculatedGasType::Legacy(tx.gas_price.unwrap_or_default() + U256::from(1000000000)) // 1 gwei extra
                    }
                    Some(_) => {
                        CalculatedGasType::Eip1559(
                            tx.max_priority_fee_per_gas.unwrap_or_default() + U256::from(1000000000),
                            tx.max_fee_per_gas.unwrap_or_default() + U256::from(1000000000)
                        )                        
                    }
                    None => {
                        // legacy tx
                        CalculatedGasType::Legacy(tx.gas_price.unwrap_or_default() + U256::from(1000000000)) // 1 gwei extra
                    }
                }
            }
        }
    }
}

pub async fn send_order(
    order: OrderEventWithResponse,
    target_block: BlockInfo,
) {
 
    let target_gas = calculate_gas(
        &order.0.order_type,
        &order.0.priority,
        target_block.base_fee
    );
    let moved_order = order.clone();
    let mut signed_txs = vec![];

    // The lowest nonce tx should get the higher priority (or which one is defined earlier)
    // We will construct the txs in reverse order, and each one will have +1 gwei added 
    let one_gwei = U256::from(1000000000);
    let mut gas_extra = U256::zero();
    // Let's sign the txs in reverse order so we can just increase the 
    for tx_signer in moved_order.0.transactions.into_iter().rev() {
        let actual_extra_gas = one_gwei + one_gwei.checked_mul(gas_extra).unwrap_or_default();
        gas_extra += U256::from(1);

        signed_txs.push(
            match target_gas {
                CalculatedGasType::Eip1559(max_fee_per_gas, max_prio_fee_per_gas) => {
                    match tx_signer.generate_signed_eip1559_transactions(
                        max_fee_per_gas + max_prio_fee_per_gas + actual_extra_gas,
                        max_prio_fee_per_gas + actual_extra_gas
                    ).await {
                        Ok(v) => v,
                        Err(_) => { continue; }
                    }
                },
                CalculatedGasType::Legacy(_) => {
                    continue;
                }
            }
                              
        );
    }
    // Reverse back the transactions to the original
    let signed_txs = signed_txs.into_iter().rev().collect::<Vec<_>>();
    let hashes = signed_txs
            .iter()
            .map(|b| H256::from(keccak256(b)))
            .collect::<Vec<H256>>();
    
    let transactin_bulder = TransactionEvent::builder()
        .transaction_id(TransactionId::from(&order.0.order_id))
        .order(order.0)
        .hashes(hashes);

    

    let providers = get_provider().await;

    let mut transaction_results = vec![];

    for provider in providers {

        
        let signed_txs = signed_txs.clone();


        transaction_results.push(tokio::task::spawn(async move {
            let mut results = vec![];

            signed_txs.into_iter().for_each(|signed_tx| {
                results.push(tokio::task::spawn(provider.send_raw_transaction(signed_tx)))
            });
            
            let results = futures::future::join_all(results).await;
            let results = results
                        .into_iter()
                        .map(|r| r.unwrap().unwrap())
                        .collect::<Vec<_>>();
                
            let results = futures::future::join_all(results).await;
            let results = results
                        .into_iter()
                        .map(|r| r.unwrap())
                        .collect::<Vec<_>>();
                
            results
        }));
    }    
    let mut result_map = HashMap::new();
    let transaction_results = futures::future::join_all(transaction_results).await;
    let transaction_results = transaction_results
                        .into_iter()
                        .map(|r| r.unwrap())
                        .flatten()
                        .for_each(|receipt| {
                            match receipt {
                                Some(receipt) => {
                                    match result_map.entry(receipt.transaction_hash) {
                                        std::collections::hash_map::Entry::Occupied(_) => {},
                                        std::collections::hash_map::Entry::Vacant(entry) => {
                                            entry.insert(receipt);
                                        }
                                    };
                                    
                                },
                                None => {}
                            };
                        });


    /*
    signed_txs.into_iter().for_each(|signed_tx| {
        results.push(tokio::task::spawn(async {
            let result: Result<H256, _> = provider.raw("eth_sendRawTransaction", [signed_tx]).await;
            result
        }))
    });
    let results = futures::future::join_all(results).await;
    let results = results
                .into_iter()
                .map(|r| r.unwrap().unwrap())
                .collect::<Vec<_>>();
     */
}
