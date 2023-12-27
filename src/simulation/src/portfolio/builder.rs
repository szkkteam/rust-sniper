
use crate::{
    dex::{Pool, PoolVariant, PoolWethPair},
    utils::{
        encode_packed::{encode_packed, PackedToken},
        constants::get_weth_address,
        create_websocket_client,
    },
    portfolio::{
        profile::{
            Profile,
            TransactionType,
            OrderSize,
            WalletType
        }
    },
    abi::*,
    simulator::event::TransactionLimits
};
use super::TransactionSigner;
use ethers::prelude::{*};
use std::sync::Arc;

fn generate_backrun_input_data(
    pair: Pool,
    tx_limits: TransactionLimits,
    profile: Profile
) -> Vec<u8> {
    match &profile.order.order_size {
        OrderSize::Limit(order_limit) => {
            match &profile.order.wallet_type {
                WalletType::BotWallets { num_wallets } => {                    
                    let target_amount = tx_limits.max_sell_amount.unwrap_or(order_limit.out_amount);
                    let target_amount = if order_limit.out_amount < target_amount { order_limit.out_amount } else { target_amount };

                    prepare_payload_buy_limit_bot_wallets_v2(
                        pair,
                        target_amount.as_u128().into(),
                        order_limit.max_amount_in.clone().as_u128().into(),
                        num_wallets.clone()
                    )
                },
                WalletType::UserWallets { wallets } => {
                    panic!("User wallets are not supported yet!")
                }
            }
        },
        OrderSize::Exact(_) => {
            panic!("Exact order is not supported yet!")
        }
        OrderSize::Strict(_) => {
            panic!("Strict order is not supported yet!")

        }
    }
}

pub async fn generate_backrun_transactions(
    pair: Pool,
    tx_limits: TransactionLimits,
    profile: &Profile
) -> Vec<TransactionSigner> {
    let mut txs = vec![];
    match &profile.order.transaction_type {
        TransactionType::InuEth => {
            panic!("Inu.eth style transactions are not supported yet!")
        },
        _ => {
            let client = create_websocket_client().await.unwrap();
            let signer = profile.private_key.parse::<LocalWallet>().unwrap();            
            let nonce = get_nonce(&client, signer.address()).await.unwrap();
            let encoded_data = generate_backrun_input_data(pair, tx_limits, profile.clone());
            
            let mut transaction = Transaction::default();

            transaction.to = Some(profile.contract_address.clone());
            transaction.from = signer.address().into();
            transaction.input = encoded_data.into();
            transaction.gas = U256::from(900000);
            transaction.nonce = nonce;
            transaction.value = U256::zero();
            
            txs.push(TransactionSigner::new(transaction, signer));
        }
    }

    txs
}

pub async fn get_wallets_balances(
    client: Arc<Provider<Ws>>,
    token_address: Address,
    contract_address: Address,
) -> Vec<U256> {
    
    let contract = SniperController::new(contract_address, client);

    let balances = contract.get_wallet_balances_88429(token_address).call().await.unwrap();
    balances
}


pub async fn get_wallets(
    client: Arc<Provider<Ws>>,
    contract_address: Address,
) -> Vec<Address> {
    let contract = SniperController::new(contract_address, client);

    let wallets = contract.get_wallets_10844().call().await.unwrap();
    wallets
}

pub async fn get_nonce(
    client: &Arc<Provider<Ws>>,
    signer_address: Address,
    ) -> Result<U256, ProviderError> {
        client.get_transaction_count(signer_address, None).await        
}

pub async fn generate_frontrun_transactions(
    pair: Pool,
    profile: &Profile
) -> Vec<TransactionSigner> {
 
    let mut txs = vec![];
    
    #[cfg(feature = "dry")] 
    {
        let wallets_with_balances = vec![0];
        let dummy_txs = generate_backrun_transactions(pair, TransactionLimits { max_buy_amount: None, max_sell_amount: None }, &profile).await;
        txs.extend(dummy_txs);     

        let client = create_websocket_client().await.unwrap();
        let signer = profile.private_key.parse::<LocalWallet>().unwrap();            
        let encoded_data = prepare_payload_sell_weth_v2(pair, wallets_with_balances);
        let nonce = get_nonce(&client, signer.address()).await.unwrap();

        let mut transaction = Transaction::default();

        transaction.to = Some(profile.contract_address.clone());
        transaction.from = signer.address().into();
        transaction.nonce = nonce;
        transaction.input = encoded_data.into();
        transaction.gas = U256::from(900000);
        transaction.value = U256::zero();
        
        txs.push(TransactionSigner::new(transaction, signer));
        txs       
    }
    #[cfg(not(feature = "dry"))] 
    {
        let client = create_websocket_client().await.unwrap();
        let signer = profile.private_key.parse::<LocalWallet>().unwrap();    
        let nonce = get_nonce(&client, signer.address()).await.unwrap();

        let balances = get_wallets_balances(client, profile.token, profile.contract_address).await;
        println!("Wallet balances: {:?}", balances);
        let wallets_with_balances = balances.
            iter()
            .enumerate()
            .filter(|(_, &r)| r > U256::from(0))
            .map(|(index, _)| u8::try_from(index).unwrap_or_default())
            .collect::<Vec<_>>();

        println!("wallets_with_balances: {:?}", wallets_with_balances);
        let encoded_data = prepare_payload_sell_weth_v2(pair, wallets_with_balances);
        println!("encoded_data: {:?}", encoded_data);
        let mut transaction = Transaction::default();

        transaction.to = Some(profile.contract_address.clone());
        transaction.from = signer.address().into();
        transaction.nonce = nonce;
        transaction.input = encoded_data.into();
        transaction.gas = U256::from(900000);
        transaction.value = U256::zero();
        
        txs.push(TransactionSigner::new(transaction, signer));
        println!("txs: {:?}", txs);
        txs
    }
    
}

pub async fn generate_take_profit_transactions(
    pair: Pool,
    profile: &Profile,
    percentage: u8,
) -> Vec<TransactionSigner> {
    let mut txs = vec![];

    #[cfg(not(feature = "dry"))] 
    {
        let client = create_websocket_client().await.unwrap();
        let signer = profile.private_key.parse::<LocalWallet>().unwrap();   
        let sender = signer.address(); 
        let mut nonce = get_nonce(&client, sender).await.unwrap();
        let balances = get_wallets_balances(client, profile.token, profile.contract_address).await;

        println!("Wallet balances: {:?}", balances);

        let mut total_balances = U256::zero();
        balances.iter().for_each(|balance| { total_balances += *balance });
        println!("total_balances: {:?}", total_balances);

        let mut wallet_number = 0;
        let mut to_sell = U256::from(percentage).checked_mul(total_balances).unwrap_or_default().checked_div(U256::from(100)).unwrap_or_default();
        println!("to_sell: {:?}", to_sell);

        while to_sell > U256::zero(){
            let wallet_balance = balances[wallet_number];
            let wallet_sell = if wallet_balance > to_sell { to_sell } else { wallet_balance };
            println!("wallet_sell: {:?}", wallet_sell);

            let encoded_data = prepare_payload_take_profit_v2(
                pair,
                wallet_sell.as_u128().into(),
                (wallet_number + 1).try_into().unwrap()
            );
            println!("encoded_data: {:?}", encoded_data);
            let mut transaction = Transaction::default();

            transaction.to = Some(profile.contract_address.clone());
            transaction.from = sender.clone();
            transaction.nonce = nonce;
            transaction.input = encoded_data.into();
            transaction.gas = U256::from(900000);
            transaction.value = U256::zero();
            
            txs.push(TransactionSigner::new(transaction, signer.clone()));
            
            nonce += U256::from(1);
            to_sell -= wallet_sell;
            wallet_number += 1;
        }
    }
    println!("txs: {:?}", txs);
    txs
}

pub async fn generate_test_frontrun_transactions(
    pair: Pool,
    profile: &Profile
) -> Vec<Transaction> 
{   
    generate_frontrun_transactions(pair, profile).await
        .into_iter()
        .map(|tx| tx.transaction)
        .collect::<Vec<_>>()
}


pub fn prepare_payload_buy_limit_bot_wallets_v2(
    //&self,
    pair: Pool,
    amount_out: U128,
    amount_in_max: U128,
    num_wallets: u8
) -> Vec<u8> {

    if pair.is_weth_pair() {
        let (token0, _) = if pair.token_0 < pair.token_1 { (pair.token_0, pair.token_1) } else { ( pair.token_1, pair.token_0 ) };
        let flip = if token0 == get_weth_address() { 0 } else { 1 };
     
        let (payload, _) = match pair.pool_variant {
            PoolVariant::UniswapV2 => encode_packed(&[
                PackedToken::Byte(1), // BuyWethBotWalletsV2
                PackedToken::Byte(num_wallets),
                PackedToken::Address(pair.address),
                PackedToken::Half(amount_out),
                PackedToken::Half(amount_in_max),
                PackedToken::Byte(flip),
            ])
        };
        payload
    } else {
        let payload = Vec::new();
        payload
    }
    
}

pub fn prepare_payload_take_profit_v2(
    //&self,
    pair: Pool,
    amount_in: U128,
    wallet: u8
) -> Vec<u8> {

    if pair.is_weth_pair() {
        let (token0, _) = if pair.token_0 < pair.token_1 { (pair.token_0, pair.token_1) } else { ( pair.token_1, pair.token_0 ) };
        let input_token = if pair.token_0 == get_weth_address() { pair.token_1 } else { pair.token_0};
        let flip = if token0 == get_weth_address() { 0 } else { 1 };
     
        let (payload, _) = match pair.pool_variant {
            PoolVariant::UniswapV2 => encode_packed(&[
                PackedToken::Byte(3), // TAKE_PROFIT_TOKEN_V2
                PackedToken::Address(pair.address),
                PackedToken::Address(input_token),
                PackedToken::Byte(flip),
                PackedToken::Half(amount_in),
                PackedToken::Byte(wallet),
            ])
        };
        payload
    } else {
        let payload = Vec::new();
        payload
    }
    
}

pub fn prepare_payload_sell_weth_v2(
    //&self,
    pair: Pool,
    wallets: Vec<u8>
) -> Vec<u8> {
    let (token0, _) = if pair.token_0 < pair.token_1 { (pair.token_0, pair.token_1) } else { ( pair.token_1, pair.token_0 ) };
    let flip = if token0 == get_weth_address() { 0 } else { 1 };

    let input_token = if pair.token_0 == get_weth_address() { pair.token_1 } else { pair.token_0};
    let (payload, _) = encode_packed(&[
        PackedToken::Byte(2), // SellWethV2
        PackedToken::Address(pair.address),
        PackedToken::Address(input_token),
        PackedToken::Byte(flip),
        PackedToken::WalletShift(wallets),
        
    ]);
    payload
}
