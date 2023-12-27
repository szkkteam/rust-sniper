use crate::{
    dex::{Pool, Dex},
    token::Token,
    utils,
};
use dashmap::DashMap;
use ethers::prelude::*;
use futures::stream::FuturesUnordered;
use revm::{
    db::{CacheDB, EmptyDB},
    primitives::{AccountInfo, Bytecode},
};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::Arc,
};
use hashbrown::{
    HashMap,
};

pub type StateDiff = BTreeMap<Address, AccountDiff>;

// Extract state diffs from a given tx
//
// Arguments:
// * `client`: Websocket provider used for making rpc calls
// * `meats`: Vec of transactions to extract state diffs from
// * `block_num`: Block number of the block the txs are in
//
// Returns:
// Some(BTreeMap<Address, AccountDiff>): State diffs for each address)
// None: If encountered error or state diffs are non existant
pub async fn get_from_txs(
    client: &Arc<Provider<Ws>>,
    transactions: &Vec<Transaction>,
    block_number: BlockNumber,
) -> Option<BTreeMap<Address, AccountDiff>> {
    // add statediff trace to each transaction
    let req = transactions
        .iter()
        .map(|tx| (tx, vec![TraceType::StateDiff]))
        .collect();

    let block_traces = match client.trace_call_many(req, Some(block_number)).await {
        Ok(x) => x,
        Err(_) => {
            // should throw error here but guess None also works :<
            return None;
        }
    };

    let mut merged_state_diffs = BTreeMap::new();

    block_traces
        .into_iter()
        .flat_map(|bt| bt.state_diff.map(|sd| sd.0.into_iter()))
        .flatten()
        .for_each(|(address, account_diff)| {
            match merged_state_diffs.entry(address) {
                Entry::Vacant(entry) => {
                    entry.insert(account_diff);
                }
                Entry::Occupied(_) => {
                    // Do nothing if the key already exists
                    // we only care abt the starting state
                }
            }
        });

    Some(merged_state_diffs)
}

pub async fn get_ending_state_from_txs(
    client: &Arc<Provider<Ws>>,
    transactions: &Vec<Transaction>,
    block_number: BlockNumber,
) -> Option<BTreeMap<Address, AccountDiff>> {
    // add statediff trace to each transaction
    let req = transactions
        .iter()
        .map(|tx| (tx, vec![TraceType::StateDiff]))
        .collect();
    
    let block_traces = match client.trace_call_many(req, Some(block_number)).await {
        Ok(x) => x,
        Err(_) => {
            // should throw error here but guess None also works :<
            return None;
        }
    };

    let mut merged_state_diffs = BTreeMap::new();
    block_traces
        .into_iter()
        .flat_map(|bt| bt.state_diff.map(|sd| sd.0.into_iter()))
        .flatten()
        .for_each(|(address, account_diff)| {
            match merged_state_diffs.entry(address) {
                Entry::Vacant(entry) => {
                    entry.insert(account_diff);
                }
                Entry::Occupied(_) => {
                    //entry.insert(account_diff);                 
                }
            }
        });

    Some(merged_state_diffs)
}

pub async fn get_from_hash(
    client: &Arc<Provider<Ws>>,
    hash: H256,
) -> Option<BTreeMap<Address, AccountDiff>> {
    
    let block_traces = match client.trace_replay_transaction(hash, vec![TraceType::StateDiff]).await {
        Ok(x) => x,
        Err(_) => {
            // should throw error here but guess None also works :<
            return None;
        }
    };

    let mut merged_state_diffs = BTreeMap::new();
    block_traces.state_diff
        .into_iter()
        .map(|sd| sd.0.into_iter())
        .flatten()
        .for_each(|(address, account_diff)| {
            match merged_state_diffs.entry(address) {
                Entry::Vacant(entry) => {
                    entry.insert(account_diff);
                }
                Entry::Occupied(mut entry) => {
                    entry.insert(account_diff);                 
                }
            }
        });
         
    Some(merged_state_diffs)
}

pub fn get_weth_balance_change(
    owner: Address,
    state_diffs: &BTreeMap<Address, AccountDiff>,
) -> Option<(U256, U256)> {
    let weth_state_diff = &state_diffs
        .get(&utils::constants::get_weth_address())?
        .storage;

    let storage_key = TxHash::from(ethers::utils::keccak256(abi::encode(&[
        abi::Token::Address(owner),
        abi::Token::Uint(U256::from(3)),
    ])));
    let(from, to) = match weth_state_diff.get(&storage_key)? {
        Diff::Changed(c) => {
            (U256::from(c.from.to_fixed_bytes()), U256::from(c.to.to_fixed_bytes()))
        },
        _ => { (U256::zero(), U256::zero()) }
    };
        
    Some((from, to))
}

pub fn extract_tokens(
    state_diffs: &BTreeMap<Address, AccountDiff>,
    all_tokens: &DashMap<Address, Token>,
) -> Option<Vec<Token>> {
    // capture all addresses that have a state change and are also a pool
    let touched_tokens: Vec<Token> = state_diffs
        .keys()
        .filter_map(|e| all_tokens.get(e).map(|p| (*p.value()).clone()))
        .collect();

    Some(touched_tokens)
}

fn uint256_to_h160(from: H256) -> H160 {
    let mut bytes = vec![];
    let mut cnt = 0;
    for b in from.to_fixed_bytes().iter().rev() {
        let c = b.to_be_bytes();
        bytes.extend(c);
        cnt += 1;
        if cnt >= 20 {
            break;
        }
        
    }
    bytes.reverse();
    H160::from_slice(&bytes)
}

pub fn update_pairs_for_tokens(
    state_diffs: &BTreeMap<Address, AccountDiff>,
    touched_tokens: &Vec<Token>,
    all_factory: &HashMap<Address, Dex>
) -> Option<HashMap<Address, Pool>> {
    let mut pools_created = HashMap::new();

    for token in touched_tokens {

        let touched_factories: Vec<Dex> = state_diffs
            .keys()
            .filter_map(|a| all_factory.get(a).map(|p| (*p).clone()))
            .collect();

        for factory in touched_factories {
            let factory_state_diff = &state_diffs
                .get(&factory.address)?
                .storage;

            // Compute the pair lenght storage key - uint256(3)
            let pair_length_storage_key = TxHash::from_uint(&U256::from(3));

            // Get the new length of the pairs
            let pair_length = match factory_state_diff.get(&pair_length_storage_key)? {
                Diff::Changed(c) => {
                    U256::from(c.from.to_fixed_bytes())
                },
                _ => continue,
            };
            
            // Compute the pair at the extracted length - uint256(keccak256(abi.encodePacked(uint256(3)))) + (pairLenght * 256/256) - 1          
            let base: U256 = ethers::utils::keccak256(abi::encode(&[
                abi::Token::Uint(U256::from(3))
            ])).into();
            let base = base + pair_length;
            let pair_address_storage_key = TxHash::from_uint(&base);
            
            let pair_address = match factory_state_diff.get(&pair_address_storage_key)? {
                Diff::Changed(c) => {
                    uint256_to_h160(c.to)
                },
                _ => continue,
            };

            let pair_state_diff = &state_diffs
                .get(&pair_address)?
                .storage;

            // Compute storage key {6} for pair address
            let token0_storage_key = TxHash::from_uint(&U256::from(6));
            // Compute storage key {7} for pair address
            let token1_storage_key = TxHash::from_uint(&U256::from(7));
            // Get token0  from pair
            let token0 = match pair_state_diff.get(&token0_storage_key)? {
                Diff::Born(c) => {
                    uint256_to_h160(*c)
                },
                _ => continue,
            };
            // Get token1 from pair
            let token1 = match pair_state_diff.get(&token1_storage_key)? {
                Diff::Born(c) => {
                    uint256_to_h160(*c)
                },
                _ => continue,
            };

            if ![token0, token1].contains(&token.address) {
                continue;
            }

            pools_created.insert(token.address, Pool { 
                address: pair_address,
                token_0: token0,
                token_1: token1,
                pool_variant: factory.pool_variant
            });

        }
    }
    Some(pools_created)
}


pub fn empty_db() -> CacheDB<EmptyDB> {
    CacheDB::new(EmptyDB::default())
}

// Turn state_diffs into a new cache_db
//
// Arguments:
// * `state`: Statediffs used as values for creation of cache_db
// * `block_num`: Block number to get state from
// * `provider`: Websocket provider used to make rpc calls
//
// Returns:
// Ok(CacheDB<EmptyDB>): cacheDB created from statediffs, if no errors
// Err(ProviderError): If encountered error during rpc calls
pub async fn to_cache_db(
    state: &BTreeMap<Address, AccountDiff>,
    block_num: Option<BlockId>,
    provider: &Arc<Provider<Ws>>,
) -> Result<CacheDB<EmptyDB>, ProviderError> {
    let mut cache_db = CacheDB::new(EmptyDB::default());

    let mut futures = FuturesUnordered::new();

    for (address, acc_diff) in state.iter() {
        let nonce_provider = provider.clone();
        let balance_provider = provider.clone();
        let code_provider = provider.clone();

        let addy = *address;

        let future = async move {
            let nonce = nonce_provider
                .get_transaction_count(addy, block_num)
                .await?;

            let balance = balance_provider.get_balance(addy, block_num).await?;

            let code = code_provider.get_code(addy, block_num).await?;

            Ok::<(AccountDiff, Address, U256, U256, Bytes), ProviderError>((
                acc_diff.clone(),
                *address,
                nonce,
                balance,
                code,
            ))
        };

        futures.push(future);
    }

    while let Some(result) = futures.next().await {
        let (acc_diff, address, nonce, balance, code) = result?;
        let info = AccountInfo::new(balance.into(), nonce.as_u64(), Bytecode::new_raw(code.0));
        cache_db.insert_account_info(address.0.into(), info);

        acc_diff.storage.iter().for_each(|(slot, storage_diff)| {
            let slot_value: U256 = match storage_diff.to_owned() {
                Diff::Changed(v) => v.from.0.into(),
                Diff::Died(v) => v.0.into(),
                _ => {
                    // for cases Born and Same no need to touch
                    return;
                }
            };
            let slot: U256 = slot.0.into();
            cache_db
                .insert_account_storage(address.0.into(), slot.into(), slot_value.into())
                .unwrap();
        });
    }

    Ok(cache_db)
}
