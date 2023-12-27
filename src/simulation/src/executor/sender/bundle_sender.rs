use crate::{
    utils::{
        create_websocket_client,
        dotenv::get_bundle_signer,
    },
    stream::BlockInfo,
    portfolio::{
        OrderType,
    },
    executor::{
        TransactionEvent,
        TransactionEventBuilder,
        OrderEventWithResponse
    },
    types::{
        TransactionId
    }
};
//use super::error::SenderError;
use std::sync::Arc;
use ethers::{
    prelude::{
        H256,
        Bytes,
        Provider,
        LocalWallet,
        Ws
    },
    utils::keccak256
};
use tokio;
use std::collections::{
    HashMap
};
//use ethers_flashbots::{BundleRequest};
use ethers_flashbots::*;
use reqwest::Url;


pub struct BundleRelay {
    pub flashbots_client:
        FlashbotsMiddleware<Arc<Provider<Ws>>, LocalWallet>,
    pub relay_name: String,
}

impl BundleRelay {
    pub fn new(
        relay_end_point: Url,
        relay_name: String,
        client: &Arc<Provider<Ws>>,
    ) -> Result<BundleRelay, url::ParseError> {
        // Extract wallets from .env keys
        let bundle_signer = get_bundle_signer();
        // Setup the Ethereum client with flashbots middleware
        let flashbots_client =
            FlashbotsMiddleware::new(client.clone(), relay_end_point, bundle_signer);

        Ok(BundleRelay {
            flashbots_client,
            relay_name,
        })
    }
}


pub async fn get_all_relay_endpoints() -> Vec<BundleRelay> {
    let client = create_websocket_client().await.unwrap();

    let endpoints = vec![
        ("flashbots", "https://relay.flashbots.net/"),
        ("builder0x69", "http://builder0x69.io/"),
        ("edennetwork", "https://api.edennetwork.io/v1/bundle"),
        ("beaverbuild", "https://rpc.beaverbuild.org/"),
        ("lightspeedbuilder", "https://rpc.lightspeedbuilder.info/"),
        ("eth-builder", "https://eth-builder.com/"),
        ("ultrasound", "https://relay.ultrasound.money/"),
        ("agnostic-relay", "https://agnostic-relay.net/"),
        ("relayoor-wtf", "https://relayooor.wtf/"),
        ("rsync-builder", "https://rsync-builder.xyz/"),
        ("blocknative", "https://api.blocknative.com/v1/auction"),
        ("blox-route", "https://mev.api.blxrbdn.com/"),
        ("build-ai", "https://buildai.net/"),
        ("gmbit", "https://builder.gmbit.co/rpc"),
        ("payload-de", "https://rpc.payload.de/"),
        ("titan-builder", "https://rpc.titanbuilder.xyz/")
        //"http://relayooor.wtf/",
        //"http://mainnet.aestus.live/",
        //"https://mainnet-relay.securerpc.com",
        //"http://agnostic-relay.net/",
        //"http://relay.ultrasound.money/",
    ];

    let mut relays: Vec<BundleRelay> = vec![];

    for (name, endpoint) in endpoints {
        let relay = BundleRelay::new(Url::parse(endpoint).unwrap(), name.into(), &client).unwrap();
        relays.push(relay);
    }

    relays
}

async fn send_bundle(
    relay: BundleRelay,
    bundle: BundleRequest,    
) -> bool {
    let pending_bundle = match relay.flashbots_client.send_bundle(&bundle).await {
        Ok(pb) => pb,
        Err(_) => {
            //log::error!("Failed to send bundle to {:?} reason: {:?}", relay.relay_name, e);
            log::warn!("Failed to send bundle to {:?}", relay.relay_name);
            return false;
        }
    };
    let bundle_hash = pending_bundle.bundle_hash;
    log::info!("Bundle {:?} sent to {:?} targeting {:?}", bundle_hash, relay.relay_name, bundle.block().unwrap());

    
    let is_bundle_included = match pending_bundle.await {
        Ok(a) => true,
        Err(ethers_flashbots::PendingBundleError::BundleNotIncluded) => false,
        Err(e) => {
            log::error!( "Bundle rejected due to error : {:?}", e );
            false
        }
    };
    //let bundle_hash = bundle_hash.clone();
    let block = bundle.block().unwrap().clone();
    tokio::task::spawn(async move {
        match relay.flashbots_client.get_bundle_stats(bundle_hash, block).await {
            Ok(stats) => {
                log::info!( "Bundle stats for {:?} stats: {:?}", bundle_hash, stats);
            },
            Err(_) => {
                //log::error!( "Failed to get bundle stats for {:?} reason: {:?}", bundle_hash, e);
            }
    
        };    
    });
   
    #[cfg(feature = "dry")]
    {
        return true;
    }
    is_bundle_included
}

pub async fn send_orders(
    orders: Vec<OrderEventWithResponse>,
    target_block: BlockInfo,
) {
    let (backrun_txs, frontrun_txs, normal_txs) = separate_orders(orders).await;

    println!("Backrun txs: {:?}", backrun_txs.len());
    println!("Backrun frontrun_txs: {:?}", frontrun_txs.len());
    println!("normal_txs txs: {:?}", normal_txs.len());

    let mut bundles_with_response = Vec::new();
    for (signed_tx, orders) in backrun_txs {
        let bundle = construct_bundle(
            BundleType::Backrun(signed_tx),
            orders,
            target_block.clone()
        ).await;
        bundles_with_response.push(bundle);
    }
    for (signed_tx, orders) in frontrun_txs {
        let bundle = construct_bundle(
            BundleType::Frontrun(signed_tx),
            orders,
            target_block.clone()
        ).await;
        bundles_with_response.push(bundle);
    }

    {
        let bundle = construct_bundle(
            BundleType::Normal,
            normal_txs,
            target_block.clone()
        ).await;
        bundles_with_response.push(bundle);
    }
    println!("bundles_with_response: {:?}", bundles_with_response.len());
    for (bundle, reponse) in bundles_with_response {

        let target_block = target_block.clone();   
        tokio::task::spawn(async move {
            let relays = get_all_relay_endpoints().await;

            let mut results = Vec::new();

            for relay in relays {

                let bundle = bundle.clone();           
                  
                results.push(tokio::task::spawn(send_bundle(
                    relay,
                    bundle
                )));                
            }
            let results = futures::future::join_all(results).await;
            let results = results
                .into_iter()
                .map(|r| r.unwrap())
                .collect::<Vec<_>>();

            let is_failed = results.iter().all(|f| !f);
            if is_failed {
                log::error!( "Failed to send bundle targetting {:?}", target_block.number);
            } else {
                // If transaction failed, do not report back to trader, just drop the sender
                for resp in reponse {
               
                    let transaction_builder = match resp.transaction.build().await {
                        Ok(a) => a,
                        Err(e) => {
                            log::error!( "Failed to build resposne transaction {:?}", e);
                            continue;
                        }
                    };
                    match resp.response.send(transaction_builder).await {
                        Ok(_) => {},
                        Err(e) => {
                            // TODO: In case if the TX type is send bundle until finish, retry it
                            log::error!( "Failed to send response to trader {:?}", e);
                        }
                    }
                }
            }
        });
    }
}

async fn separate_orders(
    orders: Vec<OrderEventWithResponse>,
) -> (HashMap<Bytes, Vec<OrderEventWithResponse>>, HashMap<Bytes, Vec<OrderEventWithResponse>>, Vec<OrderEventWithResponse>) {
    let mut backrun_collection = HashMap::<Bytes, Vec<OrderEventWithResponse>>::new();
    let mut frontrun_collection = HashMap::<Bytes, Vec<OrderEventWithResponse>>::new();
    let mut normal_collection = Vec::<OrderEventWithResponse>::new();

    for (order, response) in orders {
        match &order.order_type  {
            OrderType::Backrun(tx) => {
                let rlp = tx.rlp();
                match backrun_collection.entry(rlp) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {                        
                        entry.get_mut().push((order.clone(), response));
                    },
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(vec![(order.clone(), response)]);
                    }
                };
            },
            OrderType::Frontrun(tx) => {
                let rlp = tx.rlp();
                match frontrun_collection.entry(rlp) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {                        
                        entry.get_mut().push((order.clone(), response));
                    },
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(vec![(order.clone(), response)]);
                    }
                };
            },
            OrderType::Normal => {
                normal_collection.push((order, response));                
            },
        }  
    }
    (backrun_collection, frontrun_collection, normal_collection)
}


enum BundleType {
    Backrun(Bytes),
    Frontrun(Bytes),
    Normal,
}

#[derive(Debug)]
struct BundleTransaction {
    pub response: tokio::sync::mpsc::Sender<TransactionEvent>,
    pub transaction: TransactionEventBuilder,
}


async fn construct_bundle(
    signed_traget_tx: BundleType,
    orders: Vec<OrderEventWithResponse>,
    target_block: BlockInfo,
) -> (BundleRequest, Vec<BundleTransaction>) {
    let mut bundle_request = BundleRequest::new();
    
    bundle_request = bundle_request
        .set_block(target_block.number)
        .set_simulation_block(target_block.number - 1);
        //.set_simulation_timestamp(target_block.timestamp)
        //.set_min_timestamp(target_block.timestamp)
        //.set_max_timestamp(target_block.timestamp);
    let mut transaction_responses = Vec::new();
    let mut bundle_txs = Vec::new();

    for (order, response) in orders {
        let mut signed_txs = Vec::new();
        let moved_order = order.clone();
        moved_order.transactions
            .into_iter()
            .for_each(|tx_signer| {
                signed_txs.push(tokio::task::spawn(
                    tx_signer.generate_signed_eip1559_transactions(
                    target_block.base_fee.clone() + order.priority.max_prio_fee_per_gas,
                        order.priority.max_prio_fee_per_gas
                    )                    
                ));
            });
       
        let signed_txs = futures::future::join_all(signed_txs).await;
        let mut signed_txs = signed_txs
                .into_iter()
                .map(|r| r.unwrap().unwrap())
                .collect::<Vec<_>>();
        
        let hashes = signed_txs
            .iter()
            .map(|b| H256::from(keccak256(b)))
            .collect::<Vec<H256>>();
        
        bundle_txs.append(&mut signed_txs);
        
        transaction_responses.push(BundleTransaction {
            transaction: 
            TransactionEvent::builder()
                .transaction_id(TransactionId::from(&order.order_id))
                .order(order)
                .hashes(hashes),
                
            response
        })
    }
    

    match signed_traget_tx {
        BundleType::Backrun(signed_tx) => {
            bundle_request = bundle_request.push_transaction(signed_tx);
            for tx in bundle_txs {
                bundle_request = bundle_request.push_transaction(tx)
            }
        },
        BundleType::Frontrun(signed_tx) => {
            for tx in bundle_txs {
                bundle_request = bundle_request.push_transaction(tx)
            }
            bundle_request = bundle_request.push_transaction(signed_tx);
        },
        BundleType::Normal => {
            for tx in bundle_txs {
                bundle_request = bundle_request.push_transaction(tx)
            }
        }
    }

    (bundle_request, transaction_responses)
}

