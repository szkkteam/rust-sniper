use std::sync::Arc;
use crate::{
    utils
};
use eyre;
use tokio::sync::mpsc::{channel, Receiver};
use ethers::prelude::*;

/// Subscribe to the rpc endpoint "SubscribePending"
pub async fn subscribe_pending_txs_with_body(
    client: &Arc<Provider<Ws>>,
) -> Result<SubscriptionStream<'_, Ws, Transaction>, ProviderError>
{
    // this rpc is erigon specific
    client.subscribe(["newPendingTransactionsWithBody"]).await
}

pub async fn stream_pending_transaction() -> eyre::Result<Receiver<Transaction>> {
    let client = utils::create_websocket_client().await.unwrap();

    let (tx, rx) = channel(100);

    tokio::spawn(async move {
        let mut mempool_stream = if let Ok(stream) = subscribe_pending_txs_with_body(&client).await {
            stream
        } else {
            panic!("Could not start mempool stream!")
        };

        while let Some(transaction) = mempool_stream.next().await {            
           
            if let Err(_) = tx.send(transaction).await {
                //TODO: Logging
                continue;
            }

        }
    });
    Ok(rx)
}