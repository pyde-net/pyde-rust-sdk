// `access-guard` — Pyde contract demonstrating every halt mechanism
// the chain supports. The SDK's `halt_methods` E2E example deploys
// this contract and walks through each failure mode end-to-end.
//
// State: admin (address) + counter (uint64).
//
// Public entry points:
//   - init                             — constructor; sets admin = caller
//   - get_admin                        — view; returns stored admin
//   - get_count                        — view; returns stored counter
//   - admin_bump(n: uint64) -> uint64  — caller must == admin
//   - cause_revert_with_message        — plain UTF-8 revert
//   - cause_revert_with_err_forbidden  — revert with "ERR_FORBIDDEN" token
//   - cause_revert_with_negative_code  — revert with "-5" embedded
//   - cause_panic                      — triggers WASM trap

package main

import (
	"encoding/binary"
	"unsafe"
)

// ── Storage field names ───────────────────────────────────────

var fieldAdmin = []byte("admin")
var fieldCounter = []byte("counter")

// ── Slot derivation ──────────────────────────────────────────

// deriveSlot computes Poseidon2(self_address || field || key).
func deriveSlot(field []byte, key []byte) [32]byte {
	var preimage [32 + 96]byte
	total := 32 + len(field) + len(key)
	self_address(int32(uintptr(unsafe.Pointer(&preimage[0]))))
	copy(preimage[32:32+len(field)], field)
	copy(preimage[32+len(field):total], key)

	var out [32]byte
	hash_poseidon2(
		int32(uintptr(unsafe.Pointer(&preimage[0]))),
		int32(total),
		int32(uintptr(unsafe.Pointer(&out[0]))),
	)
	return out
}

// ── Storage codecs ───────────────────────────────────────────

func loadAdmin() [32]byte {
	slot := deriveSlot(fieldAdmin, nil)
	var buf [32]byte
	actual := sload(
		int32(uintptr(unsafe.Pointer(&slot[0]))),
		int32(uintptr(unsafe.Pointer(&buf[0]))),
		32,
	)
	if actual <= 0 {
		// Missing slot — return zero address.
		var zero [32]byte
		return zero
	}
	return buf
}

func storeAdmin(addr [32]byte) {
	slot := deriveSlot(fieldAdmin, nil)
	sstore(
		int32(uintptr(unsafe.Pointer(&slot[0]))),
		int32(uintptr(unsafe.Pointer(&addr[0]))),
		32,
	)
}

func loadCounter() uint64 {
	slot := deriveSlot(fieldCounter, nil)
	var buf [8]byte
	actual := sload(
		int32(uintptr(unsafe.Pointer(&slot[0]))),
		int32(uintptr(unsafe.Pointer(&buf[0]))),
		8,
	)
	if actual <= 0 {
		return 0
	}
	return binary.LittleEndian.Uint64(buf[:])
}

func storeCounter(value uint64) {
	slot := deriveSlot(fieldCounter, nil)
	var buf [8]byte
	binary.LittleEndian.PutUint64(buf[:], value)
	sstore(
		int32(uintptr(unsafe.Pointer(&slot[0]))),
		int32(uintptr(unsafe.Pointer(&buf[0]))),
		8,
	)
}

// ── Return helpers ──────────────────────────────────────────

func emitU64(value uint64) {
	var buf [8]byte
	binary.LittleEndian.PutUint64(buf[:], value)
	pyde_return(int32(uintptr(unsafe.Pointer(&buf[0]))), 8)
	for {
	}
}

func emitAddress(addr [32]byte) {
	pyde_return(int32(uintptr(unsafe.Pointer(&addr[0]))), 32)
	for {
	}
}

func emitVoid() {
	pyde_return(0, 0)
	for {
	}
}

// revertWith reverts with `reason` and never returns.
func revertWith(reason []byte) {
	revert(int32(uintptr(unsafe.Pointer(&reason[0]))), int32(len(reason)))
	for {
	}
}

// ── Authorization guard ─────────────────────────────────────

// callerEqualsAdmin returns true if the immediate caller's
// address matches the stored admin.
func callerEqualsAdmin() bool {
	var callerAddr [32]byte
	caller(int32(uintptr(unsafe.Pointer(&callerAddr[0]))))
	admin := loadAdmin()
	for i := 0; i < 32; i++ {
		if callerAddr[i] != admin[i] {
			return false
		}
	}
	return true
}

// guardAdmin reverts with "unauthorized" if the caller isn't the
// admin. Reused by every state-mutating entry that needs auth.
func guardAdmin() {
	if !callerEqualsAdmin() {
		revertWith([]byte("unauthorized: caller is not admin"))
	}
}

// ── Calldata helpers ────────────────────────────────────────

func readArgU64() uint64 {
	var buf [8]byte
	var limit [4]byte
	binary.LittleEndian.PutUint32(limit[:], 8)
	calldata_copy(
		int32(uintptr(unsafe.Pointer(&buf[0]))),
		int32(uintptr(unsafe.Pointer(&limit[0]))),
	)
	return binary.LittleEndian.Uint64(buf[:])
}

// ── Public entry points ─────────────────────────────────────

//go:wasmexport init
func initContract() {
	var deployer [32]byte
	caller(int32(uintptr(unsafe.Pointer(&deployer[0]))))
	storeAdmin(deployer)
	storeCounter(0)
	emitVoid()
}

//go:wasmexport get_admin
func get_admin() {
	emitAddress(loadAdmin())
}

//go:wasmexport get_count
func get_count() {
	emitU64(loadCounter())
}

//go:wasmexport admin_bump
func admin_bump() {
	guardAdmin()
	n := readArgU64()
	next := loadCounter() + n
	storeCounter(next)
	emitU64(next)
}

// ── Halt-method demonstrators ───────────────────────────────

//go:wasmexport cause_revert_with_message
func cause_revert_with_message() {
	revertWith([]byte("custom message: contract deliberately reverted"))
}

//go:wasmexport cause_revert_with_err_forbidden
func cause_revert_with_err_forbidden() {
	revertWith([]byte("ERR_FORBIDDEN: this entry never permits execution"))
}

//go:wasmexport cause_revert_with_negative_code
func cause_revert_with_negative_code() {
	revertWith([]byte("aborted with code -5"))
}

//go:wasmexport cause_panic
func cause_panic() {
	// Out-of-bounds index in TinyGo wasm-unknown -> unreachable trap.
	// The engine catches the trap and returns ERR_CROSS_CALL_FAILED.
	var arr [1]byte
	idx := int32(1_000_000)
	_ = arr[idx]
	emitVoid()
}

func main() {}
