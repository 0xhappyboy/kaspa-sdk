pub mod tool;
pub mod types;

use crate::types::{BlockHeader, Result};
use crate::types::{Block, KaspaError, RpcRequest, RpcResponse};

use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// kaspa client
pub struct KaspaClient {
    client: Client,
    base_url: String,
    request_id: AtomicU64,
    username: Option<String>,
    password: Option<String>,
}

impl KaspaClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
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

}
