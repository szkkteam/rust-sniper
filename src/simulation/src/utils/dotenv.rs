use ethers::prelude::*;


/// Construct the bundle signer
/// This is your flashbots searcher identity
pub fn get_bundle_signer() -> LocalWallet {
    let private_key = std::env::var("ETHERS_FLASHBOTS_SIGNER")
        .expect("Required environment variable \"ETHERS_FLASHBOTS_SIGNER\" not set");
    private_key
        .parse::<LocalWallet>()
        .expect("Failed to parse flashbots signer")
}

/// Read environment variables
pub fn read_env_vars() -> Vec<(String, String)> {
    let mut env_vars = Vec::new();
    let keys = vec![
        "ETHERS_WSS_PROVIDER",
        "ETHERS_FLASHBOTS_SIGNER",
    ];
    for key in keys {
        let value = dotenv::var(key).expect(&format!(
            "Required environment variable \"{}\" not set",
            key
        ));
        env_vars.push((key.to_string(), value));
    }
    env_vars
}

/// Return a new ws provider
pub async fn get_ws_provider() -> Provider<Ws> {
    let url =
        dotenv::var("ETHERS_WSS_PROVIDER").expect("Required environment variable \"ETHERS_WSS_PROVIDER\" not set");
    Provider::<Ws>::connect(&url)
        .await
        .expect("RPC Connection Error")
}
