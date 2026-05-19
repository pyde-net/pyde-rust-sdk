//! Inlined wire-format types for threshold-encrypted transactions.
//! Plaintext fields (visible to everyone):
//! sender, nonce, gas_limit, access_list, deadline, chain_id, signature
//! Encrypted fields (hidden until threshold decryption):
//! to, value, calldata
//! Encryption: threshold_encrypt(committee_pk, (to || value || calldata))
//! → ThresholdCiphertext (Kyber encaps + symmetric encryption + MAC)
//! Originally lived in the `pyde-mempool` crate; inlined here so the
//! SDK can build encrypted transactions without pulling in the
//! consensus/mempool side.

use pyde_account::address::Address;
use pyde_crypto::poseidon2::poseidon2_hash;
use pyde_crypto::threshold::{self, ThresholdCiphertext, ThresholdPublicKey};
use pyde_tx::types::AccessEntry;

/// Maximum transaction size (128 KB).
pub const MAX_TX_SIZE: usize = 128 * 1024;

const MAX_ACCESS_ENTRIES: usize = 1024;
const MAX_KEYS_PER_ACCESS_ENTRY: usize = 1024;
/// FALCON-512 signatures are ~600-690 bytes in practice; cap at 1024
/// for headroom.
const MAX_SIG_LEN: usize = 1024;
/// Wire format from `ThresholdCiphertext::to_wire_bytes` is
/// `[ct_len:4][kyber_ct:1088][msg_len:4][encrypted_msg:N][mac:32]`
/// where `N = 48-byte payload header + calldata`, and calldata is
/// bounded by `pyde_tx::validation::MAX_CALLDATA = 64 KB`. Sum
/// ≈ 65 KB; 72 KB leaves ~7 KB headroom for a future format bump.
const MAX_CT_LEN: usize = 72 * 1024;

/// An encrypted transaction. Plaintext fields are visible for
/// validation and scheduling; encrypted fields are hidden until
/// threshold decryption.
#[derive(Clone, Debug)]
pub struct EncryptedTx {
    pub sender: Address,
    pub nonce: u64,
    pub gas_limit: u64,
    pub access_list: Vec<AccessEntry>,
    pub deadline: Option<u64>,
    pub chain_id: u64,
    /// FALCON-512 signature over all fields (plaintext + encrypted).
    pub signature: Vec<u8>,
    /// Threshold-encrypted payload: contains to, value, calldata.
    pub ciphertext: ThresholdCiphertext,
}

/// Bounds-checked cursor used by `EncryptedTx::from_bytes`. Every
/// read returns `None` on underrun; length fields go through
/// `u16_count` / `u32_count` so an attacker-supplied count is
/// capped before any `Vec::with_capacity`.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.remaining() < n {
            return None;
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Some(u64::from_le_bytes(a))
    }

    fn bytes32(&mut self) -> Option<[u8; 32]> {
        let b = self.take(32)?;
        let mut a = [0u8; 32];
        a.copy_from_slice(b);
        Some(a)
    }

    fn u16_count(&mut self, max: usize) -> Option<usize> {
        let n = self.u16()? as usize;
        if n > max {
            return None;
        }
        Some(n)
    }

    fn u32_count(&mut self, max: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        if n > max {
            return None;
        }
        Some(n)
    }
}

impl EncryptedTx {
    /// Total size in bytes (rough estimate for MAX_TX_SIZE check).
    pub fn size(&self) -> usize {
        32 // sender
        + 8 // nonce
        + 8 // gas_limit
        + self.access_list.len() * 68 // rough estimate per entry
        + 8 // deadline
        + 8 // chain_id
        + self.signature.len()
        + self.ciphertext.encrypted_len()
    }

    /// Hash of the encrypted transaction (for deduplication and tx_root).
    pub fn hash(&self) -> [u8; 32] {
        let mut buf = Vec::with_capacity(96);
        buf.extend_from_slice(&self.sender);
        buf.extend_from_slice(&self.nonce.to_le_bytes());
        buf.extend_from_slice(&self.gas_limit.to_le_bytes());
        buf.extend_from_slice(&self.chain_id.to_le_bytes());
        buf.extend_from_slice(&poseidon2_hash(&self.ciphertext.to_bytes()).to_bytes());
        poseidon2_hash(&buf).to_bytes()
    }

    /// Serialize to wire bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let ct_bytes = self.ciphertext.to_wire_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.sender);
        buf.extend_from_slice(&self.nonce.to_le_bytes());
        buf.extend_from_slice(&self.gas_limit.to_le_bytes());
        buf.extend_from_slice(&self.chain_id.to_le_bytes());
        buf.push(self.deadline.is_some() as u8);
        if let Some(d) = self.deadline {
            buf.extend_from_slice(&d.to_le_bytes());
        }
        buf.extend_from_slice(&(self.access_list.len() as u32).to_le_bytes());
        for entry in &self.access_list {
            buf.extend_from_slice(&entry.address);
            buf.extend_from_slice(&(entry.reads.len() as u16).to_le_bytes());
            for r in &entry.reads {
                buf.extend_from_slice(r);
            }
            buf.extend_from_slice(&(entry.writes.len() as u16).to_le_bytes());
            for w in &entry.writes {
                buf.extend_from_slice(w);
            }
        }
        buf.extend_from_slice(&(self.signature.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.signature);
        buf.extend_from_slice(&(ct_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(&ct_bytes);
        buf
    }

    /// Deserialize from wire bytes. Returns `None` (never panics) on
    /// any malformed input; length fields are capped before any
    /// `Vec::with_capacity` to avoid allocation-amplification.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() > MAX_TX_SIZE {
            return None;
        }
        let mut cur = Cursor::new(data);
        let sender = cur.bytes32()?;
        let nonce = cur.u64()?;
        let gas_limit = cur.u64()?;
        let chain_id = cur.u64()?;
        let has_deadline = cur.u8()?;
        let deadline = if has_deadline != 0 {
            Some(cur.u64()?)
        } else {
            None
        };
        let al_count = cur.u32_count(MAX_ACCESS_ENTRIES)?;
        let mut access_list = Vec::with_capacity(al_count);
        for _ in 0..al_count {
            let address = cur.bytes32()?;
            let read_count = cur.u16_count(MAX_KEYS_PER_ACCESS_ENTRY)?;
            let mut reads = Vec::with_capacity(read_count);
            for _ in 0..read_count {
                reads.push(cur.bytes32()?);
            }
            let write_count = cur.u16_count(MAX_KEYS_PER_ACCESS_ENTRY)?;
            let mut writes = Vec::with_capacity(write_count);
            for _ in 0..write_count {
                writes.push(cur.bytes32()?);
            }
            access_list.push(AccessEntry {
                address,
                reads,
                writes,
            });
        }
        let sig_len = cur.u32_count(MAX_SIG_LEN)?;
        let signature = cur.take(sig_len)?.to_vec();
        let ct_len = cur.u32_count(MAX_CT_LEN)?;
        let ct_bytes = cur.take(ct_len)?;
        let ciphertext = ThresholdCiphertext::from_wire_bytes(ct_bytes)?;
        Some(Self {
            sender,
            nonce,
            gas_limit,
            access_list,
            deadline,
            chain_id,
            signature,
            ciphertext,
        })
    }

    /// Check if the transaction has expired.
    pub fn is_expired(&self, current_block: u64) -> bool {
        match self.deadline {
            Some(d) => current_block >= d,
            None => false,
        }
    }

    /// Check if the transaction exceeds the size limit.
    pub fn is_oversized(&self) -> bool {
        self.size() > MAX_TX_SIZE
    }
}

/// Encrypt a transaction's sensitive fields against the committee's
/// threshold public key. Plaintext fields remain visible.
#[allow(clippy::too_many_arguments)]
pub fn encrypt_transaction(
    sender: Address,
    nonce: u64,
    gas_limit: u64,
    access_list: Vec<AccessEntry>,
    deadline: Option<u64>,
    chain_id: u64,
    signature: Vec<u8>,
    to: &Address,
    value: u128,
    calldata: &[u8],
    committee_pk: &ThresholdPublicKey,
) -> Result<EncryptedTx, &'static str> {
    let mut payload = Vec::with_capacity(48 + calldata.len()); // to(32) + value(16) + calldata
    payload.extend_from_slice(to);
    payload.extend_from_slice(&value.to_le_bytes());
    payload.extend_from_slice(calldata);

    let ciphertext = threshold::threshold_encrypt(committee_pk, &payload)
        .map_err(|_| "threshold encryption failed")?;

    Ok(EncryptedTx {
        sender,
        nonce,
        gas_limit,
        access_list,
        deadline,
        chain_id,
        signature,
        ciphertext,
    })
}
