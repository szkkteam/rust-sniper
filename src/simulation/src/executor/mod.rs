use crate::{
    event::{Event, MessageTransmitter},    
    stream::{BlockOracle},
    portfolio::{
        OrderEvent,
        BlockTargetType,
        profile::TransactionType
    },
};
use parking_lot::Mutex;
use std::{collections::VecDeque, sync::Arc};
use ethers::prelude::{U256};
use tokio::sync::{mpsc, watch};
use tokio::time::{self, Duration};
use tokio;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod error;
use error::BuilderError;

mod sender;
use sender::{
    bundle_sender::send_orders
};

pub mod transaction;
pub use transaction::*;

pub type OrderEventWithResponse = (OrderEvent, mpsc::Sender<TransactionEvent>);

pub struct ExecutorLego<EventTx> 
where 
    EventTx: MessageTransmitter<Event> + Send,  
{    
    pub event_tx: EventTx,
    pub order_rx: mpsc::Receiver<OrderEventWithResponse>,
    pub block_stream: watch::Receiver<BlockOracle>,
}

pub struct Executor<EventTx>
where
    EventTx: MessageTransmitter<Event> + Clone,
{
    event_tx: EventTx,
    bundle_q: Arc<Mutex<VecDeque<OrderEventWithResponse>>>,
    block_stream: watch::Receiver<BlockOracle>,
    order_rx: mpsc::Receiver<OrderEventWithResponse>,
}

impl<EventTx> Executor<EventTx>
where
    EventTx: MessageTransmitter<Event> + Send + Clone + 'static,
{

    pub fn new(lego: ExecutorLego<EventTx>) -> Self {
        let bundle_q = VecDeque::with_capacity(50);
        let bundle_q = Mutex::new(bundle_q);
        let bundle_q = Arc::new(bundle_q);

        Self {
            event_tx: lego.event_tx,
            bundle_q,
            block_stream: lego.block_stream,
            order_rx: lego.order_rx,
        }
    }

    pub fn builder() -> ExecutorBuilder<EventTx> {
        ExecutorBuilder::new()
    }
    
    pub async fn run(mut self) {
        let oracle: BlockOracle = (*self.block_stream.borrow()).clone();
        let last_timestamp = oracle.latest.timestamp.as_u64();
        let start = SystemTime::now();
        let since_the_epoch = start
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        let current_timestamp = since_the_epoch.as_secs();
        if current_timestamp.abs_diff(last_timestamp) >= 10 {

        }


        loop {            
            tokio::select!{
                Ok(_block) = self.block_stream.changed() => {
                    let oracle: BlockOracle = (*self.block_stream.borrow()).clone();
                  
                    let block = oracle.next;
                    let bundle_q = self.bundle_q.clone();
                    let block_rx = self.block_stream.clone();

                    log::info!("{}", format!("Block {} confirmed.", oracle.latest.number));
                    self.event_tx.send(Event::BlockConfirmed(oracle.latest));

                    if !self.bundle_q.lock().is_empty() {
                        // Spawn a thread in order, if new TXs are coming, we must process them separately
                        tokio::spawn(async move {
                            time::sleep(Duration::from_millis(6000)).await;
                            
                            let mut for_next_block = Vec::new();
                            {
                                // Drain and process the bundle_q
                                let mut l_bundle_q = bundle_q.lock();

                                let mut for_future_block = l_bundle_q
                                    .drain(..)
                                    .collect::<Vec<OrderEventWithResponse>>();                    

                                for_next_block = for_future_block.clone();

                                for_next_block
                                    .retain(|e| {
                                        let target_block = match &e.0.block_target_type {
                                            BlockTargetType::Exact(block) => block,
                                            BlockTargetType::None => &block
                                        };
                                        target_block.number <= block.number}
                                    );
                                for_future_block
                                    .retain(|e| {
                                        let target_block = match &e.0.block_target_type {
                                            BlockTargetType::Exact(block) => block,
                                            BlockTargetType::None => &block
                                        };
                                        target_block.number > block.number
                                    });

                                for_future_block
                                    .into_iter()
                                    .for_each(|e| {
                                        l_bundle_q.push_back(e);
                                    }
                                );

                            }
                            log::info!("{}", format!("Preparing bundle for {:?} with {:?} orders", block.number, for_next_block.len()));
                            send_orders(
                                for_next_block,
                                block
                            ).await;
                            log::info!("{}", format!("Bundle finished"));
                        });
                    }
                     
                    
                    //println!("Bundle txs: {:?}", drained);
                },
                Some(order) = self.order_rx.recv() => {
                    let (order_event, _) = &order;
                    match &order_event.transaction_type {                        
                        TransactionType::Bundle { priority} => {
                            self.process_bundle(order).await;
                        },
                        TransactionType::Auto => {
                            let mut normal_order = order.clone();
                            normal_order.0.priority.max_prio_fee_per_gas = 
                            normal_order.0.priority.max_prio_fee_per_gas.checked_div(U256::from(2)).unwrap_or(normal_order.0.priority.max_prio_fee_per_gas);

                            self.process_bundle(order).await;
                            self.process_normal(normal_order).await;
                        },
                        _ => {
                            self.process_normal(order).await;
                        }
                    } 
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Placeholder `await` to allow other tasks to make progress
                }
                //else => { break; }
            }
        }
    }

    async fn process_normal(&mut self, order: OrderEventWithResponse) {
        let b_rx = self.block_stream.clone();
        tokio::spawn(async move {
           //process_normal_order(b_rx, r).await 
        });
    }

    async fn process_bundle(&mut self, order: OrderEventWithResponse) {
        let (order_event, _) = &order;
        let oracle: BlockOracle = (*self.block_stream.borrow()).clone();
        // Emit block confirmed event
        let next = oracle.next.clone();
        // In case if the target block is the current one, send the bundle immidiatly
        let target_block = match &order_event.block_target_type {
            BlockTargetType::Exact(block) => block,
            BlockTargetType::None => &next
        };
        // Get the last confirmed block timestamp
        let last_timestamp = oracle.latest.timestamp.as_u64();
        let start = SystemTime::now();
        let since_the_epoch = start
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        let current_timestamp = since_the_epoch.as_secs();
        if 
            // If we are early and the target block is the next block
            current_timestamp.abs_diff(last_timestamp) <= 10 &&
            target_block.number <= next.number
        {
            // execute it now
            tokio::spawn(async move {                                    
                //let client = create_websocket_client().await.unwrap();

                log::info!("{}", format!("Preparing bundle for {:?} with 1 order", next.number));
                send_orders(
                    vec![order],
                    next
                ).await;

                log::info!("{}", format!("Bundle finished"));
            });
        } else {
            // Only less then 2s left, we need to schedule it for the next block
            self.bundle_q
                .lock()
                .push_back(order);
        }
    }

}

pub struct ExecutorBuilder<EventTx>
where
    EventTx: MessageTransmitter<Event>,
{
    event_tx: Option<EventTx>,
    block_stream: Option<watch::Receiver<BlockOracle>>,
    order_rx: Option<mpsc::Receiver<OrderEventWithResponse>>,
}

impl<EventTx> ExecutorBuilder<EventTx>
where
    EventTx: MessageTransmitter<Event> + Clone,
{
    
    pub fn new() -> Self {
        Self {
            event_tx: None,
            block_stream: None,
            order_rx: None,
        }
    }

    pub fn event_tx(self, value: EventTx) -> Self {
        Self {
            event_tx: Some(value),
            ..self
        }
    }
    
    pub fn block_stream(self, value: watch::Receiver<BlockOracle>) -> Self {
        Self {
            block_stream: Some(value),
            ..self
        }
    }
    
    pub fn order_receiver(self, value: mpsc::Receiver<OrderEventWithResponse>) -> Self {
        Self {
            order_rx: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<Executor<EventTx>, BuilderError> {
        let bundle_q = VecDeque::with_capacity(50);
        let bundle_q = Mutex::new(bundle_q);
        let bundle_q = Arc::new(bundle_q);

        Ok(Executor {
            event_tx: self
                .event_tx
                .ok_or(BuilderError::BuilderIncomplete("event_tx"))?,         
            block_stream: self
                .block_stream
                .ok_or(BuilderError::BuilderIncomplete("block_stream"))?,                       
            order_rx: self
                .order_rx
                .ok_or(BuilderError::BuilderIncomplete("order_rx"))?,     
            bundle_q,
        })
    }
}
