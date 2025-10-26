/// This module is specifically designed to process and analyze the DAG data structure of the Kaspa network.
use crate::types::{Block, KaspaError, Result};
use std::collections::{HashMap, HashSet};

/// Kaspa network DAG data structure processing
///
/// # Example
///
/// Block A (BlueScore: 5)
///     /   \
/// Block B (3)  Block C (4)
///    |     |    /     \
/// Block D (2)  Block E (3)  Block F (2)
///     \    |    /      |     /
///      Block G (1) -- Block H (1)
///           \         /
///          Virtual Block (Tip)
///
pub struct DAGHandler {
    // k: block hash v: block
    blocks: HashMap<String, Block>,
    // parent hash -> [child hash]
    children: HashMap<String, Vec<String>>,
    // current DAG leaf nodes
    tips: HashSet<String>,
}

impl DAGHandler {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            children: HashMap::new(),
            tips: HashSet::new(),
        }
    }

    /// add block to dag
    ///
    /// Before:      After:
    ///   A            A
    ///  / \          / \
    /// B   C  + D   B   C
    ///              |  / \
    ///              D /   E
    ///
    /// childs map: A >[B,C], C->[D,E]
    /// tips: remove: A, Add: D, E
    ///
    pub fn add_block(&mut self, block: Block) -> Result<()> {
        let block_hash = if let Some(verbose_data) = &block.verbose_data {
            verbose_data.hash.clone()
        } else {
            return Err(KaspaError::Custom("Block missing hash".to_string()));
        };
        // Verify that the block hash is consistent with the hash in verbose_data
        if let Some(verbose_data) = &block.verbose_data {
            if verbose_data.hash != block_hash {
                return Err(KaspaError::Custom("Block hash mismatch".to_string()));
            }
        }
        // Update sub-block mapping
        for parent in &block.header.parents {
            self.children
                .entry(parent.clone())
                .or_insert_with(Vec::new)
                .push(block_hash.clone());
        }
        // Complete endpoint management
        self.update_tips(&block_hash, &block.header.parents);
        self.blocks.insert(block_hash.clone(), block);
        // Verify data structure consistency
        self.validate_consistency()?;
        Ok(())
    }

    fn update_tips(&mut self, new_block_hash: &str, parent_hashes: &[String]) {
        // Remove all parent blocks
        for parent in parent_hashes {
            self.tips.remove(parent);
        }
        // Add a new block as the end
        self.tips.insert(new_block_hash.to_string());
        self.handle_orphan_blocks(new_block_hash);
    }

    /// handle orphan blocks
    fn handle_orphan_blocks(&mut self, new_block_hash: &str) {
        if let Some(children) = self.children.get(new_block_hash) {
            for child in children {
                if self.is_block_no_longer_orphan(child) {}
            }
        }
    }

    /// Checks if all parent blocks of a block exist
    fn is_block_no_longer_orphan(&self, block_hash: &str) -> bool {
        if let Some(block) = self.blocks.get(block_hash) {
            block
                .header
                .parents
                .iter()
                .all(|parent| self.blocks.contains_key(parent))
        } else {
            false
        }
    }

    /// Verify that the terminal block does not have any subblocks
    fn validate_consistency(&self) -> Result<()> {
        for tip in &self.tips {
            if let Some(children) = self.children.get(tip) {
                if !children.is_empty() {
                    return Err(KaspaError::Custom(format!(
                        "Tip {} has children, inconsistent state",
                        tip
                    )));
                }
            }
        }
        Ok(())
    }

    ///
    /// calculate blue score
    ///
    /// # Example
    ///
    /// Structure:               blue scores:
    ///      A (5)               A: 1 + max(B:3, C:4) = 5
    ///     / \                  B: 1 + D:2 = 3
    ///    B   C (4)             C: 1 + max(E:3, F:2) = 4
    ///    |  / \                D: 1 + G:1 = 2
    ///    D /   F (2)           E: 1 + G:1 = 3
    ///     |   /                F: 1 + H:1 = 2
    ///     G   H (1)            G: 1 = 1
    ///      \ /                 H: 1 = 1
    ///    virtual
    ///
    pub fn calculate_blue_score(&self, block_hash: &str) -> u64 {
        let mut visited = HashSet::new();
        self.recursive_cal_blue_score(block_hash, &mut visited)
    }

    /// recursively calculates consensus weight
    fn recursive_cal_blue_score(&self, block_hash: &str, visited: &mut HashSet<String>) -> u64 {
        if visited.contains(block_hash) {
            return 0;
        }
        visited.insert(block_hash.to_string());

        let block = match self.blocks.get(block_hash) {
            Some(block) => block,
            None => return 0,
        };
        1 + block
            .header
            .parents
            .iter()
            .map(|parent| self.recursive_cal_blue_score(parent, visited))
            .max()
            .unwrap_or(0)
    }

    /// get virtual block
    pub fn get_virtual_block(&self) -> Option<&Block> {
        self.tips
            .iter()
            .map(|tip| (tip, self.calculate_blue_score(tip)))
            .max_by_key(|(_, score)| *score)
            .and_then(|(tip, _)| self.blocks.get(tip))
    }

    // analyze dag structure
    pub fn analyze_structure(&self) -> DAGAnalysis {
        let mut analysis = DAGAnalysis::new();
        for block in self.blocks.values() {
            analysis.total_blocks += 1;
            analysis.max_parents = analysis.max_parents.max(block.header.parents.len());
            if let Some(verbose_data) = &block.verbose_data {
                analysis.total_blue_score += verbose_data.blue_score;
            }
        }
        analysis.average_blue_score =
            analysis.total_blue_score / analysis.total_blocks.max(1) as u64;
        analysis
    }
}

pub struct DAGAnalysis {
    pub total_blocks: usize,
    pub max_parents: usize,
    pub total_blue_score: u64,
    pub average_blue_score: u64,
    pub tips_count: usize,
}

impl DAGAnalysis {
    fn new() -> Self {
        Self {
            total_blocks: 0,
            max_parents: 0,
            total_blue_score: 0,
            average_blue_score: 0,
            tips_count: 0,
        }
    }
}
