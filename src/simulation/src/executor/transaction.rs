use crate::{
    portfolio::{
        OrderEvent,
    },
    utils::create_websocket_client,
    types::{
        TransactionId,
        deserialize_transaction_id,
        serialize_transaction_id
    }
};
use futures;
use tokio;
use super::error::BuilderError;
use ethers::{prelude::{H256, Transaction}, providers::Middleware};
use serde::{Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEvent
{
    #[serde(deserialize_with = "deserialize_transaction_id", serialize_with = "serialize_transaction_id")]
    pub transaction_id: TransactionId,
    pub hashes: Vec<H256>,
    pub order: OrderEvent,
    #[serde(skip_serializing)]
    pub transactions: Vec<Option<Transaction>>,
}

impl TransactionEvent 
{
    pub fn builder() -> TransactionEventBuilder {
	    TransactionEventBuilder::new()
    }

}


#[derive(Debug, Default)]
pub struct TransactionEventBuilder
{
    pub transaction_id: Option<TransactionId>,
    pub hashes: Option<Vec<H256>>,
    pub order: Option<OrderEvent>,
}

async fn fetch_transaction_bodies(hashes: Vec<H256>) -> Vec<Option<Transaction>> {
    let client = create_websocket_client().await.unwrap();

    let mut results = vec![];
    for hash in hashes {
        let client = client.clone();
        results.push(tokio::task::spawn(async move {
            client.get_transaction(hash).await
        }
            
        ));
    }
    let results = futures::future::join_all(results).await;
    let results = results
                        .into_iter()
                        .map(|f| f.unwrap().unwrap_or_default())
                        .collect::<Vec<_>>();    
    results

}

impl TransactionEventBuilder
{
    pub fn new() -> Self {
        Self::default()
    }

    

    pub fn transaction_id(self, value: TransactionId) -> Self {
        Self {
            transaction_id: Some(value),
            ..self
        }
    }

    pub fn hashes(self, value: Vec<H256>) -> Self {
        Self {
            hashes: Some(value),
            ..self
        }
    }

    pub fn order(self, value: OrderEvent) -> Self {
        Self {
            order: Some(value),
            ..self
        }
    }

    pub async fn build(self) -> Result<TransactionEvent, BuilderError> {

        let hashes = self
                .hashes
                .ok_or(BuilderError::BuilderIncomplete("hashes"))?;

        let transactions = fetch_transaction_bodies(hashes.clone()).await;

        Ok(TransactionEvent {           
	        transaction_id: self
                .transaction_id
                .ok_or(BuilderError::BuilderIncomplete("transaction_id"))?,       
            order: self
                .order
                .ok_or(BuilderError::BuilderIncomplete("order"))?,
            hashes,
            transactions
                
	    
        })
    }
}