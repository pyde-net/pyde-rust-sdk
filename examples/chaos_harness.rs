//! Chaos harness — mixed-contract load generator for a local Pyde
//! cluster.
//!
//! Drives a deterministic pool of accounts against a running
//! 4-validator cluster: derives N wallets from a fixed seed, funds
//! them + publishes their pubkeys (`--bootstrap`), then submits a
//! weighted mix of transfers, contract calls, and encrypted
//! transfers, round-robined across every validator RPC so peers
//! have real gossip to exchange. A token-bucket governor ramps the
//! rate; a thermal governor backs off when the machine's load
//! average climbs, so a laptop running the whole stack in-process
//! doesn't cook itself.
//!
//! This is dev tooling for pre-mainnet load validation — UX +
//! indexer-correctness under realistic mixed load, NOT a TPS claim.
//! The realistic ceiling on a single laptop hosting 4 validators +
//! explorer + monitoring is a few hundred plaintext tx/s, not the
//! cloud-box number.
//!
//! ## Config (env)
//! - `CHAOS_RPCS` — comma list of RPC URLs
//!   (default the four cluster ports 9933-9936).
//! - `CHAOS_ACCOUNTS` — pool size (default 3000).
//! - `CHAOS_TARGET_TPS` / `CHAOS_MAX_TPS` — rate ramp bounds
//!   (default 200 -> 500).
//! - `CHAOS_RAMP_SECS` — ramp duration (default 900).
//! - `CHAOS_DURATION_SECS` — soak-window length (default 1800).
//! - `CHAOS_ENCRYPTED_FRAC` — 0..1 share of encrypted txs
//!   (default 0.05).
//! - `CHAOS_CONTRACTS_JSON` — path to a JSON map of deployed
//!   contract addresses (keys: prediction, flashloan_vault, ...);
//!   when absent the contract mix is disabled and only
//!   transfers + encrypted transfers run.
//! - `CHAOS_BOOTSTRAP` — `1` to run the fund + RegisterPubkey
//!   preflight before the load loop.
//! - `CHAOS_OPERATOR_SEED` — 32-byte hex seed for the prefunded
//!   funding wallet used during bootstrap. Fund this address
//!   externally first (e.g. `pyde soak --recipient <addr>
//!   --value-per-tx <big>`).
//! - `CHAOS_FUND_QUANTA` — per-account funding amount during
//!   bootstrap (default 100_000_000 = 0.1 PYDE).
//!
//! ## Run
//! ```sh
//! # one-time preflight (fund + register 3000 accounts)
//! CHAOS_BOOTSTRAP=1 CHAOS_OPERATOR_SEED=0x... \
//!   cargo run --release --example chaos_harness
//! # then the soak
//! CHAOS_CONTRACTS_JSON=deployed-contracts.json \
//!   cargo run --release --example chaos_harness
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pyde_crypto::threshold::{threshold_encrypt, ThresholdPublicKey};
use pyde_rust_sdk::contract::Contract;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ContractAbi, EncryptedTxEnvelope, TxType};
use pyde_rust_sdk::{Address, Provider, Signer, TxBuilder, Wallet};
use tokio::sync::Semaphore;

const DEFAULT_RPCS: &str = "http://127.0.0.1:9933,http://127.0.0.1:9934,http://127.0.0.1:9935,http://127.0.0.1:9936";
const GAS_TRANSFER: u64 = 21_000;
const GAS_CALL: u64 = 2_000_000;
const MAX_IN_FLIGHT: usize = 64;

/// One kind of transaction the mix can emit. Counters are per-kind
/// so the NDJSON stream shows the shape of the load, not just a
/// total.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Transfer,
    Encrypted,
    PredictionDeposit,
    PredictionCreateMarket,
    FlashloanDeposit,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Transfer => "transfer",
            Kind::Encrypted => "encrypted",
            Kind::PredictionDeposit => "prediction.deposit",
            Kind::PredictionCreateMarket => "prediction.create_market",
            Kind::FlashloanDeposit => "flashloan.deposit",
        }
    }
}

/// Shared load-loop state. Atomics for the counters (read by the
/// NDJSON emitter without a lock); a `Mutex<HashMap>` for the
/// per-account nonce store (short critical section, no `.await`
/// held across the lock).
struct Shared {
    wallets: Vec<Wallet>,
    providers: Vec<Arc<RootProvider<HttpTransport>>>,
    chain_id: u64,
    /// Per-account next nonce, seeded lazily from `get_nonce`.
    nonces: Mutex<HashMap<Address, u64>>,
    /// Current epoch threshold pubkey for the encrypted lane, or
    /// `None` if the lane is disabled / unavailable. Refreshed
    /// periodically to survive epoch rotation.
    tpk: Mutex<Option<ThresholdPublicKey>>,
    /// Deployed contracts, keyed by role. Only build_tx (sync,
    /// offline) is used, so one instance per role is reused across
    /// every RPC.
    prediction: Option<Contract>,
    flashloan_vault: Option<Contract>,
    submitted: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    by_kind: Mutex<HashMap<&'static str, u64>>,
    rr: AtomicU64,
}

impl Shared {
    fn provider_rr(&self) -> Arc<RootProvider<HttpTransport>> {
        let i = self.rr.fetch_add(1, Ordering::Relaxed) as usize % self.providers.len();
        self.providers[i].clone()
    }

    /// Reserve + return the nonce for `addr`, seeding from the chain
    /// on first use. Increments the local counter so concurrent
    /// submits from the same account don't collide on nonce.
    async fn next_nonce(&self, addr: &Address) -> Result<u64, String> {
        {
            let mut g = self.nonces.lock().unwrap();
            if let Some(n) = g.get_mut(addr) {
                let cur = *n;
                *n += 1;
                return Ok(cur);
            }
        }
        // Not seeded — fetch from chain (outside the lock).
        let chain_nonce = self
            .provider_rr()
            .get_nonce(addr)
            .await
            .map_err(|e| format!("get_nonce: {e}"))?;
        let mut g = self.nonces.lock().unwrap();
        let entry = g.entry(*addr).or_insert(chain_nonce);
        let cur = *entry;
        *entry += 1;
        Ok(cur)
    }

    /// Return an unspent nonce to the pool after a failed submit so the
    /// account doesn't desync (advance past the chain's base and then
    /// reject every subsequent tx). Only rolls back if `nonce` is still
    /// the account's most recent — a later concurrent submit may have
    /// already claimed the next slot.
    fn rollback_nonce(&self, addr: &Address, nonce: u64) {
        let mut g = self.nonces.lock().unwrap();
        if let Some(n) = g.get_mut(addr) {
            if *n == nonce + 1 {
                *n = nonce;
            }
        }
    }

    fn record(&self, kind: Kind, ok: bool) {
        self.submitted.fetch_add(1, Ordering::Relaxed);
        if ok {
            self.succeeded.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failed.fetch_add(1, Ordering::Relaxed);
        }
        let mut g = self.by_kind.lock().unwrap();
        *g.entry(kind.as_str()).or_insert(0) += 1;
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Deterministic per-account seed: `blake3("pyde-harness-v1/" || idx)`.
/// Stable across runs so the same pool + addresses reproduce.
fn account_seed(idx: u32) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"pyde-harness-v1/");
    h.update(&idx.to_le_bytes());
    *h.finalize().as_bytes()
}

/// 1-minute load average, parsed from `uptime`. Returns `None` if
/// the command isn't available or the output can't be parsed —
/// callers treat that as "no thermal pressure".
fn load_avg_1m() -> Option<f64> {
    let out = std::process::Command::new("uptime").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    // "... load average(s): 3.16 3.53 3.61"
    let idx = s.find("load average").map(|i| i + "load average".len())?;
    let tail = &s[idx..];
    tail.split(|c: char| c == ':' || c == 's')
        .flat_map(|seg| seg.split([' ', ',']))
        .find_map(|tok| tok.trim().parse::<f64>().ok())
}

fn ncpu() -> f64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as f64)
        .unwrap_or(8.0)
}

/// Load a Contract for `role` from the deployed-address map, if
/// present + reachable. Fetches the deployed wasm, extracts the ABI
/// custom section, and constructs a Contract bound to the first
/// provider (only `build_tx` — offline — is used).
async fn load_contract(
    addrs: &serde_json::Value,
    role: &str,
    provider: &Arc<RootProvider<HttpTransport>>,
) -> Option<Contract> {
    let hexaddr = addrs.get(role)?.as_str()?;
    let addr = Address::from_hex(hexaddr).ok()?;
    let code = match provider.get_contract_code(&addr).await {
        Ok(c) if !c.is_empty() => c,
        _ => {
            eprintln!("chaos: contract {role} at {hexaddr} has no code; skipping");
            return None;
        }
    };
    let abi: ContractAbi = match pyde_rust_sdk::abi::extract_abi(&code) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("chaos: extract_abi({role}) failed: {e}; skipping");
            return None;
        }
    };
    let dyn_provider: Arc<dyn Provider> = provider.clone();
    Some(Contract::new(addr, abi, dyn_provider))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpcs: Vec<String> = std::env::var("CHAOS_RPCS")
        .unwrap_or_else(|_| DEFAULT_RPCS.to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Utility mode: derive + print the operator address (so the
    // operator can be funded before --bootstrap runs), then exit.
    if std::env::var("CHAOS_PRINT_OPERATOR").map(|v| v == "1").unwrap_or(false) {
        let seed = std::env::var("CHAOS_OPERATOR_SEED")
            .ok()
            .and_then(|h| {
                let b = hex::decode(h.trim_start_matches("0x")).ok()?;
                let mut s = [0u8; 32];
                if b.len() == 32 {
                    s.copy_from_slice(&b);
                    Some(s)
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("CHAOS_OPERATOR_SEED (32-byte hex) required"))?;
        let op = Wallet::from_seed(&seed)?;
        println!("{}", op.address());
        return Ok(());
    }

    // Utility mode: print the first N pool addresses (for on-chain
    // state inspection), then exit.
    if let Ok(cnt) = std::env::var("CHAOS_PRINT_POOL") {
        let n: u32 = cnt.parse().unwrap_or(5);
        for i in 0..n {
            let w = Wallet::from_seed(&account_seed(i))?;
            println!("{i} {}", w.address());
        }
        return Ok(());
    }

    let n_accounts: u32 = env_or("CHAOS_ACCOUNTS", 3000u32);
    let target_tps: f64 = env_or("CHAOS_TARGET_TPS", 200.0);
    let max_tps: f64 = env_or("CHAOS_MAX_TPS", 500.0);
    let ramp_secs: f64 = env_or("CHAOS_RAMP_SECS", 900.0);
    let duration_secs: u64 = env_or("CHAOS_DURATION_SECS", 1800u64);
    let encrypted_frac: f64 = env_or("CHAOS_ENCRYPTED_FRAC", 0.05);
    let bootstrap = std::env::var("CHAOS_BOOTSTRAP").map(|v| v == "1").unwrap_or(false);
    let fund_quanta: u128 = env_or("CHAOS_FUND_QUANTA", 100_000_000u128);

    eprintln!(
        "chaos: rpcs={} accounts={} tps={}->{} ramp={}s dur={}s enc_frac={} bootstrap={}",
        rpcs.len(),
        n_accounts,
        target_tps,
        max_tps,
        ramp_secs,
        duration_secs,
        encrypted_frac,
        bootstrap
    );

    // ── Providers ──────────────────────────────────────────────
    let mut providers = Vec::with_capacity(rpcs.len());
    for url in &rpcs {
        let t = HttpTransport::new(url.clone())?;
        providers.push(Arc::new(RootProvider::new(t)));
    }
    let chain_id = providers[0].chain_id().await?;
    eprintln!("chaos: connected, chain_id={chain_id}");

    // ── Wallet pool ────────────────────────────────────────────
    let mut wallets = Vec::with_capacity(n_accounts as usize);
    for i in 0..n_accounts {
        wallets.push(Wallet::from_seed(&account_seed(i))?);
    }
    eprintln!("chaos: derived {} wallets", wallets.len());

    // ── Threshold pubkey (encrypted lane) ──────────────────────
    let tpk = fetch_tpk(&providers[0]).await;
    if tpk.is_none() {
        eprintln!("chaos: encrypted lane unavailable (no real threshold pubkey); disabling encrypted mix");
    }

    // ── Deployed contracts (optional) ──────────────────────────
    let (prediction, flashloan_vault) = match std::env::var("CHAOS_CONTRACTS_JSON") {
        Ok(path) => match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(map) => {
                    let p = load_contract(&map, "prediction", &providers[0]).await;
                    let f = load_contract(&map, "flashloan_vault", &providers[0]).await;
                    eprintln!(
                        "chaos: contracts loaded — prediction={} flashloan_vault={}",
                        p.is_some(),
                        f.is_some()
                    );
                    (p, f)
                }
                Err(e) => {
                    eprintln!("chaos: bad contracts json: {e}; contract mix disabled");
                    (None, None)
                }
            },
            Err(e) => {
                eprintln!("chaos: can't read {path}: {e}; contract mix disabled");
                (None, None)
            }
        },
        Err(_) => {
            eprintln!("chaos: CHAOS_CONTRACTS_JSON unset; contract mix disabled");
            (None, None)
        }
    };

    let shared = Arc::new(Shared {
        wallets,
        providers,
        chain_id,
        nonces: Mutex::new(HashMap::new()),
        tpk: Mutex::new(tpk),
        prediction,
        flashloan_vault,
        submitted: AtomicU64::new(0),
        succeeded: AtomicU64::new(0),
        failed: AtomicU64::new(0),
        by_kind: Mutex::new(HashMap::new()),
        rr: AtomicU64::new(0),
    });

    // ── Bootstrap: fund + RegisterPubkey ───────────────────────
    if bootstrap {
        bootstrap_accounts(&shared, fund_quanta).await?;
        eprintln!("chaos: bootstrap complete");
        // Bootstrap-only run — don't proceed to the load loop unless
        // the operator also wants load in the same invocation.
        if std::env::var("CHAOS_BOOTSTRAP_THEN_RUN").map(|v| v != "1").unwrap_or(true) {
            eprintln!("chaos: bootstrap-only (set CHAOS_BOOTSTRAP_THEN_RUN=1 to continue into load)");
            return Ok(());
        }
    }

    run_load(&shared, target_tps, max_tps, ramp_secs, duration_secs, encrypted_frac).await;
    Ok(())
}

/// Fetch + parse the current epoch threshold pubkey. `None` when no
/// real (non-mock) Kyber pubkey is published.
async fn fetch_tpk(provider: &Arc<RootProvider<HttpTransport>>) -> Option<ThresholdPublicKey> {
    let rec = provider.get_threshold_public_key().await.ok()??;
    if !rec.is_real() {
        return None;
    }
    let pk_bytes = hex::decode(rec.public_key.trim_start_matches("0x")).ok()?;
    ThresholdPublicKey::from_bytes(&pk_bytes)
}

/// Fund every pool account from the operator wallet, then have each
/// account publish its FALCON pubkey (RegisterPubkey). Both phases
/// round-robin the RPCs. The operator's nonce is managed locally so
/// the funding burst can pipeline.
async fn bootstrap_accounts(shared: &Arc<Shared>, fund_quanta: u128) -> anyhow::Result<()> {
    // CHAOS_SKIP_FUNDING=1 skips the operator-register + funding phases
    // (for when the pool is already funded from a prior run) and goes
    // straight to pool RegisterPubkey.
    let skip_funding = std::env::var("CHAOS_SKIP_FUNDING").map(|v| v == "1").unwrap_or(false);
    if !skip_funding {
    let op_seed = std::env::var("CHAOS_OPERATOR_SEED")
        .ok()
        .and_then(|h| {
            let b = hex::decode(h.trim_start_matches("0x")).ok()?;
            let mut s = [0u8; 32];
            if b.len() == 32 {
                s.copy_from_slice(&b);
                Some(s)
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("CHAOS_OPERATOR_SEED (32-byte hex) required for --bootstrap"))?;
    let operator = Arc::new(Wallet::from_seed(&op_seed)?);
    let op_addr = operator.address();
    let op_bal = shared.providers[0].get_balance(&op_addr).await?;
    eprintln!(
        "chaos: operator {op_addr} balance={op_bal} quanta, funding {} accounts x {fund_quanta}",
        shared.wallets.len()
    );

    // Phase 0 — register the operator's pubkey. Every account
    // (operator included) must publish its FALCON pubkey before the
    // chain will accept its signed txs; without this the operator's
    // funding transfers are rejected with "sender has no auth keys".
    // Idempotent: a second RegisterPubkey for an already-registered
    // account is rejected harmlessly, so re-runs are safe.
    {
        let op_nonce0 = shared.providers[0].get_nonce(&op_addr).await.unwrap_or(0);
        if op_nonce0 == 0 {
            let tx = TxBuilder::new()
                .from(op_addr)
                .to(Address::ZERO)
                .chain_id(shared.chain_id)
                .nonce(0)
                .gas_limit(200_000)
                .value(0)
                .tx_type(TxType::RegisterPubkey)
                .data(operator.pubkey().as_bytes().to_vec())
                .build()?;
            match shared.providers[0].send_raw_transaction(&tx).await {
                Ok(h) => {
                    eprintln!("chaos: operator RegisterPubkey submitted hash={h}; polling receipt");
                    for _ in 0..15 {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        match shared.providers[0].get_receipt(&h).await {
                            Ok(Some(r)) => {
                                eprintln!("chaos: operator register receipt: {r:?}");
                                break;
                            }
                            Ok(None) => {}
                            Err(e) => eprintln!("chaos: get_receipt err: {e}"),
                        }
                    }
                }
                Err(e) => eprintln!("chaos: operator RegisterPubkey failed at submit: {e}"),
            }
        } else {
            eprintln!("chaos: operator already has nonce {op_nonce0}; assuming registered");
        }
    }
    if op_bal < fund_quanta.saturating_mul(shared.wallets.len() as u128) {
        eprintln!(
            "chaos: WARNING operator balance may be insufficient; fund it via `pyde soak --recipient {op_addr} --value-per-tx <big>`"
        );
    }

    // Phase 1 — funding. Sequential nonce off the operator; bounded
    // concurrency on the network I/O.
    //
    // RegisterPubkey is NONCE-NEUTRAL: it installs the account's auth
    // keys without advancing the sender's nonce base. So the operator's
    // first *value* tx (funding) starts at the same base we read before
    // registering — NOT base+1. The chain reports this base directly via
    // get_nonce ("sender base=N"); we trust it.
    // The mempool accepts a bounded window of nonces above the sender's
    // included base and rejects gaps ("nonce N not acceptable (sender
    // base=B)"). Concurrent out-of-order submits therefore fail. We
    // submit strictly in nonce order and retry when the window is
    // momentarily full — as prior funding txs get included, base
    // advances and the next nonce becomes acceptable.
    let mut op_nonce = shared.providers[0].get_nonce(&op_addr).await?;
    eprintln!("chaos: operator funding starts at nonce {op_nonce}");
    let mut funded = 0u64;
    for w in &shared.wallets {
        let to = w.address();
        let mut tx = match TxBuilder::new()
            .from(op_addr)
            .chain_id(shared.chain_id)
            .nonce(op_nonce)
            .gas_limit(GAS_TRANSFER)
            .transfer(to, fund_quanta)
            .build()
        {
            Ok(t) => t,
            Err(_) => continue,
        };
        if operator.sign_tx(&mut tx).await.is_err() {
            continue;
        }
        let mut attempt = 0u32;
        loop {
            match shared.provider_rr().send_raw_transaction(&tx).await {
                Ok(_) => {
                    funded += 1;
                    op_nonce += 1;
                    break;
                }
                Err(e) => {
                    let es = e.to_string();
                    // Both window-full ("not acceptable") and the
                    // per-sender rate limit ("rate limit exceeded") are
                    // transient: back off and retry the SAME nonce so the
                    // sequence stays gap-free.
                    let transient = es.contains("not acceptable") || es.contains("rate limit");
                    if transient && attempt < 150 {
                        attempt += 1;
                        // Window full (nonce ahead of included base) or
                        // rate-limited — wait patiently for inclusion to
                        // advance the base; this paces funding to the
                        // chain's real inclusion rate.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                    static ONCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if ONCE.fetch_add(1, Ordering::Relaxed) < 3 {
                        eprintln!("chaos: funding err (nonce {op_nonce}): {e}");
                    }
                    op_nonce += 1;
                    break;
                }
            }
        }
        // Pace under the per-sender rate limit (10/s, burst 20).
        tokio::time::sleep(Duration::from_millis(110)).await;
        if funded % 100 == 0 && funded > 0 {
            eprintln!("chaos: funded {funded}/{}", shared.wallets.len());
        }
    }
    eprintln!("chaos: funding submitted {funded}/{}", shared.wallets.len());
    // Let funding settle before RegisterPubkey (register needs the
    // account to exist + not be nonce-gated behind an unincluded fund).
    tokio::time::sleep(Duration::from_secs(8)).await;
    } // end !skip_funding

    // Phase 2 — RegisterPubkey, paced + RECEIPT-CONFIRMED.
    //
    // "submitted OK" != "committed". Under a burst, a register's tx
    // body may not propagate from the origin validator to the anchor
    // producer, so it's drained locally but never becomes canonical —
    // no receipt, no auth keys, and the account's first transfer is
    // then rejected 'sender has no auth keys'. So we (a) submit with
    // bounded concurrency well inside the mempool nonce window + the
    // per-sender rate limit, routed through v0 (providers[0]), and
    // (b) poll each register's receipt to Success before counting it,
    // resubmitting once on receipt timeout. This is both the interim
    // unblock (trickle lets single-origin gossip keep up) and the
    // permanent correctness gate before the load loop.
    let reg_conc: usize = env_or("CHAOS_REGISTER_CONCURRENCY", 12usize);
    let confirm = std::env::var("CHAOS_REGISTER_CONFIRM").map(|v| v != "0").unwrap_or(true);
    eprintln!("chaos: registering {} accounts (concurrency={reg_conc}, confirm={confirm})", shared.wallets.len());
    let sem2 = Arc::new(Semaphore::new(reg_conc));
    let mut rhandles = Vec::new();
    for w in &shared.wallets {
        let from = w.address();
        let pubkey_bytes = w.pubkey().as_bytes().to_vec();
        let prov = shared.providers[0].clone();
        let cid = shared.chain_id;
        let permit = sem2.clone().acquire_owned().await.unwrap();
        rhandles.push(tokio::spawn(async move {
            let _p = permit;
            register_and_confirm(&prov, from, pubkey_bytes, cid, confirm).await
        }));
    }
    let mut registered = 0u64;
    for h in rhandles {
        if h.await.unwrap_or(false) {
            registered += 1;
        }
    }
    eprintln!("chaos: RegisterPubkey confirmed {registered}/{}", shared.wallets.len());
    // Seed the local nonce store at base 0: RegisterPubkey is
    // nonce-neutral, so each account's first *value* tx in the load
    // loop is nonce 0 (not 1). Seeding here avoids a get_nonce read
    // (which lags under load) on first use.
    {
        let mut g = shared.nonces.lock().unwrap();
        for w in &shared.wallets {
            g.insert(w.address(), 0);
        }
    }
    tokio::time::sleep(Duration::from_secs(4)).await;
    Ok(())
}

/// Submit a RegisterPubkey for `from` and (when `confirm`) poll its
/// receipt to Success before returning. Resubmits once if the receipt
/// doesn't land within the window — a submit `Ok` doesn't guarantee
/// the tx body reached the anchor producer, so an unconfirmed register
/// gets re-injected. Returns true iff the account ends up registered
/// (receipt Success, or already-registered on a re-run).
async fn register_and_confirm(
    prov: &Arc<RootProvider<HttpTransport>>,
    from: Address,
    pubkey_bytes: Vec<u8>,
    cid: u64,
    confirm: bool,
) -> bool {
    for _round in 0..2 {
        let tx = match TxBuilder::new()
            .from(from)
            .to(Address::ZERO)
            .chain_id(cid)
            .nonce(0)
            .gas_limit(25_000)
            .value(0)
            .tx_type(TxType::RegisterPubkey)
            .data(pubkey_bytes.clone())
            .build()
        {
            Ok(t) => t,
            Err(_) => return false,
        };
        // Submit — retry the transient rate-limit / nonce-window
        // rejections; treat an already-registered error as success.
        let hash = {
            let mut attempt = 0u32;
            loop {
                match prov.send_raw_transaction(&tx).await {
                    Ok(h) => break h,
                    Err(e) => {
                        let es = e.to_string();
                        if es.contains("already") || es.contains("auth keys") || es.contains("registered") {
                            return true;
                        }
                        if (es.contains("rate limit") || es.contains("not acceptable")) && attempt < 40 {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                        static ONCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                        if ONCE.fetch_add(1, Ordering::Relaxed) < 3 {
                            eprintln!("chaos: register submit err: {e}");
                        }
                        return false;
                    }
                }
            }
        };
        if !confirm {
            return true;
        }
        // Poll the receipt to Success (~22s).
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            if let Ok(Some(r)) = prov.get_receipt(&hash).await {
                return r.is_success();
            }
        }
        // Receipt never appeared — the body likely didn't propagate.
        // Loop re-injects it once.
    }
    false
}

/// The load loop: a token-bucket rate governor (ramping target ->
/// max) modulated by a thermal governor, dispatching a weighted mix
/// of tx kinds with a bounded in-flight cap, emitting NDJSON once a
/// second.
async fn run_load(
    shared: &Arc<Shared>,
    target_tps: f64,
    max_tps: f64,
    ramp_secs: f64,
    duration_secs: u64,
    encrypted_frac: f64,
) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(duration_secs);
    let sem = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let ncpus = ncpu();
    let thermal_ceiling = ncpus * 1.5;

    // Token bucket: refill at the current effective rate; each
    // submit consumes one token.
    let mut tokens = 0.0f64;
    let mut last = Instant::now();
    let mut last_report = Instant::now();
    let mut last_tpk_refresh = Instant::now();
    let mut thermal_scale = 1.0f64;
    let mut last_thermal = Instant::now();
    let mut rng_state: u64 = 0x9E37_79B9_7F4A_7C15;

    // simple xorshift for weighted picks (no rand dep needed on the
    // hot path; deterministic-enough for load shaping)
    let mut next_rand = move || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        rng_state
    };

    println!(
        "{}",
        serde_json::json!({"ev":"start","accounts":shared.wallets.len(),"rpcs":shared.providers.len(),"ncpu":ncpus})
    );

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }

        // Thermal check every 10s.
        if now.duration_since(last_thermal) >= Duration::from_secs(10) {
            last_thermal = now;
            if let Some(l) = load_avg_1m() {
                thermal_scale = if l > thermal_ceiling { 0.5 } else { 1.0 };
                if thermal_scale < 1.0 {
                    eprintln!("chaos: thermal backoff — load_1m={l:.1} > {thermal_ceiling:.1}, halving rate");
                }
            }
        }

        // Effective rate: linear ramp target -> max over ramp_secs,
        // scaled by thermal.
        let elapsed = now.duration_since(start).as_secs_f64();
        let ramp_frac = (elapsed / ramp_secs).min(1.0);
        let base_rate = target_tps + (max_tps - target_tps) * ramp_frac;
        let rate = (base_rate * thermal_scale).max(1.0);

        // Refill tokens.
        let dt = now.duration_since(last).as_secs_f64();
        last = now;
        tokens = (tokens + rate * dt).min(rate); // cap burst at ~1s worth
        if tokens < 1.0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        } else {
            while tokens >= 1.0 {
                tokens -= 1.0;
                let permit = match sem.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => break, // in-flight cap hit; let it drain
                };
                let r = next_rand();
                let kind = pick_kind(shared, r, encrypted_frac);
                let sh = shared.clone();
                let acct_idx = (r as usize >> 8) % sh.wallets.len();
                tokio::spawn(async move {
                    let _p = permit;
                    let ok = submit_one(&sh, kind, acct_idx).await;
                    sh.record(kind, ok);
                });
            }
        }

        // Refresh the epoch pubkey every 60s (survives rotation).
        if now.duration_since(last_tpk_refresh) >= Duration::from_secs(60) {
            last_tpk_refresh = now;
            if let Some(fresh) = fetch_tpk(&shared.providers[0]).await {
                *shared.tpk.lock().unwrap() = Some(fresh);
            }
        }

        // NDJSON once a second.
        if now.duration_since(last_report) >= Duration::from_secs(1) {
            last_report = now;
            emit_report(shared, elapsed, rate, thermal_scale, &sem);
        }
    }

    // Drain in-flight.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let subm = shared.submitted.load(Ordering::Relaxed);
    let ok = shared.succeeded.load(Ordering::Relaxed);
    let fail = shared.failed.load(Ordering::Relaxed);
    let bk = shared.by_kind.lock().unwrap().clone();
    println!(
        "{}",
        serde_json::json!({
            "ev":"summary",
            "duration_secs": start.elapsed().as_secs(),
            "submitted": subm, "succeeded": ok, "failed": fail,
            "by_kind": bk,
        })
    );
}

fn pick_kind(shared: &Arc<Shared>, r: u64, encrypted_frac: f64) -> Kind {
    // Encrypted gets its configured slice (only if the lane is up).
    let enc_up = shared.tpk.lock().unwrap().is_some();
    let roll = (r % 10_000) as f64 / 10_000.0;
    if enc_up && roll < encrypted_frac {
        return Kind::Encrypted;
    }
    // Remaining mass split between transfer + whichever contracts
    // are loaded. Always-succeeding contract methods only.
    let have_pred = shared.prediction.is_some();
    let have_fl = shared.flashloan_vault.is_some();
    // Weighted buckets over the post-encrypted mass.
    let m = r >> 16;
    match (have_pred, have_fl) {
        (true, true) => match m % 100 {
            0..=54 => Kind::Transfer,
            55..=74 => Kind::PredictionDeposit,
            75..=84 => Kind::PredictionCreateMarket,
            _ => Kind::FlashloanDeposit,
        },
        (true, false) => match m % 100 {
            0..=64 => Kind::Transfer,
            65..=84 => Kind::PredictionDeposit,
            _ => Kind::PredictionCreateMarket,
        },
        (false, true) => match m % 100 {
            0..=74 => Kind::Transfer,
            _ => Kind::FlashloanDeposit,
        },
        (false, false) => Kind::Transfer,
    }
}

async fn submit_one(shared: &Arc<Shared>, kind: Kind, acct_idx: usize) -> bool {
    let wallet = &shared.wallets[acct_idx];
    let from = wallet.address();
    let nonce = match shared.next_nonce(&from).await {
        Ok(n) => n,
        Err(_) => return false,
    };
    let cid = shared.chain_id;
    let prov = shared.provider_rr();

    // Wrapped so a failed submit rolls the unspent nonce back into the
    // pool instead of leaving the account desynced (see rollback_nonce).
    // `return false` inside the arms resolves this async block, not the
    // whole fn.
    let ok = async {
    match kind {
        Kind::Transfer => {
            // random recipient among the pool
            let to = shared.wallets[(nonce as usize + acct_idx) % shared.wallets.len()].address();
            let mut tx = match TxBuilder::new()
                .from(from)
                .chain_id(cid)
                .nonce(nonce)
                .gas_limit(GAS_TRANSFER)
                .transfer(to, 1)
                .build()
            {
                Ok(t) => t,
                Err(_) => return false,
            };
            if wallet.sign_tx(&mut tx).await.is_err() {
                return false;
            }
            match prov.send_raw_transaction(&tx).await {
                Ok(_) => true,
                Err(e) => {
                    static ONCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if ONCE.fetch_add(1, Ordering::Relaxed) < 5 {
                        eprintln!("chaos: transfer err (nonce {nonce}): {e}");
                    }
                    false
                }
            }
        }
        Kind::Encrypted => {
            let to = shared.wallets[(nonce as usize + 1) % shared.wallets.len()].address();
            let mut tx = match TxBuilder::new()
                .from(from)
                .chain_id(cid)
                .nonce(nonce)
                .gas_limit(100_000)
                .transfer(to, 1)
                .build()
            {
                Ok(t) => t,
                Err(_) => return false,
            };
            if wallet.sign_tx(&mut tx).await.is_err() {
                return false;
            }
            let tpk = match shared.tpk.lock().unwrap().clone() {
                Some(t) => t,
                None => return false,
            };
            let plaintext = match pyde_rust_sdk::tx::encode(&tx) {
                Ok(b) => b,
                Err(_) => return false,
            };
            let ct = match threshold_encrypt(&tpk, &plaintext) {
                Ok(c) => c,
                Err(_) => return false,
            };
            let envelope = EncryptedTxEnvelope {
                version: EncryptedTxEnvelope::VERSION,
                ciphertext: ct.to_wire_bytes(),
            };
            let hexstr = match borsh::to_vec(&envelope) {
                Ok(b) => format!("0x{}", hex::encode(b)),
                Err(_) => return false,
            };
            prov.send_raw_encrypted_transaction(&hexstr).await.is_ok()
        }
        Kind::PredictionDeposit => {
            submit_call(shared.prediction.as_ref(), wallet, from, cid, nonce, &prov, "deposit", vec![pyde_rust_sdk::contract::Value::U128(1_000)]).await
        }
        Kind::PredictionCreateMarket => {
            submit_call(shared.prediction.as_ref(), wallet, from, cid, nonce, &prov, "create_market", vec![]).await
        }
        Kind::FlashloanDeposit => {
            submit_call(shared.flashloan_vault.as_ref(), wallet, from, cid, nonce, &prov, "deposit", vec![pyde_rust_sdk::contract::Value::U128(1_000)]).await
        }
    }
    }
    .await;
    if !ok {
        shared.rollback_nonce(&from, nonce);
    }
    ok
}

#[allow(clippy::too_many_arguments)]
async fn submit_call(
    contract: Option<&Contract>,
    wallet: &Wallet,
    from: Address,
    cid: u64,
    nonce: u64,
    prov: &Arc<RootProvider<HttpTransport>>,
    method: &str,
    args: Vec<pyde_rust_sdk::contract::Value>,
) -> bool {
    let Some(c) = contract else { return false };
    let mut tx = match c.build_tx(from, method, args, cid, nonce, GAS_CALL, 0) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if wallet.sign_tx(&mut tx).await.is_err() {
        return false;
    }
    prov.send_raw_transaction(&tx).await.is_ok()
}

fn emit_report(
    shared: &Arc<Shared>,
    elapsed: f64,
    rate: f64,
    thermal_scale: f64,
    sem: &Arc<Semaphore>,
) {
    let subm = shared.submitted.load(Ordering::Relaxed);
    let ok = shared.succeeded.load(Ordering::Relaxed);
    let fail = shared.failed.load(Ordering::Relaxed);
    let in_flight = MAX_IN_FLIGHT - sem.available_permits();
    let bk = shared.by_kind.lock().unwrap().clone();
    println!(
        "{}",
        serde_json::json!({
            "ev":"tick",
            "t": elapsed as u64,
            "submitted": subm, "succeeded": ok, "failed": fail,
            "target_rate": rate as u64,
            "thermal_scale": thermal_scale,
            "in_flight": in_flight,
            "by_kind": bk,
        })
    );
}
