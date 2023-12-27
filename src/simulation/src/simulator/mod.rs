use crate::{
    dex::{Dex},
    token::Token,
    event::{Event, MessageTransmitter},
    utils::{
        create_websocket_client,
        state_diff::{
            get_from_txs,
            update_pairs_for_tokens,
            extract_tokens,
        }
    },
    types::TraderId,
    stream::{BlockOracle, BlockInfo},
};
use hashbrown::{
    HashMap,
};
use dashmap::{DashMap, mapref};
use std::{
    sync::Arc,
};
use tokio::{sync::{mpsc, watch, broadcast}};
use tokio;
use ethers::prelude::{Address, BlockNumber, Transaction, U256};

pub mod error;
mod simulator;
use simulator::{
    Simulator,
};

pub mod event;
pub mod simulation;

 #[derive(Debug)]
pub enum SimulatorRequest
{
    Transaction(event::TransactionNew),
    TradeSimulation(mpsc::Sender<Result<event::SimulationEvent, simulation::SimulationError>>),
    EstimateGas(Option<BlockInfo>, Vec<Transaction>, mpsc::Sender<Result<Vec<Transaction>, simulation::SimulationError>>),
    RegisterAntiRug(TraderId, Vec<Transaction>),
    DeRegisterAntiRug(TraderId),
    MEVProfitability,
    BuyersGas,
}


#[derive(Debug)]
pub struct SimulatorHandle {
    pub simulation_rx: broadcast::Receiver<Event>,
    pub simulation_request: mpsc::Sender<SimulatorRequest>,
}

impl SimulatorHandle {

    pub fn new(
        simulation_rx: broadcast::Receiver<Event>,
        simulation_request: mpsc::Sender<SimulatorRequest>,
    ) -> Self {
        Self {
            simulation_rx,
            simulation_request
        }
    }


}

#[derive(Debug)]
pub enum Command {
    Terminate,
    AddToken (
        Address,
        mpsc::Sender<SimulatorHandle>
    ),

}

#[derive(Debug)]
struct SimulationMap (
    broadcast::Sender<Event>,
    mpsc::Sender<SimulatorRequest>
);

pub struct SimulatorEngineLego<EventTx> 
where 
    EventTx: MessageTransmitter<Event> + Send,    
{    
    pub dexes: Vec<Dex>,
    pub command_rx: mpsc::Receiver<Command>,
    pub transaction_rx: mpsc::Receiver<Transaction>,
    pub event_tx: EventTx,
    pub block_stream: watch::Receiver<BlockOracle>,
    pub token_pool: Arc<DashMap<Address, Token>>,
}

pub struct SimulatorEngine<EventTx>
where
    EventTx: MessageTransmitter<Event> + Clone,
{
    dexes: Vec<Dex>,
    command_rx: mpsc::Receiver<Command>,
    transaction_rx: mpsc::Receiver<Transaction>,
    event_tx: EventTx,
    token_pool: Arc<DashMap<Address, Token>>,
    block_stream: watch::Receiver<BlockOracle>,
    /// Token - Simulator map TODO: replace Address with Token
    simulators: Arc<DashMap<Address, SimulationMap>>,
}

impl<EventTx> SimulatorEngine<EventTx>
where
    EventTx: MessageTransmitter<Event> + Send + Clone + 'static,
{

    pub fn new(lego: SimulatorEngineLego<EventTx>) -> Self {
        let simulators = DashMap::new();
        let simulators = Arc::new(simulators);
        Self {
            dexes: lego.dexes,
            command_rx: lego.command_rx,
            transaction_rx: lego.transaction_rx,
            event_tx: lego.event_tx,
            block_stream: lego.block_stream,
            token_pool: lego.token_pool,
            simulators
        }
    }

    pub fn builder() -> SimulatorEngineBuilder<EventTx> {
        SimulatorEngineBuilder::new()
    }
    /*
        TODO: the following is exactly the same pattern (Actor pattern) but fixing some issues what I have now!!!
        https://ryhl.io/blog/actors-with-tokio/
    */
    pub async fn run(mut self) {
        loop {              
            
            tokio::select! {
                // If new_tx channel is cloed, break
                tx = self.transaction_rx.recv() => {
                    if let Some(mut tx) = tx {                            
                        let oracle: BlockOracle = (*self.block_stream.borrow()).clone();

                        if tx.max_fee_per_gas.unwrap_or(U256::zero()) < oracle.next.base_fee {
                            // TODO: cache TX-s where basefee not enough
                            continue;
                        }
                        // recover from field from vrs (ECDSA)
                        // TODO: expensive operation, can avoid by modding rpc to share `from` field
                        if let Ok(from) = tx.recover_from() {
                            tx.from = from;
                        } else {                            
                            continue;
                        }

                        let client = create_websocket_client().await.unwrap();
                        
                        let mut tx_without_gas = tx.clone();
                        tx_without_gas.max_fee_per_gas = None;
                        tx_without_gas.max_priority_fee_per_gas = None;

                        let state_diffs = match get_from_txs(&client, &vec![tx_without_gas], BlockNumber::Number(oracle.latest.number)).await {
                            Some(v) => v,
                            None => { continue; }
                        };
                        //println!("New transaction: {:?} @ {:?}", tx.hash, block.latest.number);
                        let mut dexes = HashMap::new();
                        for dex in self.dexes.clone() {
                            dexes.insert(dex.address, dex);
                        }

                        let no_pool_tokens = self.token_pool.iter().filter_map(|p| { 
                            if p.value().pool.is_none() {
                                Some((*p.value()).clone())
                            } else {
                                None
                            }
                        }).collect::<Vec<Token>>();
                        //log::info!( "{}", format!("No pool tokens: {:?}", no_pool_tokens));

                        match update_pairs_for_tokens(&state_diffs, &no_pool_tokens, &dexes) {
                            Some(pools) => {
                                for p in pools.into_iter() {
                                    self.token_pool
                                    .alter(&p.0, |_, mut value| {
                                        value.pool = Some(p.1);

                                        self.event_tx.send(Event::PairUpdatedEvent(value.clone()));

                                        value
                                    })
                                }
                            },
                            None => {}
                        }

                        // If state diff touch any of the watched tokens record it
                        let touched_tokens = match extract_tokens(&state_diffs, &self.token_pool) {
                            Some(v) => v,
                            None => { continue; }
                        };
                        
                        for token in touched_tokens {                                    
                            //println!("Touched tokens: {:?} has pool? {:?}", token.address, token.pool.is_some());
                            //println!("sims: {:?}", self.simulators);
                            if token.pool.is_none() {
                                continue;
                            }
                            //log::info!( "{}", format!("Token: {:?} pair: {:?}", token.address, token.pool.unwrap().address));
			                let m = match self.simulators.get(&token.address) {
                                Some(v) => { v },
                                None => { continue; }
                            };
                            let sim_sender = m.1.clone();        

                            sim_sender.send(SimulatorRequest::Transaction(event::TransactionNew {
                                token,
                                oracle: oracle.clone(),
                                tx: tx.clone(),
                                state_diff: state_diffs.clone()
                            })).await;
                        }
                            
                    } else {
                        println!("txpool dropped!");
                        break;
                    }
                },
                command = self.command_rx.recv() => {
                    if let Some(command) = command {
                        match command {
                            Command::AddToken(token, respond_to) => {
                                self.add_token(token, respond_to).await;
                                // TODO: Error handling
                                println!("Token {:?} added", token);
                            }
                            Command::Terminate => {
                                // TODO: Terminate bot
                            }
                        }
                    } else {
                        println!("command channel dropped!");
                        break;
                    }
                }
            }
        }
    }

    async fn add_token(&mut self, token_address: Address, respond_to: mpsc::Sender<SimulatorHandle>)  {
        let client = create_websocket_client().await.unwrap();
        let token = match self.token_pool.entry(token_address) {
            mapref::entry::Entry::Occupied(entry) => (*entry.get()).clone(),
            mapref::entry::Entry::Vacant(entry) => {
                let t = Token::create(token_address, &self.dexes, client.clone()).await;
                entry.insert(t.clone());
                t
            }
        };
        //let token = Token::create(token_address, &self.dexes, client.clone()).await;

        let result = match self.simulators.entry(token_address) {
            mapref::entry::Entry::Occupied(entry) => {
                let m = entry.get();
                SimulatorHandle::new(m.0.subscribe(), m.1.clone())
            },
            mapref::entry::Entry::Vacant(entry) => {
                // Buffer for simulation requests can be sent from traders
                let (simulation_request, simulation_rx) = mpsc::channel(10);
                let (simulation_tx, rx) = broadcast::channel(10);

                // Store broadcast TX and the request RX
                entry.insert(SimulationMap { 0:simulation_tx.clone(), 1: simulation_request.clone() });
                // TODO: Better error handling!
                //let simulator = self.simulator_bp
                //let client = create_websocket_client().await.unwrap();
                let simulator = Simulator::builder()
                    .event_tx(self.event_tx.clone())
                    .token_id(token_address.clone())
                    .simulation_tx(simulation_tx)
                    .simulation_request(simulation_rx)
                    .token_pool(self.token_pool.clone())
                    .block_stream(self.block_stream.clone())
                    .client(client)
                    .build()
                    .expect("failed to build & initialise Simulator");

                // Spawn the new simulator task
                /*
                let handle = tokio::spawn(async move {
                    simulator.run().await;
                });
                 */
                
                let handle = tokio::spawn(simulator.run());
                let map = self.simulators.clone();
                let token = token_address.clone();

                tokio::spawn(async move {
                    handle.await;
                    map.remove(&token);
                    let simulators_left = map.len();
                    log::info!(
                        "{}", format!("Token simulator {:?} removed from map, {:?} left", token, simulators_left)
                    );
                });
                // Trader can always send requests to simulation_request and will receive new sims on rx
                SimulatorHandle::new(rx, simulation_request)
                
            }
        };
        respond_to.try_send(result);
        
    }
}

pub struct SimulatorEngineBuilder<EventTx>
where
    EventTx: MessageTransmitter<Event>,
{
    dexes: Option<Vec<Dex>>,
    command_rx: Option<mpsc::Receiver<Command>>,
    transaction_rx: Option<mpsc::Receiver<Transaction>>,
    event_tx: Option<EventTx>,
    block_stream: Option<watch::Receiver<BlockOracle>>,
    token_pool: Option<Arc<DashMap<Address, Token>>>,
}

impl<EventTx> SimulatorEngineBuilder<EventTx>
where
    EventTx: MessageTransmitter<Event> + Clone,
{
    
    pub fn new() -> Self {
        Self {
            dexes: None,
            command_rx: None,
            transaction_rx: None,
            event_tx: None,
            block_stream: None,
            token_pool: None,
        }
    }

    pub fn command_rx(self, value: mpsc::Receiver<Command>) -> Self {
        Self {
            command_rx: Some(value),
            ..self
        }
    }

    pub fn token_pool(self, value: Arc<DashMap<Address, Token>>) -> Self {
        Self {
            token_pool: Some(value),
            ..self
        }
    }

    pub fn dex_list(self, dexes: Vec<Dex>) -> Self {
        Self {
            dexes: Some(dexes),
            ..self
        }
    }

    pub fn transaction_rx(self, value: mpsc::Receiver<Transaction>) -> Self {
        Self {
            transaction_rx: Some(value),
            ..self
        }
    }
    
    pub fn block_stream(self, value: watch::Receiver<BlockOracle>) -> Self {
        Self {
            block_stream: Some(value),
            ..self
        }
    }

    pub fn event_tx(self, value: EventTx) -> Self {
        Self {
            event_tx: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<SimulatorEngine<EventTx>, error::EngineError> {
        let simulators = DashMap::new();
        let simulators = Arc::new(simulators);
        Ok(SimulatorEngine {
            dexes: self
                .dexes
                .ok_or(error::EngineError::BuilderIncomplete("dexes"))?,
                token_pool: self
                .token_pool
                .ok_or(error::EngineError::BuilderIncomplete("token_pool"))?,
            command_rx: self
                .command_rx
                .ok_or(error::EngineError::BuilderIncomplete("command_rx"))?,
            transaction_rx: self
                .transaction_rx
                .ok_or(error::EngineError::BuilderIncomplete("transaction_rx"))?,
            event_tx: self
                .event_tx
                .ok_or(error::EngineError::BuilderIncomplete("event_tx"))?,  
            block_stream: self
                .block_stream
                .ok_or(error::EngineError::BuilderIncomplete("block_stream"))?,          
            simulators
        })
    }
}