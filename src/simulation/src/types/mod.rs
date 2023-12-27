use ethers::prelude::{Address};
use std::str::FromStr;
use serde::{Deserialize, Serialize, Serializer, de::{self, Deserializer}};
use serde::Deserialize as DeserializeTrait;
use std::{fmt, hash::{Hash, Hasher}};

pub mod interface;

pub fn serialize_trader_id<S>(trader_id: &TraderId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_str(&trader_id.to_string())
}

pub fn deserialize_trader_id<'de, D>(deserializer: D) -> Result<TraderId, D::Error>
where
    D: Deserializer<'de>,
{
    struct TraderIdVisitor;

    impl<'de> de::Visitor<'de> for TraderIdVisitor {
        type Value = TraderId;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string in the format 'user_id_token_id'")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let parts: Vec<&str> = value.split('_').collect();

            if parts.len() != 2 {
                return Err(E::custom("Invalid TraderId format"));
            }

            let user_id = parts[0].to_string();
            let token_id = Address::from_str(parts[1])
                .map_err(|_| E::custom("Invalid token_id format"))?;

            Ok(TraderId { user_id, token_id })
        }
    }

    deserializer.deserialize_str(TraderIdVisitor)
}

pub trait IdGenerator {
    fn determine_id(user_id: &String, token_id: &Address) -> Self;
}


#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TraderId {
    pub user_id: String,
    pub token_id: Address,
}

impl fmt::Display for TraderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{}_{:#x}", self.user_id, self.token_id))
    }
}

impl<'de> Deserialize<'de> for TraderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_trader_id(deserializer)
    }
}

impl IdGenerator for TraderId {
    fn determine_id(user_id: &String, token_id: &Address) -> Self {
        Self {
            user_id: user_id.clone(),
            token_id: token_id.clone(),
        }
    }
}

impl Hash for TraderId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_string().hash(state);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileId {
    pub user_id: String,
    pub token_id: Address,
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{}_{:#x}_profile", self.user_id, self.token_id))
    }
}

impl IdGenerator for ProfileId {
    fn determine_id(user_id: &String, token_id: &Address) -> Self {
        Self {
            user_id: user_id.clone(),
            token_id: token_id.clone(),
        }
    }
}

impl From<TraderId> for ProfileId {
    fn from(value: TraderId) -> Self {
        Self {
            user_id: value.user_id,
            token_id: value.token_id
        }
    }
}

impl From<&TraderId> for ProfileId {
    fn from(value: &TraderId) -> Self {
        Self {
            user_id: value.user_id.clone(),
            token_id: value.token_id.clone()
        }
    }
}

pub fn serialize_position_id<S>(trader_id: &PositionId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_str(&trader_id.to_string())
}

pub fn deserialize_position_id<'de, D>(deserializer: D) -> Result<PositionId, D::Error>
where
    D: Deserializer<'de>,
{
    struct PositionIdVisitor;

    impl<'de> de::Visitor<'de> for PositionIdVisitor {
        type Value = PositionId;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string in the format 'user_id_token_id'")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let parts: Vec<&str> = value.split('_').collect();

            let user_id = parts[0].to_string();
            let token_id = Address::from_str(parts[1])
                .map_err(|_| E::custom("Invalid token_id format"))?;

            Ok(PositionId { user_id, token_id })
        }
    }

    deserializer.deserialize_str(PositionIdVisitor)
}



#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PositionId {
    pub user_id: String,
    pub token_id: Address,
}

impl fmt::Display for PositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{}_{:#x}_position", self.user_id, self.token_id))
    }
}

impl<'de> Deserialize<'de> for PositionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_position_id(deserializer)
    }
}

impl IdGenerator for PositionId {
    fn determine_id(user_id: &String, token_id: &Address) -> Self {
        Self {
            user_id: user_id.clone(),
            token_id: token_id.clone(),
        }
    }
}

impl From<TraderId> for PositionId {
    fn from(value: TraderId) -> Self {
        Self {
            user_id: value.user_id,
            token_id: value.token_id
        }
    }
}

impl From<&TraderId> for PositionId {
    fn from(value: &TraderId) -> Self {
        Self {
            user_id: value.user_id.clone(),
            token_id: value.token_id.clone()
        }
    }
}


pub fn serialize_order_id<S>(trader_id: &OrderId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_str(&trader_id.to_string())
}

pub fn deserialize_order_id<'de, D>(deserializer: D) -> Result<OrderId, D::Error>
where
    D: Deserializer<'de>,
{
    struct OrderIdVisitor;

    impl<'de> de::Visitor<'de> for OrderIdVisitor {
        type Value = OrderId;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string in the format 'user_id_token_id'")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let parts: Vec<&str> = value.split('_').collect();

            let user_id = parts[0].to_string();
            let token_id = Address::from_str(parts[1])
                .map_err(|_| E::custom("Invalid token_id format"))?;

            Ok(OrderId { user_id, token_id })
        }
    }

    deserializer.deserialize_str(OrderIdVisitor)
}


#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderId {
    pub user_id: String,
    pub token_id: Address,
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{}_{:#x}_order", self.user_id, self.token_id))
    }
}

impl<'de> Deserialize<'de> for OrderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_order_id(deserializer)
    }
}


impl IdGenerator for OrderId {
    fn determine_id(user_id: &String, token_id: &Address) -> Self {
        Self {
            user_id: user_id.clone(),
            token_id: token_id.clone(),
        }
    }
}

impl From<TraderId> for OrderId {
    fn from(value: TraderId) -> Self {
        Self {
            user_id: value.user_id,
            token_id: value.token_id
        }
    }
}

impl From<&TraderId> for OrderId {
    fn from(value: &TraderId) -> Self {
        Self {
            user_id: value.user_id.clone(),
            token_id: value.token_id.clone()
        }
    }
}


pub fn serialize_transaction_id<S>(trader_id: &TransactionId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_str(&trader_id.to_string())
}

pub fn deserialize_transaction_id<'de, D>(deserializer: D) -> Result<TransactionId, D::Error>
where
    D: Deserializer<'de>,
{
    struct TransactionIdVisitor;

    impl<'de> de::Visitor<'de> for TransactionIdVisitor {
        type Value = TransactionId;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string in the format 'user_id_token_id'")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let parts: Vec<&str> = value.split('_').collect();

            let user_id = parts[0].to_string();
            let token_id = Address::from_str(parts[1])
                .map_err(|_| E::custom("Invalid token_id format"))?;

            Ok(TransactionId { user_id, token_id })
        }
    }

    deserializer.deserialize_str(TransactionIdVisitor)
}


#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransactionId {
    pub user_id: String,
    pub token_id: Address,
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{}_{:#x}_transaction", self.user_id, self.token_id))
    }
}

impl<'de> Deserialize<'de> for TransactionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_transaction_id(deserializer)
    }
}

impl IdGenerator for TransactionId {
    fn determine_id(user_id: &String, token_id: &Address) -> Self {
        Self {
            user_id: user_id.clone(),
            token_id: token_id.clone(),
        }
    }
}

impl From<TraderId> for TransactionId {
    fn from(value: TraderId) -> Self {
        Self {
            user_id: value.user_id,
            token_id: value.token_id
        }
    }
}

impl From<&TraderId> for TransactionId {
    fn from(value: &TraderId) -> Self {
        Self {
            user_id: value.user_id.clone(),
            token_id: value.token_id.clone()
        }
    }
}

impl From<OrderId> for TransactionId {
    fn from(value: OrderId) -> Self {
        Self {
            user_id: value.user_id,
            token_id: value.token_id
        }
    }
}

impl From<&OrderId> for TransactionId {
    fn from(value: &OrderId) -> Self {
        Self {
            user_id: value.user_id.clone(),
            token_id: value.token_id.clone()
        }
    }
}
 