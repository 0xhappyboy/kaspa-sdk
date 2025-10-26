/// This module contains the wallet interoperability module related functions of the kaspa network.
use crate::types::{KaspaError, NetworkType, Result};
use crate::types::{
    Outpoint, ScriptPublicKey, Transaction, TransactionInput, TransactionOutput, UtxoEntry,
};
use bech32::{self, ToBase32, Variant};
use ripemd::Ripemd160;
use secp256k1::rand;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const MAIN_NET_ID: &str = "0000000000000000000000000000000000000000";

pub struct KaspaKeypair {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
}

pub struct Wallet {
    keys: HashMap<String, KaspaKeypair>,    // address -> keypair
    utxos: HashMap<String, Vec<UtxoEntry>>, // address -> utxos
    addresses: HashSet<String>,
    network: NetworkType,
}

impl Wallet {
    pub fn new(network: NetworkType) -> Self {
        Self {
            keys: HashMap::new(),
            utxos: HashMap::new(),
            addresses: HashSet::new(),
            network,
        }
    }

    // create new address
    pub fn create_new_address(&mut self) -> Result<String> {
        let secp = Secp256k1::new();
        let (secret_key, public_key) = secp.generate_keypair(&mut rand::thread_rng());
        let keypair = KaspaKeypair {
            secret_key,
            public_key,
        };
        let address = self.public_key_to_kaspa_address(&public_key)?;
        self.keys.insert(address.clone(), keypair);
        self.addresses.insert(address.clone());
        Ok(address)
    }

    /// public key to kaspa address
    fn public_key_to_kaspa_address(&self, public_key: &PublicKey) -> Result<String> {
        let public_key_bytes = public_key.serialize();
        let sha256_hash = Sha256::digest(&public_key_bytes);
        let ripemd160_hash = Ripemd160::digest(&sha256_hash);
        // byte version
        let byte_version = match self.network {
            NetworkType::Mainnet => 0x00,
            NetworkType::Testnet => 0x6f,
            NetworkType::Devnet => 0x6f,
            NetworkType::Simnet => 0x6f,
        };
        let mut payload = Vec::with_capacity(21);
        payload.push(byte_version);
        payload.extend_from_slice(&ripemd160_hash);
        let checksum = Self::calculate_checksum(&payload);
        payload.extend_from_slice(&checksum[..4]);
        // Base58
        let address = bs58::encode(&payload).into_string();
        Ok(address)
    }

    // calculate Base58 check sum
    fn calculate_checksum(data: &[u8]) -> [u8; 4] {
        let first_sha = Sha256::digest(data);
        let second_sha = Sha256::digest(&first_sha);
        let mut checksum = [0u8; 4];
        checksum.copy_from_slice(&second_sha[..4]);
        checksum
    }

    // create Bech32 address
    fn public_key_to_bech32_address(&self, public_key: &PublicKey) -> Result<String> {
        let public_key_bytes = public_key.serialize();
        // SHA256 + RIPEMD160
        let sha256_hash = Sha256::digest(&public_key_bytes);
        let ripemd160_hash = Ripemd160::digest(&sha256_hash);
        // bech32
        let hrp = match self.network {
            NetworkType::Mainnet => "kaspa",
            NetworkType::Testnet => "kaspatest",
            NetworkType::Devnet => "kaspadev",
            NetworkType::Simnet => "kaspasim",
        };
        let encoded = bech32::encode(hrp, ripemd160_hash.to_base32(), Variant::Bech32)
            .map_err(|e| KaspaError::Custom(format!("Bech32 encoding failed: {}", e)))?;
        Ok(encoded)
    }

    // import private key
    pub fn import_private_key(&mut self, private_key_hex: &str) -> Result<String> {
        let private_key_bytes = hex::decode(private_key_hex)
            .map_err(|e| KaspaError::Custom(format!("Invalid hex: {}", e)))?;
        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .map_err(|e| KaspaError::Custom(format!("Invalid private key: {}", e)))?;
        let secp = Secp256k1::new();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let keypair = KaspaKeypair {
            secret_key,
            public_key,
        };
        let address = self.public_key_to_kaspa_address(&public_key)?;
        self.keys.insert(address.clone(), keypair);
        self.addresses.insert(address.clone());
        Ok(address)
    }

    /// export private key
    pub fn export_private_key(&self, address: &str) -> Result<String> {
        let keypair = self
            .keys
            .get(address)
            .ok_or_else(|| KaspaError::Custom("Address not found in wallet".to_string()))?;
        Ok(hex::encode(keypair.secret_key.secret_bytes()))
    }

    /// get balance
    pub fn get_balance(&self, address: &str) -> u64 {
        self.utxos
            .get(address)
            .map(|utxos| utxos.iter().map(|utxo| utxo.amount).sum())
            .unwrap_or(0)
    }

    // get total balance
    pub fn get_total_balance(&self) -> u64 {
        self.addresses
            .iter()
            .map(|addr| self.get_balance(addr))
            .sum()
    }

    // add utxo
    pub fn add_utxo(&mut self, address: &str, utxo: UtxoEntry) {
        self.utxos
            .entry(address.to_string())
            .or_insert_with(Vec::new)
            .push(utxo);
    }

    /// remove utxo
    pub fn remove_utxo(&mut self, address: &str, outpoint: &Outpoint) -> Result<()> {
        if let Some(utxos) = self.utxos.get_mut(address) {
            utxos.retain(|utxo| true);
        }
        Ok(())
    }

    /// build transaction
    pub fn build_transaction(
        &self,
        from_address: &str,
        to_address: &str,
        amount: u64,
        fee_per_input: u64,
    ) -> Result<Transaction> {
        let keypair = self
            .keys
            .get(from_address)
            .ok_or_else(|| KaspaError::Custom("From address not found".to_string()))?;
        let utxos = self
            .utxos
            .get(from_address)
            .ok_or_else(|| KaspaError::Custom("No UTXOs for address".to_string()))?;
        let (selected_indices, total_input) = self.select_utxos(utxos, amount, fee_per_input)?;
        let selected_utxos: Vec<&UtxoEntry> = selected_indices
            .iter()
            .map(|&index| &utxos[index])
            .collect();
        let total_fee = selected_utxos.len() as u64 * fee_per_input;
        let change = total_input
            .checked_sub(amount + total_fee)
            .ok_or_else(|| KaspaError::Custom("Insufficient balance".to_string()))?;
        // build treade input
        let inputs = self.build_inputs(&selected_utxos)?;
        // build treade output
        let outputs = self.build_outputs(to_address, amount, from_address, change)?;
        // create transaction
        let transaction = Transaction {
            version: 0,
            inputs,
            outputs,
            lock_time: 0,
            subnetwork_id: MAIN_NET_ID.to_string(), // Main net id
            gas: 0,
            payload: "".to_string(),
            verbose_data: None,
        };
        Ok(transaction)
    }

    /// select UTXOs
    fn select_utxos(
        &self,
        utxos: &[UtxoEntry],
        amount: u64,
        fee_per_input: u64,
    ) -> Result<(Vec<usize>, u64)> {
        let mut selected_indices = Vec::new();
        let mut total = 0u64;
        let mut indices: Vec<usize> = (0..utxos.len()).collect();
        indices.sort_by_key(|&i| utxos[i].amount);
        for index in indices {
            selected_indices.push(index);
            total += utxos[index].amount;
            let estimated_fee = selected_indices.len() as u64 * fee_per_input;
            if total >= amount + estimated_fee {
                break;
            }
        }
        if total < amount {
            return Err(KaspaError::Custom("Insufficient balance".to_string()));
        }
        Ok((selected_indices, total))
    }

    /// build inputs
    fn build_inputs(&self, utxos: &[&UtxoEntry]) -> Result<Vec<TransactionInput>> {
        let mut inputs = Vec::new();
        for (index, utxo) in utxos.iter().enumerate() {
            let input = TransactionInput {
                previous_outpoint: Outpoint {
                    transaction_id: format!("tx_{}", index),
                    index: 0,
                },
                signature_script: "".to_string(),
                sequence: 0xFFFFFFFF,
                sig_op_count: 1,
            };
            inputs.push(input);
        }
        Ok(inputs)
    }

    /// build outputs
    fn build_outputs(
        &self,
        to_address: &str,
        amount: u64,
        change_address: &str,
        change_amount: u64,
    ) -> Result<Vec<TransactionOutput>> {
        let mut outputs = Vec::new();
        let to_script = self.address_to_script(to_address)?;
        outputs.push(TransactionOutput {
            value: amount,
            script_public_key: to_script,
            verbose_data: None,
        });
        if change_amount > 0 {
            let change_script = self.address_to_script(change_address)?;
            outputs.push(TransactionOutput {
                value: change_amount,
                script_public_key: change_script,
                verbose_data: None,
            });
        }
        Ok(outputs)
    }

    /// address to script
    fn address_to_script(&self, address: &str) -> Result<ScriptPublicKey> {
        let script = format!(
            "76a914{}88ac",
            hex::encode(Sha256::digest(address.as_bytes()))
        );
        Ok(ScriptPublicKey {
            version: 0,
            script_public_key: script,
        })
    }

    /// sign message
    pub fn sign_message(&self, address: &str, message: &str) -> Result<String> {
        let keypair = self
            .keys
            .get(address)
            .ok_or_else(|| KaspaError::Custom("Address not found".to_string()))?;

        let secp = Secp256k1::new();
        let message_hash = Sha256::digest(message.as_bytes());
        let message = Message::from_slice(&message_hash)
            .map_err(|e| KaspaError::Custom(format!("Invalid message hash: {}", e)))?;
        let signature = secp.sign_ecdsa(&message, &keypair.secret_key);
        Ok(hex::encode(signature.serialize_compact()))
    }

    /// verify message
    pub fn verify_message(
        &self,
        address: &str,
        message: &str,
        signature_hex: &str,
    ) -> Result<bool> {
        let keypair = self
            .keys
            .get(address)
            .ok_or_else(|| KaspaError::Custom("Address not found".to_string()))?;
        let signature_bytes = hex::decode(signature_hex)
            .map_err(|e| KaspaError::Custom(format!("Invalid signature hex: {}", e)))?;
        let secp = Secp256k1::new();
        let message_hash = Sha256::digest(message.as_bytes());
        let message = Message::from_slice(&message_hash)
            .map_err(|e| KaspaError::Custom(format!("Invalid message hash: {}", e)))?;
        let signature = Signature::from_compact(&signature_bytes)
            .map_err(|e| KaspaError::Custom(format!("Invalid signature: {}", e)))?;
        Ok(secp
            .verify_ecdsa(&message, &signature, &keypair.public_key)
            .is_ok())
    }

    // Whether the address belongs to the current wallet
    pub fn is_my_address(&self, address: &str) -> bool {
        self.addresses.contains(address)
    }

    // get all address
    pub fn get_all_address(&self) -> Vec<String> {
        self.addresses.iter().cloned().collect()
    }

    // backup wallet
    pub fn backup_wallet(&self) -> WalletBackup {
        let mut key_backups = Vec::new();
        for (address, keypair) in &self.keys {
            key_backups.push(KeyBackup {
                address: address.clone(),
                private_key: hex::encode(keypair.secret_key.secret_bytes()),
                public_key: hex::encode(keypair.public_key.serialize()),
            });
        }
        WalletBackup {
            network: self.network,
            keys: key_backups,
        }
    }

    // restore from backup
    pub fn restore_from_backup(backup: WalletBackup) -> Result<Self> {
        let mut wallet = Wallet::new(backup.network);
        for key_backup in backup.keys {
            wallet.import_private_key(&key_backup.private_key)?;
        }
        Ok(wallet)
    }
}

/// wallet backup struct
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WalletBackup {
    pub network: NetworkType,
    pub keys: Vec<KeyBackup>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyBackup {
    pub address: String,
    pub private_key: String,
    pub public_key: String,
}

impl<'de> serde::Deserialize<'de> for NetworkType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "mainnet" => Ok(NetworkType::Mainnet),
            "testnet" => Ok(NetworkType::Testnet),
            "devnet" => Ok(NetworkType::Devnet),
            "simnet" => Ok(NetworkType::Simnet),
            _ => Err(serde::de::Error::custom("Invalid network type")),
        }
    }
}

// HD wallet
pub struct HDWallet {
    base_wallet: Wallet,
    seed: [u8; 64],
    derivation_path: String,
    key_index: u32,
}

impl HDWallet {
    pub fn new(network: NetworkType, seed_phrase: &str) -> Result<Self> {
        let seed = Self::phrase_to_seed(seed_phrase);
        Ok(Self {
            base_wallet: Wallet::new(network),
            seed,
            derivation_path: "m/44'/111111'/0'/0".to_string(), // Kaspa BIP44 path
            key_index: 0,
        })
    }

    pub fn generate_next_address(&mut self) -> Result<String> {
        let derived_key = self.derive_key(self.key_index)?;
        let address = self
            .base_wallet
            .public_key_to_kaspa_address(&derived_key.public_key)?;
        self.base_wallet.keys.insert(address.clone(), derived_key);
        self.base_wallet.addresses.insert(address.clone());
        self.key_index += 1;
        Ok(address)
    }

    fn derive_key(&self, index: u32) -> Result<KaspaKeypair> {
        let secp = Secp256k1::new();
        let mut hasher = Sha256::new();
        hasher.update(&self.seed);
        hasher.update(&index.to_be_bytes());
        let hash_result = hasher.finalize();
        let secret_key = SecretKey::from_slice(&hash_result)
            .map_err(|e| KaspaError::Custom(format!("Key derivation failed: {}", e)))?;
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        Ok(KaspaKeypair {
            secret_key,
            public_key,
        })
    }

    fn phrase_to_seed(phrase: &str) -> [u8; 64] {
        let mut seed = [0u8; 64];
        let phrase_bytes = phrase.as_bytes();
        let len = phrase_bytes.len().min(64);
        seed[..len].copy_from_slice(&phrase_bytes[..len]);
        seed
    }
}
