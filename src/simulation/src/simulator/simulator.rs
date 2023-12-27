use crate::{
    event::{Event, MessageTransmitter},    
    token::Token,
    types::TraderId,
    stream::{BlockOracle, BlockInfo},
};
use ethers::{prelude::{
    Address,
    Transaction,
    Provider,
    Ws
}};
use futures;
use dashmap::{DashMap, mapref};
use tokio::{sync::{mpsc, watch, broadcast}, time::Instant};
use std::{collections::VecDeque, sync::Arc, vec};

use super::{
    error::EngineError,
    event::{
        SimulationEvent,
        SellSimulationEvent,
        SimulationState,
        SimulationStateChanged,
        SimulationStateLaunch,
    },
    SimulatorRequest,    
    simulation::{
        prepare_database,
        simulate_token,
        estimage_gas,
        simulate_sell,
        SimulationError,
        SimulationResult,
    },
};

async fn simulate_trade_on_request(
    prev_state: SimulationState,
    client: Arc<Provider<Ws>>,
    token: Token,
    block_oracle: BlockOracle,
) -> Result<SimulationEvent, SimulationError> {
    let start = Instant::now();
    let next_block = block_oracle.next.clone();

    // Perform simulation
    let fork_block = block_oracle.next.clone();
    let mut fork_factory = prepare_database(
        client.clone(), 
        fork_block.clone(),
        None
    ).await?;
    
    // TODO: In parallel with subscribed sims
    let result = simulate_token(
        &token,
        &vec![],
        &fork_block,
        &mut fork_factory
    ).await?;
    log::info!("{}", format!("simulate_trade_on_request for token {:?} took {:?}", token.address, start.elapsed()));
    // We need to generate an event, but not saving state (or do we?)
    let state = match generate_state(prev_state, result) {
        // Override launch state, because we don't want to target a random TX
        SimulationState::Launch(state) => { 
            let mut r = SimulationStateChanged::from(state);
            // TODO: This is a shitty workaround, because I eliminated the Unkown state, so if the token is already launched
            // it will always end up in Launch state, but we don't have a valid TX at this point and during conversion unwrap_or_default is used
            // the problem is that, if the TX is default it will become invalid therefore the estimate gas will fail
            r.tx = None;
            SimulationState::Changed(r)
        },
        state => { state},        
    };
    

    let event = SimulationEvent::new(token, next_block, state);
    //println!("event: {:?}", event);
    Ok(event)
}


async fn simulate_estiamte_gas(
    client: Arc<Provider<Ws>>,
    target_block: BlockInfo,
    txs: Vec<Transaction>,
    //state_diff: Option<StateDiff>,
    estimate_txs: Vec<Transaction>
) -> Result<Vec<Transaction>, SimulationError> {
    
    let start = Instant::now();
    // Perform simulation
    let mut fork_factory = prepare_database(
        client.clone(), 
        target_block.clone(),
        None
    ).await?;

    let result = estimage_gas(
        txs,
        estimate_txs,
        &target_block,
        &mut fork_factory
    ).await;
    log::info!("{}", format!("simulate_estiamte_gas took {:?}", start.elapsed()));
    result
}
  

fn generate_state(state: SimulationState, simulation: SimulationResult) -> SimulationState {
    //log::info!("{}", format!("Generate state prev: {:?} result {:?}", state, simulation));
    match &state {
        SimulationState::Closed(_) => {
            if simulation.buy_valid() && simulation.sell_valid() {
                // Launch iminent
                SimulationState::Launch(SimulationStateLaunch::from(simulation))
            } else {
                state
            }
        },
        // SimulationState::Launch is handled in the new block
        SimulationState::Changed(_) => {
            SimulationState::Changed(SimulationStateChanged::from(simulation))
        },       
        _ => { state }
    }
    
}


pub struct SimulatorLego<EventTx> 
where 
    EventTx: MessageTransmitter<Event>,
{    
    pub token_id: Address,
    pub event_tx: EventTx,
    pub simulation_tx: broadcast::Sender<Event>,
    pub requests: mpsc::Receiver<SimulatorRequest>,
    pub block_stream: watch::Receiver<BlockOracle>,
    pub client: Arc<Provider<Ws>>,
    pub sell_check: DashMap<TraderId, Vec<Transaction>>,
    pub token_pool: Arc<DashMap<Address, Token>>,
}

pub struct Simulator<EventTx> 
where 
    EventTx: MessageTransmitter<Event>,
{
    token_id: Address,
    event_tx: EventTx,
    simulation_tx: broadcast::Sender<Event>,
    requests: mpsc::Receiver<SimulatorRequest>,
    block_stream: watch::Receiver<BlockOracle>,
    event_q: VecDeque<Event>,
    client: Arc<Provider<Ws>>,
    sell_check: DashMap<TraderId, Vec<Transaction>>,
    token_pool: Arc<DashMap<Address, Token>>,
    // State members
    state: SimulationState,
}

impl <EventTx> Simulator<EventTx> 
where 
    EventTx: MessageTransmitter<Event>,
{

    pub fn new(lego: SimulatorLego<EventTx>) -> Self {  
        let state = SimulationState::default();
        let sell_check = DashMap::new();
        Self {
            token_id: lego.token_id,
            event_tx: lego.event_tx,
            simulation_tx: lego.simulation_tx,
            requests: lego.requests,
            block_stream: lego.block_stream,
            token_pool: lego.token_pool,
            client: lego.client,
            event_q: VecDeque::with_capacity(10),
            sell_check,
            state,
        }
    }

    pub fn builder() -> SimulatorBuilder<EventTx>  {
        SimulatorBuilder::new()
    }

    pub fn get_token(&self) -> Token {
        *self.token_pool.get(&self.token_id).unwrap()
    }

    pub async fn run(mut self) {
        'simulation: loop {

            tokio::select! {
                Some(request) = self.requests.recv() => {
                    match request {
                        SimulatorRequest::Transaction(value) => {
                            self.event_tx.send(Event::TransactionNew(value.clone()));
                            self.event_q.push_back(Event::TransactionNew(value));
                        },
                        SimulatorRequest::RegisterAntiRug(trader_id, transactions) => {
                            match self.sell_check.entry(trader_id) {
                                mapref::entry::Entry::Occupied(entry) => {
                                    entry.replace_entry(transactions);
                                },
                                mapref::entry::Entry::Vacant(entry) => {
                                    entry.insert(transactions);
                                }
                            }
                        },
                        SimulatorRequest::DeRegisterAntiRug(trader_id) => {
                            match self.sell_check.entry(trader_id) {
                                mapref::entry::Entry::Occupied(entry) => {
                                    entry.remove();
                                },
                                mapref::entry::Entry::Vacant(_) => {
                                    // nothing to do
                                }
                            }
                        }
                        SimulatorRequest::TradeSimulation(response) => {

                            let block_oracle: BlockOracle = (*self.block_stream.borrow()).clone();        
                            //let next_block = block_oracle.next.clone();
                            let client = self.client.clone();
                            let token = self.get_token().clone();
                            let prev_state = self.state.clone();

                            tokio::spawn(async move {
                                let event = simulate_trade_on_request(
                                    prev_state,
                                    client,
                                    token,
                                    block_oracle,
                                ).await;
    
                                response.send(event).await;
                            });                            
                        },                        
                        SimulatorRequest::EstimateGas(block, estimate_txs, response) => {
                            // Get open tx
                            let txs = match &self.state {
                                SimulationState::Launch(launch) => {
                                    vec![launch.tx.clone()]
                                },
                                _ => { vec![] }
                            };
                            let client = self.client.clone();  
                            let block = block.unwrap_or_else(|| { 
                                let block_oracle = self.block_stream.borrow();
                                block_oracle.next.clone()
                             });
                        

                            tokio::spawn(async move {
                                let result = simulate_estiamte_gas(
                                    client,
                                    block,
                                    txs,
                                    estimate_txs
                                ).await;
                                response.send(result).await;
                            });
                            
                        }
                        _ => {}
                    }
                    
                },
                Ok(_block) = self.block_stream.changed() => {
                    let oracle = self.block_stream.borrow();
                    let latest = oracle.latest.clone();

                    self.event_q.push_back(Event::BlockConfirmed(latest));
                }
            }
            
            if self.simulation_tx.receiver_count() == 0 {
                self.event_tx.send(Event::SimulationClosed(self.token_id));
                break 'simulation ;
            }
            
            while let Some(event) = self.event_q.pop_front() {
                match event {
                    Event::TransactionNew(transaction_event) => {      
                        // Update token pool if already found
                        if self.get_token().pool.is_none() {
                            continue;
                        }

                        let event_transaction = transaction_event.tx.clone();
                        let token = self.get_token().clone();
                        let hash = transaction_event.tx.hash.clone();

                        let txs = match &self.state {
                            SimulationState::Launch(launch) => {
                                vec![launch.tx.clone(), event_transaction.clone()]
                            },                          
                            _ => { vec![event_transaction.clone()] }
                        };
                        let start = Instant::now();
                        // Perform simulation
                        let fork_block = transaction_event.oracle.next.clone();
                        let mut fork_factory = match prepare_database(
                            self.client.clone(), 
                            fork_block.clone(),
                            Some(transaction_event.state_diff)
                        ).await {
                            Ok(v) => v,
                            Err(e) => { log::error!("{}", format!("{:?}", e)); continue;}
                        };
                        
                        let mut sell_results = vec![];

                        self.sell_check.iter().for_each(|f| {
                            let (trader_id, user_transactions) = f.pair();

                            let token = token.clone();
                            let txs = txs.clone();
                            let fork_block = fork_block.clone();
                            let fork_factory = fork_factory.clone();
                            let trader_id = trader_id.clone();
                            let test_txs = user_transactions.clone();

                            sell_results.push(
                                tokio::task::spawn(async move {
                                    let r = simulate_sell(
                                        token,
                                        txs,
                                        test_txs,
                                        fork_block,
                                        fork_factory
                                    ).await;
                                    (trader_id, r)
                                })
                            )
                        });
                     
                        let result = simulate_token(
                            &transaction_event.token,
                            &txs,
                            &fork_block,
                            &mut fork_factory
                        );
                        
                        let (result, sell_results) = tokio::join!(result, futures::future::join_all(sell_results));
                        let result = match result {
                            Ok(v) => v,
                            Err(e) => { log::error!("{}", format!("{:?}", e)); continue;}
                        };
                        let sell_results = sell_results
                            .into_iter()
                            .map(|f| f.unwrap())
                            .collect::<Vec<_>>();
                        
                        log::info!("{}", format!("Simulate transaction {:?} took {:?}", hash, start.elapsed()));
                        // TODO: We also need to simulate blacklist token transfer, and based on result and everything we need to find out

                        let new_state = generate_state(self.state, result);
                        // Update states
                        self.state = new_state.clone();
                        let mut events = sell_results
                            .into_iter()
                            .map(|(trader_id, simulation)| Event::SellSimulationEvent(
                                SellSimulationEvent::new(
                                    trader_id,
                                    token.clone(),
                                    fork_block.clone(),
                                    simulation.unwrap(),
                                    new_state.clone()
                                ))
                            )
                            .collect::<Vec<_>>();

                        let event = SimulationEvent::new(
                            token,
                            fork_block,
                            new_state
                        );
                        events.push(Event::SimulationEvent(event));

                        self.event_tx.send_many(events.clone());
                        events
                            .into_iter()
                            .for_each(|e| { self.simulation_tx.send(e); });

                    },         
                    Event::BlockConfirmed(_) => {
                        let oracle = (self.block_stream.borrow()).clone();

                        match &self.state {
                            SimulationState::Launch(launch) => {
                                if oracle.block.transactions.contains(&launch.tx.hash) {
                                    self.state = SimulationState::Changed(SimulationStateChanged::from(launch.clone()))
                                } else {
                                }
                            },
                            _ => {  }
                        };
                        // Update token pool if already found
                        if self.get_token().pool.is_none() {
                            continue;
                        }
                                    
                        let start = Instant::now();
                        let fork_block = oracle.next.clone();
                        let token = self.get_token().clone();
                        let token_address = token.address;

                        let mut fork_factory = match prepare_database(
                            self.client.clone(), 
                            fork_block.clone(),
                            // TODO: Maybe based on block tx-es, get the trace_callMany state diffs? Need some benchmark
                            None
                        ).await {
                            Ok(v) => v,
                            Err(e) => { log::error!("{}", format!("{:?}", e)); continue;}
                        };
                        let txs = vec![];
                        let mut sell_results = vec![];
                        
                        self.sell_check.iter().for_each(|f| {
                            let (trader_id, user_transactions) = f.pair();

                            let token = token.clone();
                            let txs = vec![];
                            let fork_block = fork_block.clone();
                            let fork_factory = fork_factory.clone();
                            let trader_id = trader_id.clone();
                            let test_txs = user_transactions.clone();

                            sell_results.push(
                                tokio::task::spawn(async move {
                                    let r = simulate_sell(
                                        token,
                                        txs,
                                        test_txs,
                                        fork_block,
                                        fork_factory
                                    ).await;
                                    (trader_id, r)
                                })
                            )
                        });
                        let result = simulate_token(
                            &token,
                            &txs,
                            &fork_block,
                            &mut fork_factory
                        );
                        
                        let (result, sell_results) = tokio::join!(result, futures::future::join_all(sell_results));
                        let result = match result {
                            Ok(v) => v,
                            Err(e) => { log::error!("{}", format!("{:?}", e)); continue;}
                        };
                        let sell_results = sell_results
                            .into_iter()
                            .map(|f| f.unwrap())
                            .collect::<Vec<_>>();
                        
                        log::info!("{}", format!("Simulate block {:?} for {:?} took {:?}", fork_block.number, token_address, start.elapsed()));

                        let new_state = generate_state(self.state.clone(), result);

                        let mut events = sell_results
                            .into_iter()
                            .map(|(trader_id, simulation)| Event::BlockSellSimulationEvent(
                                SellSimulationEvent::new(
                                    trader_id,
                                    token.clone(),
                                    fork_block.clone(),
                                    simulation.unwrap(),
                                    new_state.clone()
                                ))
                            )
                            .collect::<Vec<_>>();

                        let event = SimulationEvent::new(
                            token,
                            fork_block,
                            new_state
                        );
                        match &self.state {
                            SimulationState::Changed(_) => {},
                            _ => {
                                events.push(Event::BlockSimulationEvent(event));
                            }
                        }
                        
                        self.event_tx.send_many(events.clone());
                        events
                            .into_iter()
                            .for_each(|e| { self.simulation_tx.send(e); });

                    },
                    _ => {}
                }
            }
        }
    }

}

pub struct SimulatorBuilder<EventTx> 
where 
    EventTx: MessageTransmitter<Event>,
{
    token_id: Option<Address>,
    event_tx: Option<EventTx>,
    simulation_tx: Option<broadcast::Sender<Event>>,
    requests: Option<mpsc::Receiver<SimulatorRequest>>,
    block_stream: Option<watch::Receiver<BlockOracle>>,
    token_pool: Option<Arc<DashMap<Address, Token>>>,
    client: Option<Arc<Provider<Ws>>>,    
}

impl<EventTx> SimulatorBuilder<EventTx>
where
    EventTx: MessageTransmitter<Event>,
{
    
    pub fn new() -> Self {
        Self {
            token_id: None,
            event_tx: None,
            simulation_tx: None,
            requests: None,
            block_stream: None,
            token_pool: None,
            client: None,
        }
    }

    pub fn token_id(self, value: Address) -> Self {
        Self {
            token_id: Some(value),
            ..self
        }
    }

    pub fn token_pool(self, value: Arc<DashMap<Address, Token>>) -> Self {
        Self {
            token_pool: Some(value),
            ..self
        }
    }

    pub fn client(self, value: Arc<Provider<Ws>>) -> Self {
        Self {
            client: Some(value),
            ..self
        }
    }

    pub fn simulation_tx(self, value: broadcast::Sender<Event>) -> Self {
        Self {
            simulation_tx: Some(value),
            ..self
        }
    }

    pub fn block_stream(self, value: watch::Receiver<BlockOracle>) -> Self {
        Self {
            block_stream: Some(value),
            ..self
        }
    }

    pub fn simulation_request(self, value: mpsc::Receiver<SimulatorRequest>) -> Self {
        Self {
            requests: Some(value),
            ..self
        }
    }

    pub fn event_tx(self, value: EventTx) -> Self {
        Self {
            event_tx: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<Simulator<EventTx>, EngineError> {
        let state = SimulationState::default();
        let sell_check = DashMap::new();

        Ok(Simulator {
            token_id: self
                .token_id
                .ok_or(EngineError::BuilderIncomplete("token_id"))?,   
            token_pool: self
                .token_pool
                .ok_or(EngineError::BuilderIncomplete("token_pool"))?,   
            event_tx: self
                .event_tx
                .ok_or(EngineError::BuilderIncomplete("event_tx"))?,   
            simulation_tx: self
                .simulation_tx
                .ok_or(EngineError::BuilderIncomplete("simulation_tx"))?,   
            requests: self
                .requests
                .ok_or(EngineError::BuilderIncomplete("requests"))?,  
            block_stream: self
                .block_stream
                .ok_or(EngineError::BuilderIncomplete("block_stream"))?,   
            client: self
                .client
                .ok_or(EngineError::BuilderIncomplete("client"))?,   
            event_q: VecDeque::with_capacity(10),
            sell_check,
            state,
        })
    }
}