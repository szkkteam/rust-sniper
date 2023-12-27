use crate::{
    types::{TraderId, deserialize_trader_id, serialize_trader_id},    
    portfolio::profile::{AntiRug, Priority},
};

use serde::{Deserialize, Serialize};


mod serialize_trader_id {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{de, Serializer, Deserialize, Deserializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
        where T: Display,
              S: Serializer
    {
        serializer.collect_str(value)
    }

}


#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraderCreatedResponseInterface {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
}


#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAntiRugInterface {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub anti_rug: Option<AntiRug>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TerminateTraderInterface {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ForceExitPositionInterface {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub priority: Priority,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TakeProfitInterface {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub sell_percentage: u8,
    pub priority: Priority,
}
