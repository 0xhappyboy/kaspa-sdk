pub mod batch;
pub mod config;
pub mod dag;
pub mod events;
pub mod mempool;
pub mod miner;
pub mod monitor;
pub mod tool;
pub mod types;
pub mod wallet;

use crate::config::KaspaConfig;
use crate::events::{EventListenerConfig, KaspaEventListener};
use crate::tool::default_sequence;
use crate::types::{
    Balance, Block, BlockHeader, DaaScoreTimestampEstimate, EstimateNetworkHashesPerSecondResponse,
    KaspaError, MempoolEntry, NodeInfo, Result, RpcRequest, RpcResponse, SubmitTransactionResponse,
    Transaction, UtxoEntry,
};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct NetWorkStatistics {
    pub daily_transactions: u64,
    pub average_block_size: f64,
    pub network_growth: f64,
    pub active_addresses: usize,
    pub transaction_volume: u64,
}

/// kaspa client
pub struct KaspaClient {
    client: Client,
    base_url: String,
    request_id: AtomicU64,
    username: Option<String>,
    password: Option<String>,
}

impl KaspaClient {
    /// create client
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
            request_id: AtomicU64::new(0),
            username: None,
            password: None,
        }
    }

    /// create client from config
    pub fn from_config(config: KaspaConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.rpc_url,
            request_id: AtomicU64::new(0),
            username: None,
            password: None,
        }
    }

    /// create by username and password
    pub fn auth(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }

    /// create on timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.client = Client::builder().timeout(timeout).build().unwrap();
        self
    }

    /// send request
    async fn send_request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<T> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };
        let mut request_builder = self
            .client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&request);
        // Add authentication if provided
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            request_builder = request_builder.basic_auth(username, Some(password));
        }
        let response: Response = request_builder.send().await?;
        if !response.status().is_success() {
            return Err(KaspaError::ConnectionError(format!(
                "HTTP error: {}",
                response.status()
            )));
        }
        let rpc_response: RpcResponse<T> = response.json().await?;
        if let Some(error) = rpc_response.error {
            return Err(KaspaError::RpcError {
                code: error.code,
                message: error.message,
            });
        }
        rpc_response.result.ok_or_else(|| {
            KaspaError::InvalidResponse("Missing result in RPC response".to_string())
        })
    }

    /// get network statistics
    pub async fn get_network_statistics(&self, days: u64) -> Result<NetWorkStatistics> {
        let current_block_count = self.get_block_count().await?;
        let start_block = current_block_count.saturating_sub(days * 144); // 假设每天144个区块
        let mut total_transactions = 0;
        let mut total_block_size = 0;
        let mut active_addresses = HashSet::new();
        let mut total_volume = 0;
        for block_num in start_block..=current_block_count {
            if let Ok(block) = self.get_block_by_number(block_num).await {
                total_transactions += block.transactions.len();
                for tx in &block.transactions {
                    self.analyze_transaction_addresses(tx, &mut active_addresses);
                    total_volume += self.cal_transaction_volume(tx);
                }
            }
        }
        let block_count = (current_block_count - start_block) as f64;
        let average_block_size = total_block_size as f64 / block_count.max(1.0);
        Ok(NetWorkStatistics {
            daily_transactions: total_transactions as u64 / days,
            average_block_size,
            network_growth: self.cal_network_growth().await?,
            active_addresses: active_addresses.len(),
            transaction_volume: total_volume,
        })
    }

    /// get block info by number
    async fn get_block_by_number(&self, block_number: u64) -> Result<Block> {
        let blocks = self.get_blocks("", false, true).await?;
        blocks
            .first()
            .cloned()
            .ok_or_else(|| KaspaError::Custom(format!("Block number {} not found", block_number)))
    }

    /// analyze transaction addresses
    fn analyze_transaction_addresses(
        &self,
        transaction: &Transaction,
        active_addresses: &mut HashSet<String>,
    ) -> HashSet<String> {
        for output in &transaction.outputs {
            if let Some(verbose_data) = &output.verbose_data {
                active_addresses.insert(verbose_data.script_public_key_address.clone());
            }
        }
        active_addresses.clone()
    }

    /// Calculate the total value of all transactions in a transaction.
    pub fn cal_transaction_volume(&self, transaction: &Transaction) -> u64 {
        transaction.outputs.iter().map(|output| output.value).sum()
    }

    /// calculate network growth
    pub async fn cal_network_growth(&self) -> Result<f64> {
        // Get current block information
        let current_info = self.get_info().await?;
        let current_block_count = current_info.block_count;
        // Get block information from 24 hours ago (estimated by virtual chain changes)
        let virtual_chain = self.get_virtual_chain_from_block("", false).await?;
        // Calculate block growth within 24 hours
        let blocks_24h = if let Some(added_blocks) = virtual_chain.get("addedChainBlockHashes") {
            added_blocks.as_array().map(|arr| arr.len()).unwrap_or(0)
        } else {
            let avg_block_time = 1.0; // Kaspa average block time (seconds)
            let estimated_24h_blocks = (24 * 60 * 60) as f64 / avg_block_time;
            estimated_24h_blocks as usize
        };
        let growth_rate = if current_block_count > 0 {
            (blocks_24h as f64) / (current_block_count as f64) * 100.0
        } else {
            0.0
        };
        Ok(growth_rate)
    }

    /// Address cluster analysis.
    /// Used to identify related addresses belonging to the same user or entity.
    /// Analyze all transactions in the most recent `depth` blocks.
    /// For each transaction, collect all output addresses and cluster together multiple addresses that appear in the same transaction.
    ///
    /// # Example
    /// ```rust
    /// let client = KaspaClient::new("http://localhost:16110");
    /// let clusters = client.analyze_address_clusters(500).await?;
    /// let mixing_patterns: Vec<_> = clusters
    /// .iter()
    /// .filter(|(_, addresses)| {
    ///      // Detection characteristics
    ///       addresses.len() > 20
    /// }).collect();
    /// ```
    pub async fn analyze_address_clusters(
        &self,
        depth: u64,
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut clusters: HashMap<String, Vec<String>> = HashMap::new();
        let mut visited_addresses = HashSet::new();
        let blocks = self.get_blocks("", false, true).await?;
        let total_blocks = blocks.len();
        let start_index = total_blocks.saturating_sub(depth as usize);
        for i in start_index..total_blocks {
            if let Some(block) = blocks.get(i) {
                for tx in &block.transactions {
                    let mut addresses_in_tx = HashSet::new();
                    for output in &tx.outputs {
                        if let Some(verbose_data) = &output.verbose_data {
                            addresses_in_tx.insert(verbose_data.script_public_key_address.clone());
                        }
                    }
                    if addresses_in_tx.len() > 1 {
                        let addresses: Vec<String> = addresses_in_tx.into_iter().collect();
                        let cluster_key = addresses[0].clone();
                        let cluster = clusters.entry(cluster_key).or_insert_with(Vec::new);
                        for address in addresses {
                            if !cluster.contains(&address) && !visited_addresses.contains(&address)
                            {
                                cluster.push(address.clone());
                                visited_addresses.insert(address);
                            }
                        }
                    }
                }
            }
        }
        Ok(clusters)
    }

    /// get block
    pub async fn get_block(&self, hash: &str, include_transactions: bool) -> Result<Block> {
        let params = serde_json::json!([hash, {
            "includeTransactions": include_transactions,
            "includeBlockVerboseData": true
        }]);
        self.send_request("getBlock", Some(params)).await
    }

    /// get blocks
    pub async fn get_blocks(
        &self,
        low_hash: &str,
        include_transactions: bool,
        include_block_verbose_data: bool,
    ) -> Result<Vec<Block>> {
        let params = serde_json::json!([low_hash, {
            "includeTransactions": include_transactions,
            "includeBlockVerboseData": include_block_verbose_data
        }]);
        self.send_request("getBlocks", Some(params)).await
    }

    /// get block count
    pub async fn get_block_count(&self) -> Result<u64> {
        self.send_request("getBlockCount", None).await
    }

    /// get block dag info
    pub async fn get_block_dag_info(&self) -> Result<serde_json::Value> {
        self.send_request("getBlockDagInfo", None).await
    }

    /// get block template
    pub async fn get_block_template(
        &self,
        pay_address: &str,
        extra_data: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params = serde_json::json!({
            "payAddress": pay_address
        });
        if let Some(extra_data) = extra_data {
            params["extraData"] = serde_json::Value::String(extra_data.to_string());
        }
        self.send_request("getBlockTemplate", Some(serde_json::json!([params])))
            .await
    }

    /// get current network
    pub async fn get_current_network(&self) -> Result<String> {
        self.send_request("getCurrentNetwork", None).await
    }

    /// get headers
    pub async fn get_headers(&self, low_hash: &str) -> Result<Vec<BlockHeader>> {
        let params = serde_json::json!([low_hash]);
        self.send_request("getHeaders", Some(params)).await
    }

    /// get peer address
    pub async fn get_peer_addresses(&self) -> Result<Vec<serde_json::Value>> {
        self.send_request("getPeerAddresses", None).await
    }

    /// get sink
    pub async fn get_sink(&self) -> Result<String> {
        self.send_request("getSink", None).await
    }

    pub async fn get_sink_blue_score(&self) -> Result<u64> {
        self.send_request("getSinkBlueScore", None).await
    }

    pub async fn get_mempool_entries(
        &self,
        include_orphan_pool: Option<bool>,
    ) -> Result<Vec<MempoolEntry>> {
        let params = if let Some(include_orphan) = include_orphan_pool {
            Some(serde_json::json!([include_orphan]))
        } else {
            None
        };
        self.send_request("getMempoolEntries", params).await
    }

    pub async fn get_mempool_entries_by_addresses(
        &self,
        addresses: Vec<&str>,
    ) -> Result<Vec<MempoolEntry>> {
        let params = serde_json::json!([addresses]);
        self.send_request("getMempoolEntriesByAddresses", Some(params))
            .await
    }

    pub async fn get_mempool_entry(&self, transaction_id: &str) -> Result<MempoolEntry> {
        let params = serde_json::json!([transaction_id]);
        self.send_request("getMempoolEntry", Some(params)).await
    }

    pub async fn get_connected_peer_info(&self) -> Result<Vec<serde_json::Value>> {
        self.send_request("getConnectedPeerInfo", None).await
    }

    pub async fn get_subnetwork(&self, subnetwork_id: &str) -> Result<serde_json::Value> {
        let params = serde_json::json!([subnetwork_id]);
        self.send_request("getSubnetwork", Some(params)).await
    }

    pub async fn get_utxos_by_addresses(&self, addresses: Vec<&str>) -> Result<Vec<UtxoEntry>> {
        let params = serde_json::json!([addresses]);
        self.send_request("getUtxosByAddresses", Some(params)).await
    }

    pub async fn get_balance_by_address(&self, address: &str) -> Result<Balance> {
        let params = serde_json::json!([address]);
        self.send_request("getBalanceByAddress", Some(params)).await
    }

    pub async fn get_balances_by_addresses(&self, addresses: Vec<&str>) -> Result<Vec<Balance>> {
        let params = serde_json::json!([addresses]);
        self.send_request("getBalancesByAddresses", Some(params))
            .await
    }

    pub async fn get_virtual_chain_from_block(
        &self,
        start_hash: &str,
        include_accepted_transaction_ids: bool,
    ) -> Result<serde_json::Value> {
        let params = serde_json::json!([start_hash, include_accepted_transaction_ids]);
        self.send_request("getVirtualChainFromBlock", Some(params))
            .await
    }

    pub async fn get_info(&self) -> Result<NodeInfo> {
        self.send_request("getInfo", None).await
    }

    // Mining methods
    pub async fn submit_block(&self, block: &str) -> Result<String> {
        let params = serde_json::json!([block]);
        self.send_request("submitBlock", Some(params)).await
    }

    pub async fn estimate_network_hashes_per_second(
        &self,
        window_size: Option<u32>,
    ) -> Result<EstimateNetworkHashesPerSecondResponse> {
        let params = if let Some(window) = window_size {
            Some(serde_json::json!([window]))
        } else {
            None
        };
        self.send_request("estimateNetworkHashesPerSecond", params)
            .await
    }

    pub async fn get_daa_score_timestamp_estimate(
        &self,
        daa_scores: Vec<u64>,
    ) -> Result<Vec<DaaScoreTimestampEstimate>> {
        let params = serde_json::json!([daa_scores]);
        self.send_request("getDaaScoreTimestampEstimate", Some(params))
            .await
    }

    // Transaction methods
    pub async fn submit_transaction(&self, transaction: &str) -> Result<SubmitTransactionResponse> {
        let params = serde_json::json!([transaction]);
        self.send_request("submitTransaction", Some(params)).await
    }

    pub async fn get_transaction(&self, transaction_id: &str) -> Result<Transaction> {
        let params = serde_json::json!([transaction_id]);
        self.send_request("getTransaction", Some(params)).await
    }

    // Subscription methods (these would typically use WebSocket)
    pub async fn notify_block_added(&self, command: Option<&str>) -> Result<()> {
        let params = if let Some(cmd) = command {
            Some(serde_json::json!([cmd]))
        } else {
            None
        };
        self.send_request("notifyBlockAdded", params).await
    }

    pub async fn notify_finality_conflict(&self, command: Option<&str>) -> Result<()> {
        let params = if let Some(cmd) = command {
            Some(serde_json::json!([cmd]))
        } else {
            None
        };
        self.send_request("notifyFinalityConflict", params).await
    }

    pub async fn notify_utxos_changed(
        &self,
        addresses: Vec<&str>,
        command: Option<&str>,
    ) -> Result<()> {
        let mut params_obj = serde_json::json!({
            "addresses": addresses
        });

        if let Some(cmd) = command {
            params_obj["command"] = serde_json::Value::String(cmd.to_string());
        }

        self.send_request("notifyUtxosChanged", Some(serde_json::json!([params_obj])))
            .await
    }

    pub async fn notify_virtual_chain_changed(
        &self,
        include_accepted_transaction_ids: bool,
        command: Option<&str>,
    ) -> Result<()> {
        let mut params_obj = serde_json::json!({
            "includeAcceptedTransactionIds": include_accepted_transaction_ids
        });

        if let Some(cmd) = command {
            params_obj["command"] = serde_json::Value::String(cmd.to_string());
        }

        self.send_request(
            "notifyVirtualChainChanged",
            Some(serde_json::json!([params_obj])),
        )
        .await
    }

    pub async fn stop_notifying_block_added(&self) -> Result<()> {
        self.send_request("stopNotifyingBlockAdded", None).await
    }

    pub async fn stop_notifying_finality_conflict(&self) -> Result<()> {
        self.send_request("stopNotifyingFinalityConflict", None)
            .await
    }

    pub async fn stop_notifying_utxos_changed(&self) -> Result<()> {
        self.send_request("stopNotifyingUtxosChanged", None).await
    }

    pub async fn stop_notifying_virtual_chain_changed(&self) -> Result<()> {
        self.send_request("stopNotifyingVirtualChainChanged", None)
            .await
    }

    // Utility methods
    pub async fn ping(&self) -> Result<bool> {
        match self.get_info().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub async fn get_sync_status(&self) -> Result<bool> {
        let info = self.get_info().await?;
        Ok(info.is_synced)
    }

    /// create event listener
    pub fn create_event_listener(&self) -> KaspaEventListener {
        let ws_url = self.base_url.replace("http", "ws");
        let config = EventListenerConfig {
            ws_url,
            username: self.username.clone(),
            password: self.password.clone(),
            ..Default::default()
        };
        KaspaEventListener::new(config)
    }
}

/// The kaspa client includes some advanced client features.
pub struct KaspaClient2 {
    client: KaspaClient,
}

impl KaspaClient2 {
    pub fn new(client: KaspaClient) -> Self {
        Self { client }
    }
    /// Batch operations
    pub async fn get_multiple_blocks(&self, hashes: Vec<&str>) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        for hash in hashes {
            match self.client.get_block(hash, false).await {
                Ok(block) => blocks.push(block),
                Err(e) => return Err(e),
            }
        }
        Ok(blocks)
    }
    /// get multiple transactions
    pub async fn get_multiple_transactions(
        &self,
        transaction_ids: Vec<&str>,
    ) -> Result<Vec<Transaction>> {
        let mut transactions = Vec::new();
        for tx_id in transaction_ids {
            match self.client.get_transaction(tx_id).await {
                Ok(tx) => transactions.push(tx),
                Err(e) => return Err(e),
            }
        }
        Ok(transactions)
    }
    /// Address management
    pub async fn get_combined_balance(&self, addresses: Vec<&str>) -> Result<u64> {
        let balances = self.client.get_balances_by_addresses(addresses).await?;
        Ok(balances.iter().map(|b| b.balance).sum())
    }

    /// get utxos for addresses
    pub async fn get_utxos_for_addresses(&self, addresses: Vec<&str>) -> Result<Vec<UtxoEntry>> {
        self.client.get_utxos_by_addresses(addresses).await
    }

    /// Blockchain analysis
    pub async fn get_recent_blocks(&self, count: u64) -> Result<Vec<Block>> {
        let current_height = self.client.get_block_count().await?;
        let mut blocks = Vec::new();
        for i in 0..count {
            if current_height > i {
                // This is simplified - in practice you'd need to get block hash by height first
                // For now, we'll get the latest blocks using getBlocks
                if let Ok(mut recent_blocks) = self.client.get_blocks("", false, true).await {
                    if recent_blocks.len() > i as usize {
                        blocks.push(recent_blocks.remove(0));
                    }
                }
            }
        }
        Ok(blocks)
    }

    // Transaction building utilities
    pub async fn create_transaction_template(
        &self,
        from_address: &str,
        to_address: &str,
        amount: u64,
        fee_per_byte: u64,
    ) -> Result<serde_json::Value> {
        // Get UTXOs for the sender
        let utxos = self
            .client
            .get_utxos_by_addresses(vec![from_address])
            .await?;
        // Calculate total available
        let total_available: u64 = utxos.iter().map(|utxo| utxo.amount).sum();
        if total_available < amount {
            return Err(KaspaError::Custom(format!(
                "Insufficient balance: available {}, required {}",
                total_available, amount
            )));
        }
        // Simple transaction template
        let template = serde_json::json!({
            "inputs": utxos.iter().map(|utxo| {
                serde_json::json!({
                    "previousOutpoint": {
                        "transactionId": "TODO", // Would need transaction ID from UTXO
                        "index": 0
                    },
                    "signatureScript": "",
                    "sequence": default_sequence()
                })
            }).collect::<Vec<_>>(),
            "outputs": vec![
                serde_json::json!({
                    "value": amount,
                    "scriptPublicKey": {
                        "version": 0,
                        "scriptPublicKey": to_address // This should be converted to script
                    }
                })
            ],
            "lockTime": 0,
            "subnetworkId": "0000000000000000000000000000000000000000",
            "gas": 0,
            "payload": ""
        });
        Ok(template)
    }

    /// Network monitoring
    pub async fn get_network_stats(&self) -> Result<serde_json::Value> {
        let info = self.client.get_info().await?;
        let peer_info = self.client.get_connected_peer_info().await?;
        let mempool_size = self.client.get_mempool_entries(None).await?.len();

        Ok(serde_json::json!({
            "node_info": info,
            "connected_peers": peer_info.len(),
            "mempool_transactions": mempool_size,
            "network": "kaspa-mainnet" // This should come from getCurrentNetwork
        }))
    }

    /// Health check
    pub async fn health_check(&self) -> Result<serde_json::Value> {
        let is_connected = self.client.ping().await?;
        let is_synced = self.client.get_sync_status().await.unwrap_or(false);
        let block_count = self.client.get_block_count().await.unwrap_or(0);

        Ok(serde_json::json!({
            "status": if is_connected && is_synced { "healthy" } else { "unhealthy" },
            "connected": is_connected,
            "synced": is_synced,
            "block_count": block_count,
            "timestamp": chrono::Utc::now().timestamp()
        }))
    }
}
