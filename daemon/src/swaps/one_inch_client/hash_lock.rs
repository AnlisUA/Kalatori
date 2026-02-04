use alloy::primitives::{B256, keccak256};

/// Simple Merkle tree for multiple secrets
/// Uses the same algorithm as OpenZeppelin's SimpleMerkleTree
///
/// OpenZeppelin's algorithm:
/// 1. Sort leaves by their byte value
/// 2. Store tree in array format: tree[0] = root, leaves at end in reverse order
/// 3. Use commutative hash (sort pair before hashing)
#[derive(Debug, Clone)]
pub struct SimpleMerkleTree {
    /// The complete tree stored as an array (tree[0] = root)
    tree: Vec<B256>,
    /// Original leaf indices mapping (sorted_index -> original_index)
    leaf_indices: Vec<usize>,
    /// Number of leaves
    num_leaves: usize,
}

impl SimpleMerkleTree {
    /// Build a Merkle tree from leaves (matches OpenZeppelin SimpleMerkleTree.of)
    /// Leaves are sorted before building the tree
    pub fn new(leaves: Vec<B256>) -> Self {
        assert!(!leaves.is_empty(), "Cannot create Merkle tree with no leaves");

        // Create indexed leaves for sorting
        let mut indexed_leaves: Vec<(usize, B256)> = leaves
            .iter()
            .enumerate()
            .map(|(i, leaf)| (i, *leaf))
            .collect();

        // Sort leaves by their byte value (matching OpenZeppelin's compare)
        indexed_leaves.sort_by(|a, b| a.1.as_slice().cmp(b.1.as_slice()));

        let num_leaves = leaves.len();
        let tree_size = 2 * num_leaves - 1;
        let mut tree = vec![B256::default(); tree_size];

        // Store the mapping from sorted position to original index
        let leaf_indices: Vec<usize> = indexed_leaves.iter().map(|(i, _)| *i).collect();

        // Place leaves at the end of the array in reverse order
        // tree[tree.length - 1 - i] = leaves[i]
        for (i, (_, leaf)) in indexed_leaves.iter().enumerate() {
            tree[tree_size - 1 - i] = *leaf;
        }

        // Build tree from leaves to root
        // for (let i = tree.length - 1 - leaves.length; i >= 0; i--)
        //   tree[i] = nodeHash(tree[leftChildIndex(i)], tree[rightChildIndex(i)])
        for i in (0..tree_size - num_leaves).rev() {
            let left = tree[Self::left_child_index(i)];
            let right = tree[Self::right_child_index(i)];
            tree[i] = Self::hash_pair(&left, &right);
        }

        Self { tree, leaf_indices, num_leaves }
    }

    #[inline]
    fn left_child_index(i: usize) -> usize {
        2 * i + 1
    }

    #[inline]
    fn right_child_index(i: usize) -> usize {
        2 * i + 2
    }

    #[inline]
    fn parent_index(i: usize) -> usize {
        (i - 1) / 2
    }

    #[inline]
    fn sibling_index(i: usize) -> usize {
        if i % 2 == 0 {
            i - 1 // even index -> sibling is i-1
        } else {
            i + 1 // odd index -> sibling is i+1
        }
    }

    /// Hash two nodes together (sorted for commutative hashing)
    /// Matches OpenZeppelin's standardNodeHash
    fn hash_pair(a: &B256, b: &B256) -> B256 {
        // Sort the pair before hashing (commutative hash)
        let (left, right) = if a.as_slice() < b.as_slice() {
            (a, b)
        } else {
            (b, a)
        };

        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(left.as_slice());
        data.extend_from_slice(right.as_slice());
        keccak256(&data)
    }

    /// Get the Merkle root
    pub fn root(&self) -> B256 {
        self.tree[0]
    }

    /// Get the tree index for a leaf at the given original index
    fn get_tree_index(&self, original_index: usize) -> usize {
        // Find the sorted position of this original index
        let sorted_pos = self.leaf_indices.iter()
            .position(|&i| i == original_index)
            .expect("Original index not found");
        // Leaves are stored at the end in reverse order
        self.tree.len() - 1 - sorted_pos
    }

    /// Get proof for a leaf at given original index
    pub fn get_proof(&self, original_index: usize) -> Vec<B256> {
        assert!(original_index < self.num_leaves, "Index out of bounds");

        let mut proof = Vec::new();
        let mut tree_index = self.get_tree_index(original_index);

        // Walk up the tree, collecting siblings
        while tree_index > 0 {
            let sibling = Self::sibling_index(tree_index);
            if sibling < self.tree.len() {
                proof.push(self.tree[sibling]);
            }
            tree_index = Self::parent_index(tree_index);
        }

        proof
    }

    /// Verify a proof
    pub fn verify_proof(leaf: &B256, proof: &[B256], root: &B256) -> bool {
        let mut computed = *leaf;

        for sibling in proof {
            computed = Self::hash_pair(&computed, sibling);
        }

        computed == *root
    }
}

/// Represents a hashlock for 1inch Fusion+ orders
///
/// For single fills: hashlock = keccak256(secret)
/// For multiple fills: hashlock = merkle_root | (leaf_count << 240)
#[derive(Debug, Clone)]
pub enum HashLock {
    /// Single fill - just the hash of the secret
    SingleFill {
        hashlock: B256,
    },
    /// Multiple fills - merkle root with leaf count encoded
    MultipleFills {
        hashlock: B256,
        tree: SimpleMerkleTree,
        secrets: Vec<B256>,
    },
}

impl HashLock {
    /// Create a hashlock for a single fill order
    /// hashlock = keccak256(secret)
    pub fn for_single_fill(secret: &B256) -> Self {
        Self::SingleFill {
            hashlock: keccak256(secret.as_slice()),
        }
    }

    /// Create a hashlock for multiple fills order
    /// Uses a Merkle tree of secrets with the leaf count encoded in upper 16 bits
    pub fn for_multiple_fills(secrets: &[B256]) -> Self {
        assert!(secrets.len() >= 3, "Multiple fills requires at least 3 secrets");

        // Create merkle leaves: keccak256(abi.encodePacked(uint64(idx), keccak256(secret)))
        let leaves = Self::get_merkle_leaves(secrets);

        // Build merkle tree
        let tree = SimpleMerkleTree::new(leaves);
        let root = tree.root();

        // Encode leaf count in upper 16 bits (bits 240-255)
        // rootWithCount = root | ((leaves.length - 1) << 240)
        let leaf_count = (secrets.len() - 1) as u64;
        let mut root_bytes = root.0;

        // Set upper 16 bits (bytes 0-1 in big-endian) to leaf_count
        // Bits 240-255 correspond to bytes 0-1
        let count_bytes = (leaf_count as u16).to_be_bytes();
        root_bytes[0] = count_bytes[0];
        root_bytes[1] = count_bytes[1];

        Self::MultipleFills {
            hashlock: B256::from(root_bytes),
            tree,
            secrets: secrets.to_vec(),
        }
    }

    /// Create merkle leaves from secrets
    /// leaf[i] = keccak256(abi.encodePacked(uint64(idx), keccak256(secret)))
    pub fn get_merkle_leaves(secrets: &[B256]) -> Vec<B256> {
        secrets.iter().enumerate().map(|(idx, secret)| {
            let secret_hash = keccak256(secret.as_slice());
            // abi.encodePacked(uint64, bytes32) = 8 bytes + 32 bytes = 40 bytes
            let mut data = Vec::with_capacity(40);
            data.extend_from_slice(&(idx as u64).to_be_bytes()); // uint64 big-endian
            data.extend_from_slice(secret_hash.as_slice());
            keccak256(&data)
        }).collect()
    }

    /// Get the hashlock value (bytes32)
    pub fn value(&self) -> B256 {
        match self {
            Self::SingleFill { hashlock } => *hashlock,
            Self::MultipleFills { hashlock, .. } => *hashlock,
        }
    }

    /// Get the number of secrets/parts
    pub fn secrets_count(&self) -> usize {
        match self {
            Self::SingleFill { .. } => 1,
            Self::MultipleFills { secrets, .. } => secrets.len(),
        }
    }

    /// Get secret at index
    pub fn get_secret(&self, idx: usize) -> Option<&B256> {
        match self {
            Self::SingleFill { .. } if idx == 0 => None, // Caller should use the original secret
            Self::SingleFill { .. } => None,
            Self::MultipleFills { secrets, .. } => secrets.get(idx),
        }
    }

    /// Get all secrets
    pub fn secrets(&self) -> &[B256] {
        match self {
            Self::SingleFill { .. } => &[],
            Self::MultipleFills { secrets, .. } => secrets,
        }
    }

    /// Get merkle proof for a secret at given index (only for multiple fills)
    pub fn get_proof(&self, idx: usize) -> Option<Vec<B256>> {
        match self {
            Self::SingleFill { .. } => None,
            Self::MultipleFills { tree, .. } => Some(tree.get_proof(idx)),
        }
    }

    /// Check if this is a single fill hashlock
    pub fn is_single_fill(&self) -> bool {
        matches!(self, Self::SingleFill { .. })
    }

    /// Get the leaf count encoded in the hashlock (for multiple fills)
    /// This is (secrets.len() - 1) encoded in bits 240-255
    pub fn get_parts_count(&self) -> u64 {
        match self {
            Self::SingleFill { .. } => 0,
            Self::MultipleFills { hashlock, .. } => {
                let bytes = hashlock.as_slice();
                u16::from_be_bytes([bytes[0], bytes[1]]) as u64
            }
        }
    }
}
