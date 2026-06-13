//! Proc-macros for [`pyde-rust-sdk`].
//!
//! ## `pyde_abi!`
//!
//! Generate a strongly-typed contract wrapper from an ABI JSON file.
//!
//! ```ignore
//! pyde_rust_sdk::contract::pyde_abi!(Counter, "abi/counter.json");
//!
//! # async fn run(
//! #     provider: std::sync::Arc<dyn pyde_rust_sdk::Provider>,
//! #     wallet: pyde_rust_sdk::Wallet,
//! #     address: pyde_rust_sdk::Address,
//! # ) -> pyde_rust_sdk::Result<()> {
//! let counter = Counter::new(address, provider);
//! let count: u64 = counter.get_count().await?;
//! let pending = counter.increment(&wallet, 200_000, 0).await?;
//! let receipt = pending.wait_for_receipt().await?;
//! # Ok(()) }
//! ```
//!
//! Each ABI function gets:
//!
//! - **VIEW functions** → `async fn name(&self, args...) -> Result<RetType>` —
//!   dispatches `pyde_call` and decodes the return.
//! - **Non-view functions** → `async fn name(&self, signer: &dyn Signer,
//!   args..., gas_limit: u64, value: u128) -> Result<PendingTx>` — builds
//!   the tx, signs it, submits it.
//!
//! Selectors are derived as `Blake3(name)[..4]`; the SDK doesn't
//! actually use the selector for dispatch (Pyde dispatches by
//! function name through the embedded ABI per HOST_FN_ABI §3.7.4)
//! but the field is preserved for parity with the on-chain ABI.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use serde::Deserialize;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Ident, LitStr, Token};

// ── Input parsing ──────────────────────────────────────────────

/// Parsed `pyde_abi!(StructName, "path/to/abi.json")` invocation.
struct AbiInput {
    struct_name: Ident,
    abi_path: LitStr,
}

impl Parse for AbiInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let struct_name: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let abi_path: LitStr = input.parse()?;
        Ok(Self {
            struct_name,
            abi_path,
        })
    }
}

// ── ABI deserialisation ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonAbi {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    functions: Vec<JsonFunction>,
    #[serde(default)]
    events: Vec<JsonEvent>,
}

#[derive(Debug, Deserialize)]
struct JsonFunction {
    name: String,
    #[serde(default)]
    attrs: u32,
    #[serde(default)]
    params: Vec<JsonParam>,
    #[serde(default)]
    returns: Option<JsonType>,
}

#[derive(Debug, Deserialize)]
struct JsonParam {
    name: String,
    #[serde(rename = "type")]
    ty: JsonType,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonType {
    Simple(String),
    Composite(JsonCompositeType),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum JsonCompositeType {
    Vec {
        inner: Box<JsonType>,
    },
    Map {
        key: Box<JsonType>,
        value: Box<JsonType>,
    },
    Option {
        inner: Box<JsonType>,
    },
    FixedBytes {
        len: u32,
    },
    Custom {
        name: String,
    },
}

#[derive(Debug, Deserialize)]
struct JsonEvent {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    #[serde(default)]
    params: Vec<JsonParam>,
    #[allow(dead_code)]
    #[serde(default)]
    indexed_mask: u8,
}

// ── Codegen helpers ────────────────────────────────────────────

fn rust_type_tokens(ty: &JsonType) -> syn::Result<TokenStream2> {
    Ok(match ty {
        JsonType::Simple(s) => match s.as_str() {
            "u8" => quote!(u8),
            "u16" => quote!(u16),
            "u32" => quote!(u32),
            "u64" => quote!(u64),
            "u128" => quote!(u128),
            "i8" => quote!(i8),
            "i16" => quote!(i16),
            "i32" => quote!(i32),
            "i64" => quote!(i64),
            "i128" => quote!(i128),
            "bool" => quote!(bool),
            "address" => quote!(::pyde_rust_sdk::Address),
            "bytes" => quote!(::std::vec::Vec<u8>),
            "string" => quote!(::std::string::String),
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("unknown ABI type: {other:?}"),
                ))
            }
        },
        JsonType::Composite(JsonCompositeType::Vec { inner }) => {
            let inner_ty = rust_type_tokens(inner)?;
            quote!(::std::vec::Vec<#inner_ty>)
        }
        JsonType::Composite(JsonCompositeType::Map { key, value }) => {
            let k = rust_type_tokens(key)?;
            let v = rust_type_tokens(value)?;
            quote!(::std::collections::BTreeMap<#k, #v>)
        }
        JsonType::Composite(JsonCompositeType::Option { inner }) => {
            let inner_ty = rust_type_tokens(inner)?;
            quote!(::std::option::Option<#inner_ty>)
        }
        JsonType::Composite(JsonCompositeType::FixedBytes { len }) => {
            let len = *len as usize;
            quote!([u8; #len])
        }
        JsonType::Composite(JsonCompositeType::Custom { .. }) => {
            // Custom types come through as opaque Vec<u8> at the
            // macro layer — the typed wrapper passes through the
            // pre-encoded Borsh bytes.
            quote!(::std::vec::Vec<u8>)
        }
    })
}

fn rust_to_value_expr(ty: &JsonType, value_expr: TokenStream2) -> syn::Result<TokenStream2> {
    Ok(match ty {
        JsonType::Simple(s) => match s.as_str() {
            "u8" => quote!(::pyde_rust_sdk::contract::Value::U8(#value_expr)),
            "u16" => quote!(::pyde_rust_sdk::contract::Value::U16(#value_expr)),
            "u32" => quote!(::pyde_rust_sdk::contract::Value::U32(#value_expr)),
            "u64" => quote!(::pyde_rust_sdk::contract::Value::U64(#value_expr)),
            "u128" => quote!(::pyde_rust_sdk::contract::Value::U128(#value_expr)),
            "i8" => quote!(::pyde_rust_sdk::contract::Value::I8(#value_expr)),
            "i16" => quote!(::pyde_rust_sdk::contract::Value::I16(#value_expr)),
            "i32" => quote!(::pyde_rust_sdk::contract::Value::I32(#value_expr)),
            "i64" => quote!(::pyde_rust_sdk::contract::Value::I64(#value_expr)),
            "i128" => quote!(::pyde_rust_sdk::contract::Value::I128(#value_expr)),
            "bool" => quote!(::pyde_rust_sdk::contract::Value::Bool(#value_expr)),
            "address" => quote!(::pyde_rust_sdk::contract::Value::Address(#value_expr)),
            "bytes" => quote!(::pyde_rust_sdk::contract::Value::Bytes(#value_expr)),
            "string" => quote!(::pyde_rust_sdk::contract::Value::String(#value_expr)),
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("unknown ABI type for arg conversion: {other:?}"),
                ))
            }
        },
        JsonType::Composite(JsonCompositeType::FixedBytes { .. }) => {
            quote!(::pyde_rust_sdk::contract::Value::FixedBytes((#value_expr).to_vec()))
        }
        JsonType::Composite(JsonCompositeType::Custom { .. }) => {
            quote!(::pyde_rust_sdk::contract::Value::Custom(#value_expr))
        }
        // Vec / Map / Option carry typed inners; the generated code
        // builds the Value by recursing per element. For v1 we keep
        // this path narrow: callers pass already-encoded Vec<u8> via
        // a Custom or raw tagged value if they want composites.
        JsonType::Composite(JsonCompositeType::Vec { .. })
        | JsonType::Composite(JsonCompositeType::Map { .. })
        | JsonType::Composite(JsonCompositeType::Option { .. }) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "composite ABI types (Vec/Map/Option) are not yet supported by the typed \
                 macro — call through `Contract` directly with `Value` args for now",
            ))
        }
    })
}

fn value_to_rust_expr(ty: &JsonType) -> syn::Result<TokenStream2> {
    Ok(match ty {
        JsonType::Simple(s) => match s.as_str() {
            "u8" => quote!(if let ::pyde_rust_sdk::contract::Value::U8(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected u8, got {:?}", value),
                ));
            }),
            "u16" => quote!(if let ::pyde_rust_sdk::contract::Value::U16(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected u16, got {:?}", value),
                ));
            }),
            "u32" => quote!(if let ::pyde_rust_sdk::contract::Value::U32(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected u32, got {:?}", value),
                ));
            }),
            "u64" => quote!(if let ::pyde_rust_sdk::contract::Value::U64(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected u64, got {:?}", value),
                ));
            }),
            "u128" => quote!(if let ::pyde_rust_sdk::contract::Value::U128(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected u128, got {:?}", value),
                ));
            }),
            "i8" => quote!(if let ::pyde_rust_sdk::contract::Value::I8(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected i8, got {:?}", value),
                ));
            }),
            "i64" => quote!(if let ::pyde_rust_sdk::contract::Value::I64(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected i64, got {:?}", value),
                ));
            }),
            "bool" => quote!(if let ::pyde_rust_sdk::contract::Value::Bool(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected bool, got {:?}", value),
                ));
            }),
            "address" => quote!(
                if let ::pyde_rust_sdk::contract::Value::Address(v) = value {
                    v
                } else {
                    return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                        ::std::format!("expected address, got {:?}", value),
                    ));
                }
            ),
            "bytes" => quote!(if let ::pyde_rust_sdk::contract::Value::Bytes(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected bytes, got {:?}", value),
                ));
            }),
            "string" => quote!(if let ::pyde_rust_sdk::contract::Value::String(v) = value {
                v
            } else {
                return ::std::result::Result::Err(::pyde_rust_sdk::SdkError::InvalidResponse(
                    ::std::format!("expected string, got {:?}", value),
                ));
            }),
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("unknown ABI type for return conversion: {other:?}"),
                ))
            }
        },
        JsonType::Composite(_) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "composite return types are not yet supported by the typed macro \
                 — use `Contract::call` directly",
            ))
        }
    })
}

fn abi_type_tokens(ty: &JsonType) -> syn::Result<TokenStream2> {
    Ok(match ty {
        JsonType::Simple(s) => match s.as_str() {
            "u8" => quote!(::pyde_rust_sdk::types::ParamType::U8),
            "u16" => quote!(::pyde_rust_sdk::types::ParamType::U16),
            "u32" => quote!(::pyde_rust_sdk::types::ParamType::U32),
            "u64" => quote!(::pyde_rust_sdk::types::ParamType::U64),
            "u128" => quote!(::pyde_rust_sdk::types::ParamType::U128),
            "i8" => quote!(::pyde_rust_sdk::types::ParamType::I8),
            "i16" => quote!(::pyde_rust_sdk::types::ParamType::I16),
            "i32" => quote!(::pyde_rust_sdk::types::ParamType::I32),
            "i64" => quote!(::pyde_rust_sdk::types::ParamType::I64),
            "i128" => quote!(::pyde_rust_sdk::types::ParamType::I128),
            "bool" => quote!(::pyde_rust_sdk::types::ParamType::Bool),
            "address" => quote!(::pyde_rust_sdk::types::ParamType::Address),
            "bytes" => quote!(::pyde_rust_sdk::types::ParamType::Bytes),
            "string" => quote!(::pyde_rust_sdk::types::ParamType::String),
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("unknown ABI type for ABI ParamType: {other:?}"),
                ))
            }
        },
        JsonType::Composite(JsonCompositeType::FixedBytes { len }) => {
            quote!(::pyde_rust_sdk::types::ParamType::FixedBytes(#len))
        }
        JsonType::Composite(JsonCompositeType::Custom { name }) => {
            quote!(::pyde_rust_sdk::types::ParamType::Custom(::std::string::String::from(#name)))
        }
        JsonType::Composite(_) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "composite ParamTypes not yet supported by the typed macro",
            ))
        }
    })
}

// ── Codegen ────────────────────────────────────────────────────

/// `pyde_abi!(StructName, "path/to/abi.json")`.
///
/// Generates a typed contract wrapper. See the crate-level docs for
/// the supported feature surface in v1.
#[proc_macro]
pub fn pyde_abi(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as AbiInput);

    // Resolve the ABI path relative to `CARGO_MANIFEST_DIR`. Same
    // convention as `include_bytes!` so callers don't have to think
    // about working-directory shenanigans.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let abi_path_str = parsed.abi_path.value();
    let abi_full_path = std::path::Path::new(&manifest_dir).join(&abi_path_str);
    let abi_bytes = match std::fs::read(&abi_full_path) {
        Ok(b) => b,
        Err(e) => {
            return syn::Error::new(
                parsed.abi_path.span(),
                format!("could not read ABI file {abi_full_path:?}: {e}"),
            )
            .to_compile_error()
            .into();
        }
    };
    let abi: JsonAbi = match serde_json::from_slice(&abi_bytes) {
        Ok(a) => a,
        Err(e) => {
            return syn::Error::new(
                parsed.abi_path.span(),
                format!("could not parse ABI JSON: {e}"),
            )
            .to_compile_error()
            .into();
        }
    };

    match build_contract_tokens(&parsed.struct_name, &abi) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn build_contract_tokens(struct_name: &Ident, abi: &JsonAbi) -> syn::Result<TokenStream2> {
    let mut methods = Vec::new();
    let abi_name = &abi.name;
    let abi_version = &abi.version;

    for f in &abi.functions {
        methods.push(build_function_tokens(f)?);
    }

    let event_names: Vec<&str> = abi.events.iter().map(|e| e.name.as_str()).collect();
    let function_inits = build_function_init_tokens(&abi.functions)?;

    Ok(quote! {
        /// Generated by `pyde_abi!` — typed wrapper over an
        /// on-chain Pyde contract.
        pub struct #struct_name {
            contract: ::pyde_rust_sdk::contract::Contract,
        }

        impl #struct_name {
            /// Contract name from the ABI.
            pub const NAME: &'static str = #abi_name;
            /// Contract version from the ABI.
            pub const VERSION: &'static str = #abi_version;
            /// Names of declared events.
            pub const EVENTS: &'static [&'static str] = &[#(#event_names),*];

            /// Build a typed handle from an address + provider.
            ///
            /// The ABI used at construction is the inline ABI baked
            /// into the macro invocation — pinned at compile time.
            /// For runtime ABI loading use
            /// [`::pyde_rust_sdk::contract::Contract::load_at`]
            /// instead.
            pub fn new(
                address: ::pyde_rust_sdk::Address,
                provider: ::std::sync::Arc<dyn ::pyde_rust_sdk::Provider>,
            ) -> Self {
                let abi = Self::baked_abi();
                Self {
                    contract: ::pyde_rust_sdk::contract::Contract::new(
                        address, abi, provider,
                    ),
                }
            }

            /// On-chain address.
            pub fn address(&self) -> ::pyde_rust_sdk::Address {
                self.contract.address()
            }

            /// Underlying [`Contract`] handle — useful for the
            /// dynamic surface (`call_with`, `build_tx`, …).
            pub fn contract(&self) -> &::pyde_rust_sdk::contract::Contract {
                &self.contract
            }

            /// The compile-time-baked ABI.
            fn baked_abi() -> ::pyde_rust_sdk::types::ContractAbi {
                ::pyde_rust_sdk::types::ContractAbi {
                    pyde_abi_version: ::pyde_rust_sdk::types::ContractAbi::V1_2,
                    contract_type: ::pyde_rust_sdk::types::ContractType::Contract,
                    functions: ::std::vec![ #( #function_inits ),* ],
                    state_schema_hash: [0u8; 32],
                    constructor_index: ::std::option::Option::None,
                    fallback_index: ::std::option::Option::None,
                    receive_index: ::std::option::Option::None,
                    name: ::std::string::String::from(Self::NAME),
                    version: ::std::string::String::from(Self::VERSION),
                    events: ::std::vec::Vec::new(),
                    parachain_imports: ::std::vec::Vec::new(),
                    state_schema: ::pyde_rust_sdk::types::StateSchema::empty(),
                    types: ::std::vec::Vec::new(),
                }
            }

            #(#methods)*
        }
    })
}

fn build_function_init_tokens(functions: &[JsonFunction]) -> syn::Result<Vec<TokenStream2>> {
    let mut parts = Vec::new();
    for f in functions {
        let name = &f.name;
        let attrs = f.attrs;
        let selector_bytes = selector_blake3_4(name.as_bytes());
        let s0 = selector_bytes[0];
        let s1 = selector_bytes[1];
        let s2 = selector_bytes[2];
        let s3 = selector_bytes[3];
        let mut param_parts = Vec::new();
        for p in &f.params {
            let pname = &p.name;
            let ptype = abi_type_tokens(&p.ty)?;
            param_parts.push(quote!(
                ::pyde_rust_sdk::types::ParamAbi {
                    name: ::std::string::String::from(#pname),
                    ty: #ptype,
                }
            ));
        }
        let returns = match &f.returns {
            None => quote!(::std::option::Option::None),
            Some(t) => {
                let ty_tokens = abi_type_tokens(t)?;
                quote!(::std::option::Option::Some(#ty_tokens))
            }
        };
        parts.push(quote!(
            ::pyde_rust_sdk::types::FunctionAbi {
                selector: [#s0, #s1, #s2, #s3],
                name: ::std::string::String::from(#name),
                attrs: ::pyde_rust_sdk::types::FunctionAttrs::from_bits(#attrs),
                params: ::std::vec![ #( #param_parts ),* ],
                returns: #returns,
            }
        ));
    }
    Ok(parts)
}

fn build_function_tokens(f: &JsonFunction) -> syn::Result<TokenStream2> {
    let fn_name = format_ident!("{}", f.name);
    let name_lit = &f.name;
    let is_view = f.attrs & 0x01 != 0;

    // Per-arg pieces.
    let mut arg_decls: Vec<TokenStream2> = Vec::new();
    let mut value_exprs: Vec<TokenStream2> = Vec::new();
    for p in &f.params {
        let ident = format_ident!("{}", p.name);
        let ty = rust_type_tokens(&p.ty)?;
        arg_decls.push(quote!(#ident: #ty));
        let conv = rust_to_value_expr(&p.ty, quote!(#ident))?;
        value_exprs.push(conv);
    }

    if is_view {
        // VIEW: call + decode return.
        let ret_decl = match &f.returns {
            None => quote!(()),
            Some(t) => rust_type_tokens(t)?,
        };
        let decode_return = match &f.returns {
            None => quote!(let _ = result; ::std::result::Result::Ok(())),
            Some(t) => {
                let coerce = value_to_rust_expr(t)?;
                quote!({
                    let value = result.ok_or_else(|| {
                        ::pyde_rust_sdk::SdkError::InvalidResponse(
                            ::std::string::String::from("expected return value, got ()")
                        )
                    })?;
                    let coerced = #coerce;
                    ::std::result::Result::Ok(coerced)
                })
            }
        };
        Ok(quote! {
            #[doc = concat!("Call the contract's `", #name_lit, "` view function.")]
            pub async fn #fn_name(&self #(, #arg_decls)*) -> ::pyde_rust_sdk::Result<#ret_decl> {
                let args: ::std::vec::Vec<::pyde_rust_sdk::contract::Value> = ::std::vec![
                    #(#value_exprs),*
                ];
                let result = self.contract.call(#name_lit, args).await?;
                #decode_return
            }
        })
    } else {
        // Non-view: build tx, sign, submit, return PendingTx.
        Ok(quote! {
            #[doc = concat!("Invoke the contract's `", #name_lit, "` function.")]
            pub async fn #fn_name(
                &self,
                signer: &dyn ::pyde_rust_sdk::Signer,
                #(#arg_decls,)*
                gas_limit: u64,
                value: u128,
            ) -> ::pyde_rust_sdk::Result<::pyde_rust_sdk::PendingTx> {
                let args: ::std::vec::Vec<::pyde_rust_sdk::contract::Value> = ::std::vec![
                    #(#value_exprs),*
                ];
                self.contract.send(signer, #name_lit, args, gas_limit, value).await
            }
        })
    }
}

/// Compute a 4-byte function selector — `Blake3(name)[..4]` per
/// [HOST_FN_ABI §3.7.4](https://book.pyde.network/companion/HOST_FN_ABI_SPEC#374-function-selector).
///
/// Pyde dispatches by function NAME, not by selector — the chain
/// never verifies the selector field at call time. We compute the
/// canonical value anyway so explorers and indexers that
/// cross-check `selector == Blake3(name)[..4]` against the ABI
/// see a clean match.
fn selector_blake3_4(name: &[u8]) -> [u8; 4] {
    let hash = blake3::hash(name);
    let bytes = hash.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}
