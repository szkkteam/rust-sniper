use eyre::Result;
use simulation::prelude::*;
use tokio::sync::mpsc;
use tokio::time::{Duration};
use tokio;
use ethers::prelude::{H160};
use std::{
    str::FromStr,
    sync::Arc, 
    time::{SystemTime, UNIX_EPOCH}
};
use dashmap::{DashMap};
use parking_lot::Mutex;

use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};

use futures::{StreamExt};

use rdkafka::config::{ClientConfig};
use rdkafka::consumer::stream_consumer::StreamConsumer;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::consumer::{Consumer};
use rdkafka::message::{Message, OwnedHeaders, Header};
use rdkafka::util::get_rdkafka_version;

const TOPIC_CREATE_PROFILE: &str = "eth-bot.create";
const TOPIC_CREATE_PROFILE_REPLY: &str = "eth-bot.create.reply";
const TOPIC_UPDATE_ANTIRUG: &str = "eth-bot.update.antirug";
const TOPIC_FORCE_EXIT: &str = "eth-bot.force-exit";
const TOPIC_TAKE_PROFIT: &str = "eth-bot.take-profit";
const TOPIC_CANCEL_TRADE: &str = "eth-bot.cancel";

const TOPIC_EVENT: &str = "eth-bot.event";


#[tokio::main]
async fn main() -> Result<()> {    
    //console_subscriber::init();
    // setup logger configs
    let mut colors = ColoredLevelConfig::new();
    colors.trace = Color::Cyan;
    colors.debug = Color::Magenta;
    colors.info = Color::Green;
    colors.warn = Color::Red;
    colors.error = Color::BrightRed;

    // setup logging both to stdout and file
    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{}[{}] {}",
                chrono::Local::now().format("[%H:%M:%S.%f]"),
                colors.color(record.level()),
                message
            ))
        })
        .chain(std::io::stdout())
        .chain(fern::log_file("logs/bot.log")?)
        // hide all logs for everything other than bot
        .level(log::LevelFilter::Info)
        .level_for("eth-bot", log::LevelFilter::Info)
        .apply()?;

    
    log::info!("{}", format!("Booting up ...").bold().cyan().on_black());

    // Build dex list
    let dex = simulation::dex::Dex::new(
        H160::from_str("0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f").unwrap(),
        simulation::dex::PoolVariant::UniswapV2
    );
    let mut dexes = vec![];
    dexes.push(dex);

    // Communication channles to the engine
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let event_tx = simulation::event::EventTx::new(event_tx);

    // Channel to send commands to the simulator engine
    let (simulator_command_tx, simulator_command_rx) = mpsc::channel(20);
    // Channel to send commands to the trader engine
    let (trader_command_tx, trader_command_rx) = mpsc::channel(20);
    // Channel to send orders to the executor
    let (order_sender, order_receiver) = mpsc::channel(20);

    // Token pool
    let token_pool = Arc::new(DashMap::new());

    // Streams
    let block_stream = stream_block_notification().await.unwrap();

    let repository = repository::redis::RedisRepository::builder()
        .pool(repository::redis::RedisRepository::setup_redis_connection("redis://127.0.0.1:6379"))
        .build()
        .expect("Invalid redis port");

        log::info!("{}", format!("Redis repository - Ready").bold().cyan().on_black());

    let repository = std::sync::Arc::new(Mutex::new(repository));

    let simulator = SimulatorEngine::builder()
        .block_stream(block_stream.clone())
        .command_rx(simulator_command_rx)
        .dex_list(dexes)
        .token_pool(token_pool.clone())
        .event_tx(event_tx.clone())
        .transaction_rx(stream_pending_transaction().await.unwrap())
        .build()
        .expect("Simulator engine cannot be built");

        log::info!("{}", format!("Simulator - Ready").bold().cyan().on_black());

    let executor = Executor::builder()
        .block_stream(block_stream.clone())
        .event_tx(event_tx.clone())
        .order_receiver(order_receiver)
        .build()
        .expect("Executor cannot be built");

        log::info!("{}", format!("Executor - Ready").bold().cyan().on_black());

    let trader = TraderEngine::builder()
        .command_rx(trader_command_rx)
        .event_tx(event_tx.clone())
        .executor_tx(order_sender)
        .token_pool(token_pool.clone())
        .repository(repository)
        .build()
        .expect("Trader engine cannot be built");

    log::info!("{}", format!("Trader - Ready").bold().cyan().on_black());

    log::info!("{}", format!("Eth-Bot ready!").bold().cyan().on_black());

    let (version_n, version_s) = get_rdkafka_version();
    log::info!("rd_kafka_version: 0x{:08x}, {}", version_n, version_s);
    
    let brokers = "localhost:9092";
    let topics= vec![TOPIC_CREATE_PROFILE, TOPIC_UPDATE_ANTIRUG, TOPIC_CANCEL_TRADE, TOPIC_TAKE_PROFIT, TOPIC_FORCE_EXIT];
    let group_id = "test.group";

    #[cfg(feature = "dry")] 
    {
        log::warn!("{}", format!("Bot running in DRY mode!").bold().cyan().on_black()); 
    }

    let consumer: StreamConsumer = ClientConfig::new()
        .set("group.id", group_id)
        .set("bootstrap.servers", brokers)
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", "6000")
        .set("enable.auto.commit", "true")
        //.set("statistics.interval.ms", "30000")
        //.set("auto.offset.reset", "smallest")
        .create()
        .expect("Consumer creation failed");

    consumer
        .subscribe(&topics.to_vec())
        .expect("Can't subscribe to specified topics");

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("message.timeout.ms", "5000")
        .create()
        .expect("Producer creation error");

    let mut threads = vec![];        

    // Spawn task to send events to kafka
    let events_producer = producer.clone();
    threads.push(tokio::spawn(async move { listen_to_engine_events(event_rx, events_producer).await; }));

    // Start executor thread
    threads.push(tokio::spawn(executor.run()));
    // Start trader thread
    threads.push(tokio::spawn(trader.run()));
    // Start simulator thread
    threads.push(tokio::spawn(simulator.run()));
    
    while let Some(msg) = consumer.stream().next().await {
        let borrowed_message = match msg {
            Err(e) => {
                log::warn!("Kafka error: {}", e);
                continue;
            },
            Ok(m) => m
        };
        let owned_message = borrowed_message.detach();
        let simulator_command_tx= simulator_command_tx.clone();
        let trader_command_tx = trader_command_tx.clone();
        let producer = producer.clone();
        //let _event_tx = event_tx.clone();
        // Spawn a task to process the msg
        tokio::spawn(async move {
            let producer = producer.clone();
            let payload = match owned_message.payload_view::<str>() {
                None => "",
                Some(Ok(s)) => s,
                Some(Err(e)) => {
                    log::warn!("Error while deserializing message payload: {:?}", e);
                    ""
                }
            };
            match owned_message.topic() {
                TOPIC_CREATE_PROFILE => {               
                    let profile: Profile = serde_json::from_str(payload).unwrap();
                    
                    let (tx, mut response) = mpsc::channel(1);
                    let token = profile.token;
                    
                    match simulator_command_tx.send(SimulatorCommand::AddToken(token.clone(), tx)).await {
                        Ok(_) => 
                            match response.recv().await {
                                Some(handle) => {
                                    let (tx, mut respone) = mpsc::channel(1);
                                    match trader_command_tx.send(TraderCommand::CreateTrader(
                                        handle, 
                                        profile,
                                        tx
                                    )).await {
                                        Ok(_) =>{
                                            match respone.recv().await {
                                                Some(trader_id) => {
                                                    let payload = serde_json::to_string(&trader_id).unwrap();
                                                    let headers = create_reply_header();
                                                    let record = FutureRecord::to(TOPIC_CREATE_PROFILE_REPLY)
                                                        .key("")
                                                        .headers(headers)
                                                        .payload(&payload);
                                                    
                                                    let produce_future = producer.send(record,
                                                        Duration::from_secs(0),
                                                    );
                                                    produce_future.await;

                                                    log::info!( "{}", format!("Trader created for {:?}", token));
                                                },
                                                None => {
                                                    log::error!( "{}", format!("Cannot create trader"));
                                                }
                                            }
                                            
                                        },
                                        Err(e) => {
                                            log::error!( "{}", format!("Cannot send command to trader: {:?}", e));
                                        }
                                    }
                                },
                                None => {
                                    log::error!("Cannot add token to simulator");
                                }
                            },
                        Err(e) => {
                            log::error!( "{}", format!("Cannot send command to simulator: {:?}", e));
                        }
                    };

                },
                TOPIC_UPDATE_ANTIRUG => {
                    let anti_rug: interface::UpdateAntiRugInterface = serde_json::from_str(payload).unwrap();
                    match trader_command_tx.send(TraderCommand::UpdateTraderAntiRug(
                        anti_rug.clone()
                    )).await {
                        Ok(_) =>{
                            log::info!( "{}", format!("Trader Anti-Rug updated to {:?}", anti_rug));
                        },
                        Err(e) => {
                            log::error!( "{}", format!("Failed to update trader Anti-Rug: {:?}", e));
                        }
                    };

                },
                TOPIC_FORCE_EXIT => {
                    let request: interface::ForceExitPositionInterface = serde_json::from_str(payload).unwrap();
		    println!("Force exit data: {:?}", request);
                    match trader_command_tx.send(TraderCommand::ForceExitPosition(
                        request.clone()
                    )).await {
                        Ok(_) =>{
                            log::info!( "{}", format!("Trader {:?} force exited position", request.trader_id));
                        },
                        Err(e) => {
                            log::error!( "{}", format!("Trader {:?} failed to force exit position due to {:?}", request.trader_id, e));
                        }
                    };

                },
                TOPIC_TAKE_PROFIT => {
                    let request: interface::TakeProfitInterface = serde_json::from_str(payload).unwrap();
                    match trader_command_tx.send(TraderCommand::TakeProfit(
                        request.clone()
                    )).await {
                        Ok(_) =>{
                            log::info!( "{}", format!("Trader {:?} took profit", request.trader_id));
                        },
                        Err(e) => {
                            log::error!( "{}", format!("Trader {:?} failed to take profit due to {:?}", request.trader_id, e));
                        }
                    };
                },
                TOPIC_CANCEL_TRADE => {
                    let trader: interface::TerminateTraderInterface = serde_json::from_str(payload).unwrap();
                    let trader_id = trader.trader_id;
                    match trader_command_tx.send(TraderCommand::TerminateTrader(
                        trader_id.clone()
                    )).await {
                        Ok(_) =>{
                            log::info!( "{}", format!("Trader {:?} terminated", trader_id));
                        },
                        Err(e) => {
                            log::error!( "{}", format!("Failed to terminate trader: {:?}", e));
                        }
                    };

                },
                &_ => {
                    log::warn!("Unknown topic!")
                }
            }
        });


    }
   
    Ok(())
}

fn current_timestamp() -> String {
    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("time going backwards");
    since_the_epoch.as_millis().to_string()
}

fn create_default_header(event: &Event) -> OwnedHeaders {
    let event_str = event.to_string();

    let headers = OwnedHeaders::new()
        .insert(Header { key: "type", value: Some(&event_str) })
        .insert(Header { key: "timestamp", value: Some(&current_timestamp()) })
        .insert(Header { key: "version", value: Some("v1") })
        .insert(Header { key: "bot-version", value: Some("eth-bot") });

    headers

}

fn create_reply_header() -> OwnedHeaders {
    let headers = OwnedHeaders::new()
        .insert(Header { key: "timestamp", value: Some(&current_timestamp()) })
        .insert(Header { key: "version", value: Some("v1") })
        .insert(Header { key: "bot-version", value: Some("eth-bot") });

    headers

}

async fn listen_to_engine_events(mut event_rx: mpsc::UnboundedReceiver<simulation::event::Event>, producer: FutureProducer) {
    loop {
        if let Some(event) = event_rx.recv().await {
            let (headers, payload) = match &event {
                Event::SimulationEvent(event_data) => {
                    let token = event_data.token.clone();
                    let payload = serde_json::to_string(event_data).unwrap();

                    // Event specific
                    let token_str = token.address.to_string();
                    //let key = event.to_string().clone();

                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "token", value: Some(&token_str) });
                    
                    (headers, payload)                    
                },
                Event::BlockSimulationEvent(event_data) => {
                    let token = event_data.token.clone();
                    let payload = serde_json::to_string(event_data).unwrap();

                    // Event specific
                    let token_str = token.address.to_string();
                    //let key = event.to_string().clone();

                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "token", value: Some(&token_str) });
                    
                    (headers, payload)                    
                },
                Event::BlockConfirmed(block) => {
                    let payload = serde_json::to_string(block).unwrap();
                    let key = block.number.to_string().clone();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "block", value: Some(&key.clone()) });

                    (headers, payload)
                },
                Event::TransactionNew(tx) => {
                    let payload = serde_json::to_string(tx).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "hash", value: Some(&tx.tx.hash.to_string().clone()) });

                    (headers, payload)
                },
                Event::TransactionEvent(tx) => {
                    let payload = serde_json::to_string(tx).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&tx.transaction_id.to_string()) });

                    (headers, payload)
                },
                Event::OrderNew(order) => {
                    let payload = serde_json::to_string(order).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&order.order_id.to_string()) });

                    (headers, payload)
                },
                Event::PairUpdatedEvent(token) => {
                    let payload = serde_json::to_string(token).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&token.address.to_string()) });

                    (headers, payload)
                },
                Event::SellSimulationEvent(sell_event) =>{
                    let payload = serde_json::to_string(sell_event).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&sell_event.trader_id.to_string()) });

                    (headers, payload)
                },
                Event::BlockSellSimulationEvent(sell_event) =>{
                    let payload = serde_json::to_string(sell_event).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&sell_event.trader_id.to_string()) });

                    (headers, payload)
                },
                Event::PositionNew(position) => {
                    let payload = serde_json::to_string(position).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&position.position_id.to_string()) });

                    (headers, payload)
                },
                Event::PositionUpdated(position) => {
                    let payload = serde_json::to_string(position).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&position.position_id.to_string()) });

                    (headers, payload)
                },
                Event::PositionExited(position) => {
                    let payload = serde_json::to_string(position).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&position.position_id.to_string()) });

                    (headers, payload)
                },
                Event::TraderStatisticsUpdated(statistics) => {
                    let payload = serde_json::to_string(statistics).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&statistics.trader_id.to_string()) });

                    (headers, payload)
                },
                Event::TraderCreated(trader) => {
                    let payload = serde_json::to_string(trader).unwrap();

                    // Event specific
                    let headers = 
                        create_default_header(&event)
                        .insert(Header { key: "id", value: Some(&trader.trader_id.to_string()) });

                    (headers, payload)
                },
                
                
                _ => {
                    continue;
                }
                 
            };
            let record = FutureRecord::to(TOPIC_EVENT)
                .key("")
                .headers(headers)
                .payload(&payload);
            
            let produce_future = producer.send(record,
                Duration::from_secs(0),
            );
            match produce_future.await {
                Ok(delivery) =>  {/*println!("Sent: {:?}", delivery) */},
                Err((e, _)) => println!("Error: {:?}", e),
            }
            
        }
    
    }
}