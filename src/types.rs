/// This module contains abstractions for the data structures needed to interact with the Kaspa network.
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetworkType {
    Mainnet,
    Testnet,
    Devnet,
    Simnet,
}

impl serde::Serialize for NetworkType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            NetworkType::Mainnet => serializer.serialize_str("mainnet"),
            NetworkType::Testnet => serializer.serialize_str("testnet"),
            NetworkType::Devnet => serializer.serialize_str("devnet"),
            NetworkType::Simnet => serializer.serialize_str("simnet"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse<T> {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<T>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

// Blockchain types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub verbose_data: Option<BlockVerboseData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u16,
    pub parents: Vec<String>,
    pub hash_merkle_root: String,
    pub accepted_id_merkle_root: String,
    pub utxo_commitment: String,
    pub timestamp: u64,
    pub bits: u32,
    pub nonce: u64,
    pub daa_score: u64,
    pub blue_work: String,
    pub blue_score: u64,
    pub pruning_point: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockVerboseData {
    pub hash: String,
    pub difficulty: f64,
    pub selected_parent_hash: String,
    pub transaction_ids: Vec<String>,
    pub is_header_only: bool,
    pub blue_score: u64,
    pub children_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub version: u16,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub lock_time: u64,
    pub subnetwork_id: String,
    pub gas: u64,
    pub payload: String,
    pub verbose_data: Option<TransactionVerboseData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionInput {
    pub previous_outpoint: Outpoint,
    pub signature_script: String,
    pub sequence: u64,
    pub sig_op_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutput {
    pub value: u64,
    pub script_public_key: ScriptPublicKey,
    pub verbose_data: Option<TransactionOutputVerboseData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionVerboseData {
    pub transaction_id: String,
    pub hash: String,
    pub mass: u64,
    pub block_hash: String,
    pub block_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutputVerboseData {
    pub script_public_key_type: String,
    pub script_public_key_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outpoint {
    pub transaction_id: String,
    pub index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptPublicKey {
    pub version: u16,
    pub script_public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub amount: u64,
    pub script_public_key: ScriptPublicKey,
    pub block_daa_score: u64,
    pub is_coinbase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub server_version: String,
    pub is_utxo_indexed: bool,
    pub is_synced: bool,
    pub has_notify_command: bool,
    pub mempool_size: u64,
    pub block_count: u64,
    pub header_count: u64,
    pub virtual_daa_score: u64,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub network_name: String,
    pub block_count: u64,
    pub header_count: u64,
    pub virtual_daa_score: u64,
    pub mempool_size: u64,
    pub server_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub balance: u64,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    pub transaction_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateNetworkHashesPerSecondResponse {
    pub network_hashes_per_second: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolEntry {
    pub fee: u64,
    pub transaction: Transaction,
    pub is_orphan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaaScoreTimestampEstimate {
    pub timestamp: u64,
    pub daa_score: u64,
}

// Subscription types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualChainChangedNotification {
    pub removed_chain_block_hashes: Vec<String>,
    pub added_chain_block_hashes: Vec<String>,
    pub accepted_transaction_ids: Vec<AcceptedTransactionIds>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedTransactionIds {
    pub accepting_block_hash: String,
    pub accepted_transaction_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxosChangedNotification {
    pub added: Vec<UtxoChange>,
    pub removed: Vec<UtxoChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoChange {
    pub address: String,
    pub outpoint: Outpoint,
    pub utxo_entry: UtxoEntry,
}

#[derive(Debug)]
pub enum KaspaError {
    HttpError(reqwest::Error),
    JsonError(serde_json::Error),
    RpcError { code: i32, message: String },
    InvalidResponse(String),
    ConnectionError(String),
    AuthenticationError,
    InvalidAddress(String),
    Custom(String),
    Error(String),
}

impl fmt::Display for KaspaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KaspaError::HttpError(e) => write!(f, "HTTP request failed: {}", e),
            KaspaError::JsonError(e) => write!(f, "JSON serialization failed: {}", e),
            KaspaError::RpcError { code, message } => {
                write!(f, "RPC error: code={}, message={}", code, message)
            }
            KaspaError::InvalidResponse(msg) => write!(f, "Invalid response format: {}", msg),
            KaspaError::ConnectionError(msg) => write!(f, "Connection failed: {}", msg),
            KaspaError::AuthenticationError => write!(f, "Authentication failed"),
            KaspaError::InvalidAddress(msg) => write!(f, "Invalid address: {}", msg),
            KaspaError::Custom(msg) => write!(f, "{}", msg),
            KaspaError::Error(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for KaspaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KaspaError::HttpError(e) => Some(e),
            KaspaError::JsonError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for KaspaError {
    fn from(err: reqwest::Error) -> Self {
        KaspaError::HttpError(err)
    }
}

impl From<serde_json::Error> for KaspaError {
    fn from(err: serde_json::Error) -> Self {
        KaspaError::JsonError(err)
    }
}

pub type Result<T> = std::result::Result<T, KaspaError>;
