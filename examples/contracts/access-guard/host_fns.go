// Pyde host-fn imports — minimal set the access-guard contract
// actually calls. Trimmed from the canonical template to keep the
// WASM under the chain's 64 KiB calldata cap.

package main

//go:wasmimport pyde sload
func sload(slotPtr int32, outPtr int32, outMaxLen int32) int32

//go:wasmimport pyde sstore
func sstore(slotPtr int32, valPtr int32, valLen int32)

//go:wasmimport pyde caller
func caller(addrOutPtr int32) int32

//go:wasmimport pyde self_address
func self_address(addrOutPtr int32) int32

//go:wasmimport pyde calldata_copy
func calldata_copy(outPtr int32, outLenPtr int32) int32

//go:wasmimport pyde hash_poseidon2
func hash_poseidon2(inPtr int32, inLen int32, outPtr int32)

//go:wasmimport pyde return
func pyde_return(dataPtr int32, dataLen int32)

//go:wasmimport pyde revert
func revert(reasonPtr int32, reasonLen int32)
