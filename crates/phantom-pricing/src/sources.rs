//! On-chain price source adapters.
//!
//! Implements the `PriceSource` trait for major DEXs: Uniswap V3, Curve, and
//! Balancer V2. Each adapter queries the respective DEX's on-chain quoter or
//! pool contracts to obtain real-time token prices.
//!
//! Uses `SolCall` encoding with raw `provider.call()` to support
//! trait-object providers (`Arc<DynProvider>`).

use std::sync::Arc;

use alloy::primitives::{Address, Bytes, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::sol_types::SolCall;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use phantom_chain::provider::DynProvider;
use phantom_common::error::PricingError;
use phantom_common::traits::PriceSource;
use phantom_common::types::{ChainId, Token};
use tracing::debug;

use crate::dex_abi::{self, fee_tiers};

/// A price quote from a specific source with metadata.
#[derive(Debug, Clone)]
pub struct PriceQuote {
    /// Amount of quote token received per unit of base token.
    pub price: U256,
    /// Name of the price source.
    pub source: String,
    /// Timestamp when the quote was obtained.
    pub timestamp: DateTime<Utc>,
    /// Estimated gas cost of the underlying swap (if available).
    pub gas_estimate: Option<U256>,
}

impl PriceQuote {
    /// Creates a new price quote.
    pub fn new(price: U256, source: impl Into<String>) -> Self {
        Self {
            price,
            source: source.into(),
            timestamp: Utc::now(),
            gas_estimate: None,
        }
    }

    /// Sets the gas estimate.
    pub fn with_gas_estimate(mut self, gas: U256) -> Self {
        self.gas_estimate = Some(gas);
        self
    }

    /// Returns the age of this quote in seconds.
    pub fn age_seconds(&self) -> u64 {
        let elapsed = Utc::now() - self.timestamp;
        elapsed.num_seconds().unsigned_abs()
    }

    /// Returns true if the quote is older than the given number of seconds.
    pub fn is_stale(&self, max_age_seconds: u64) -> bool {
        self.age_seconds() > max_age_seconds
    }
}

/// Execute an `eth_call` against a contract using encoded calldata.
///
/// Encodes calldata via `SolCall`, sends via the provider's raw request,
/// and returns the raw response bytes for decoding.
async fn eth_call(
    provider: &Arc<DynProvider>,
    to: Address,
    calldata: Vec<u8>,
) -> Result<Bytes, PricingError> {
    let tx = TransactionRequest::default().to(to).input(calldata.into());

    provider
        .call(tx)
        .await
        .map_err(|e| PricingError::SourceUnavailable(format!("eth_call failed: {e}")))
}

/// Uniswap V3 price source using the QuoterV2 contract.
///
/// Queries the Uniswap V3 QuoterV2 to get exact output amounts for a given
/// input. Tries multiple fee tiers and returns the best price.
pub struct UniswapV3Source {
    provider: Arc<DynProvider>,
    quoter_address: Address,
    chain_id: ChainId,
}

impl UniswapV3Source {
    /// Creates a new Uniswap V3 price source.
    pub fn new(provider: Arc<DynProvider>, quoter_address: Address, chain_id: ChainId) -> Self {
        Self {
            provider,
            quoter_address,
            chain_id,
        }
    }

    /// Creates a source for a known chain (uses well-known quoter address).
    pub fn for_chain(provider: Arc<DynProvider>, chain_id: ChainId) -> Option<Self> {
        dex_abi::addresses::uniswap_v3_quoter(chain_id.as_u64()).map(|addr| Self {
            provider,
            quoter_address: addr,
            chain_id,
        })
    }

    /// Queries the quoter for a specific fee tier.
    async fn quote_single(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        fee: u32,
    ) -> Result<PriceQuote, PricingError> {
        let call = dex_abi::IQuoterV2::quoteExactInputSingleCall {
            params: dex_abi::IQuoterV2::QuoteExactInputSingleParams {
                tokenIn: token_in,
                tokenOut: token_out,
                amountIn: amount_in,
                fee: fee
                    .try_into()
                    .map_err(|_| PricingError::SourceUnavailable("invalid fee tier".to_string()))?,
                sqrtPriceLimitX96: Default::default(),
            },
        };

        let result_bytes = eth_call(&self.provider, self.quoter_address, call.abi_encode()).await?;

        let decoded =
            dex_abi::IQuoterV2::quoteExactInputSingleCall::abi_decode_returns(&result_bytes)
                .map_err(|e| {
                    PricingError::SourceUnavailable(format!(
                        "failed to decode uniswap v3 quote: {e}"
                    ))
                })?;

        let quote =
            PriceQuote::new(decoded.amountOut, "uniswap_v3").with_gas_estimate(decoded.gasEstimate);

        Ok(quote)
    }

    /// Queries all fee tiers and returns the best (highest output) quote.
    async fn best_quote(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<PriceQuote, PricingError> {
        let mut best: Option<PriceQuote> = None;

        for &fee in &fee_tiers::ALL_FEE_TIERS {
            match self.quote_single(token_in, token_out, amount_in, fee).await {
                Ok(quote) => {
                    if best.as_ref().is_none_or(|b| quote.price > b.price) {
                        best = Some(quote);
                    }
                }
                Err(e) => {
                    debug!(fee, error = %e, "fee tier quote failed, trying next");
                }
            }
        }

        best.ok_or_else(|| PricingError::NoPriceAvailable {
            token_a: format!("{token_in}"),
            token_b: format!("{token_out}"),
            chain_id: self.chain_id,
        })
    }
}

#[async_trait]
impl PriceSource for UniswapV3Source {
    fn name(&self) -> &str {
        "uniswap_v3"
    }

    async fn get_price(
        &self,
        base: &Token,
        quote: &Token,
        chain_id: ChainId,
    ) -> Result<U256, PricingError> {
        if chain_id != self.chain_id {
            return Err(PricingError::SourceUnavailable(format!(
                "uniswap v3 source configured for {:?}, requested {:?}",
                self.chain_id, chain_id
            )));
        }

        let one_unit = U256::from(10u64).pow(U256::from(base.decimals));
        let result = self
            .best_quote(base.address, quote.address, one_unit)
            .await?;
        Ok(result.price)
    }
}

/// Curve StableSwap price source.
///
/// Queries a Curve pool's `get_dy` function to get the exchange rate
/// between two tokens in the pool.
pub struct CurveSource {
    provider: Arc<DynProvider>,
    pool_address: Address,
    chain_id: ChainId,
    /// Mapping of token address to pool coin index.
    coin_indices: Vec<(Address, i128)>,
}

impl CurveSource {
    /// Creates a new Curve price source for a specific pool.
    pub fn new(
        provider: Arc<DynProvider>,
        pool_address: Address,
        chain_id: ChainId,
        coin_indices: Vec<(Address, i128)>,
    ) -> Self {
        Self {
            provider,
            pool_address,
            chain_id,
            coin_indices,
        }
    }

    /// Finds the coin index for a token address.
    fn coin_index(&self, token: Address) -> Option<i128> {
        self.coin_indices
            .iter()
            .find(|(addr, _)| *addr == token)
            .map(|(_, idx)| *idx)
    }
}

#[async_trait]
impl PriceSource for CurveSource {
    fn name(&self) -> &str {
        "curve"
    }

    async fn get_price(
        &self,
        base: &Token,
        quote: &Token,
        chain_id: ChainId,
    ) -> Result<U256, PricingError> {
        if chain_id != self.chain_id {
            return Err(PricingError::SourceUnavailable(format!(
                "curve source configured for {:?}, requested {:?}",
                self.chain_id, chain_id
            )));
        }

        let i = self
            .coin_index(base.address)
            .ok_or_else(|| PricingError::NoPriceAvailable {
                token_a: base.symbol.clone(),
                token_b: quote.symbol.clone(),
                chain_id,
            })?;

        let j = self
            .coin_index(quote.address)
            .ok_or_else(|| PricingError::NoPriceAvailable {
                token_a: base.symbol.clone(),
                token_b: quote.symbol.clone(),
                chain_id,
            })?;

        let one_unit = U256::from(10u64).pow(U256::from(base.decimals));
        let call = dex_abi::ICurvePool::get_dyCall { i, j, dx: one_unit };

        let result_bytes = eth_call(&self.provider, self.pool_address, call.abi_encode()).await?;

        let decoded =
            dex_abi::ICurvePool::get_dyCall::abi_decode_returns(&result_bytes).map_err(|e| {
                PricingError::SourceUnavailable(format!("failed to decode curve get_dy: {e}"))
            })?;

        Ok(decoded)
    }
}

/// Balancer V2 price source using the Vault's query interface.
///
/// Queries the Balancer V2 Vault to simulate a swap and get the output amount.
pub struct BalancerSource {
    provider: Arc<DynProvider>,
    vault_address: Address,
    chain_id: ChainId,
    /// Pool ID for the token pair.
    pool_id: [u8; 32],
}

impl BalancerSource {
    /// Creates a new Balancer V2 price source.
    pub fn new(
        provider: Arc<DynProvider>,
        vault_address: Address,
        chain_id: ChainId,
        pool_id: [u8; 32],
    ) -> Self {
        Self {
            provider,
            vault_address,
            chain_id,
            pool_id,
        }
    }
}

#[async_trait]
impl PriceSource for BalancerSource {
    fn name(&self) -> &str {
        "balancer_v2"
    }

    async fn get_price(
        &self,
        base: &Token,
        quote: &Token,
        chain_id: ChainId,
    ) -> Result<U256, PricingError> {
        if chain_id != self.chain_id {
            return Err(PricingError::SourceUnavailable(format!(
                "balancer source configured for {:?}, requested {:?}",
                self.chain_id, chain_id
            )));
        }

        let one_unit = U256::from(10u64).pow(U256::from(base.decimals));

        let call = dex_abi::IBalancerVault::querySwapCall {
            singleSwap: dex_abi::IBalancerVault::SingleSwap {
                poolId: self.pool_id.into(),
                kind: dex_abi::IBalancerVault::SwapKind::GIVEN_IN,
                assetIn: base.address,
                assetOut: quote.address,
                amount: one_unit,
                userData: Default::default(),
            },
            funds: dex_abi::IBalancerVault::FundManagement {
                sender: Address::ZERO,
                fromInternalBalance: false,
                recipient: Address::ZERO,
                toInternalBalance: false,
            },
        };

        let result_bytes = eth_call(&self.provider, self.vault_address, call.abi_encode()).await?;

        let decoded = dex_abi::IBalancerVault::querySwapCall::abi_decode_returns(&result_bytes)
            .map_err(|e| {
                PricingError::SourceUnavailable(format!("failed to decode balancer querySwap: {e}"))
            })?;

        Ok(decoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_quote_creation() {
        let quote = PriceQuote::new(U256::from(1000u64), "test_source");
        assert_eq!(quote.price, U256::from(1000u64));
        assert_eq!(quote.source, "test_source");
        assert!(quote.gas_estimate.is_none());
    }

    #[test]
    fn price_quote_with_gas() {
        let quote =
            PriceQuote::new(U256::from(1000u64), "test").with_gas_estimate(U256::from(21000u64));
        assert_eq!(quote.gas_estimate, Some(U256::from(21000u64)));
    }

    #[test]
    fn price_quote_staleness() {
        let quote = PriceQuote::new(U256::from(1000u64), "test");
        assert!(!quote.is_stale(60));
        assert_eq!(quote.age_seconds(), 0);
    }

    #[test]
    fn uniswap_v3_source_name() {
        use alloy::providers::ProviderBuilder;
        let provider: Arc<DynProvider> =
            Arc::new(ProviderBuilder::new().connect_http("http://localhost:8545".parse().unwrap()));
        let source = UniswapV3Source::new(
            provider,
            dex_abi::addresses::UNISWAP_V3_QUOTER_MAINNET,
            ChainId::Ethereum,
        );
        assert_eq!(source.name(), "uniswap_v3");
    }

    #[test]
    fn uniswap_v3_for_chain() {
        use alloy::providers::ProviderBuilder;
        let provider: Arc<DynProvider> =
            Arc::new(ProviderBuilder::new().connect_http("http://localhost:8545".parse().unwrap()));
        let source = UniswapV3Source::for_chain(Arc::clone(&provider), ChainId::Ethereum);
        assert!(source.is_some());

        let source = UniswapV3Source::for_chain(provider, ChainId::Optimism);
        assert!(source.is_none());
    }

    #[test]
    fn curve_source_coin_index() {
        use alloy::providers::ProviderBuilder;
        let provider: Arc<DynProvider> =
            Arc::new(ProviderBuilder::new().connect_http("http://localhost:8545".parse().unwrap()));

        let dai = crate::tokens::DAI_MAINNET.address;
        let usdc = crate::tokens::USDC_MAINNET.address;
        let usdt = crate::tokens::USDT_MAINNET.address;

        let source = CurveSource::new(
            provider,
            dex_abi::addresses::CURVE_3POOL_MAINNET,
            ChainId::Ethereum,
            vec![(dai, 0), (usdc, 1), (usdt, 2)],
        );

        assert_eq!(source.coin_index(dai), Some(0));
        assert_eq!(source.coin_index(usdc), Some(1));
        assert_eq!(source.coin_index(usdt), Some(2));
        assert_eq!(source.coin_index(Address::ZERO), None);
        assert_eq!(source.name(), "curve");
    }

    #[test]
    fn balancer_source_name() {
        use alloy::providers::ProviderBuilder;
        let provider: Arc<DynProvider> =
            Arc::new(ProviderBuilder::new().connect_http("http://localhost:8545".parse().unwrap()));
        let source = BalancerSource::new(
            provider,
            dex_abi::addresses::BALANCER_VAULT,
            ChainId::Ethereum,
            [0u8; 32],
        );
        assert_eq!(source.name(), "balancer_v2");
    }

    #[test]
    fn uniswap_v3_call_encoding() {
        let call = dex_abi::IQuoterV2::quoteExactInputSingleCall {
            params: dex_abi::IQuoterV2::QuoteExactInputSingleParams {
                tokenIn: Address::with_last_byte(0xAA),
                tokenOut: Address::with_last_byte(0xBB),
                amountIn: U256::from(1_000_000u64),
                fee: 3000u32.try_into().unwrap(),
                sqrtPriceLimitX96: Default::default(),
            },
        };
        let encoded = call.abi_encode();
        assert!(encoded.len() > 4);
    }

    #[test]
    fn curve_call_encoding() {
        let call = dex_abi::ICurvePool::get_dyCall {
            i: 0,
            j: 1,
            dx: U256::from(1_000_000u64),
        };
        let encoded = call.abi_encode();
        assert!(encoded.len() > 4);
    }

    #[test]
    fn balancer_call_encoding() {
        let call = dex_abi::IBalancerVault::querySwapCall {
            singleSwap: dex_abi::IBalancerVault::SingleSwap {
                poolId: [0u8; 32].into(),
                kind: dex_abi::IBalancerVault::SwapKind::GIVEN_IN,
                assetIn: Address::with_last_byte(0xAA),
                assetOut: Address::with_last_byte(0xBB),
                amount: U256::from(1_000_000u64),
                userData: Default::default(),
            },
            funds: dex_abi::IBalancerVault::FundManagement {
                sender: Address::ZERO,
                fromInternalBalance: false,
                recipient: Address::ZERO,
                toInternalBalance: false,
            },
        };
        let encoded = call.abi_encode();
        assert!(encoded.len() > 4);
    }
}
