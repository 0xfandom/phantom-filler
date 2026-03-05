//! Well-known token addresses for supported chains.
//!
//! Provides canonical token address lookups used by price source adapters
//! for constructing on-chain price queries.

use alloy::primitives::{address, Address};
use phantom_common::types::ChainId;

/// Well-known token on a specific chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownToken {
    /// Token contract address.
    pub address: Address,
    /// Token symbol.
    pub symbol: &'static str,
    /// Decimal places.
    pub decimals: u8,
}

// ── Ethereum Mainnet tokens ──────────────────────────────────────────

/// WETH on Ethereum.
pub const WETH_MAINNET: KnownToken = KnownToken {
    address: address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
    symbol: "WETH",
    decimals: 18,
};

/// USDC on Ethereum.
pub const USDC_MAINNET: KnownToken = KnownToken {
    address: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
    symbol: "USDC",
    decimals: 6,
};

/// USDT on Ethereum.
pub const USDT_MAINNET: KnownToken = KnownToken {
    address: address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
    symbol: "USDT",
    decimals: 6,
};

/// DAI on Ethereum.
pub const DAI_MAINNET: KnownToken = KnownToken {
    address: address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
    symbol: "DAI",
    decimals: 18,
};

/// WBTC on Ethereum.
pub const WBTC_MAINNET: KnownToken = KnownToken {
    address: address!("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
    symbol: "WBTC",
    decimals: 8,
};

// ── Arbitrum tokens ──────────────────────────────────────────────────

/// WETH on Arbitrum.
pub const WETH_ARBITRUM: KnownToken = KnownToken {
    address: address!("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1"),
    symbol: "WETH",
    decimals: 18,
};

/// USDC on Arbitrum (native).
pub const USDC_ARBITRUM: KnownToken = KnownToken {
    address: address!("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
    symbol: "USDC",
    decimals: 6,
};

/// Returns well-known tokens for a chain.
pub fn known_tokens(chain_id: ChainId) -> &'static [KnownToken] {
    match chain_id {
        ChainId::Ethereum => &MAINNET_TOKENS,
        ChainId::Arbitrum => &ARBITRUM_TOKENS,
        _ => &[],
    }
}

/// All well-known mainnet tokens.
static MAINNET_TOKENS: [KnownToken; 5] = [
    WETH_MAINNET,
    USDC_MAINNET,
    USDT_MAINNET,
    DAI_MAINNET,
    WBTC_MAINNET,
];

/// All well-known Arbitrum tokens.
static ARBITRUM_TOKENS: [KnownToken; 2] = [WETH_ARBITRUM, USDC_ARBITRUM];

/// Finds a known token by address on a given chain.
pub fn find_by_address(chain_id: ChainId, address: Address) -> Option<&'static KnownToken> {
    known_tokens(chain_id).iter().find(|t| t.address == address)
}

/// Finds a known token by symbol on a given chain.
pub fn find_by_symbol(chain_id: ChainId, symbol: &str) -> Option<&'static KnownToken> {
    known_tokens(chain_id).iter().find(|t| t.symbol == symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_tokens_exist() {
        let tokens = known_tokens(ChainId::Ethereum);
        assert_eq!(tokens.len(), 5);
    }

    #[test]
    fn arbitrum_tokens_exist() {
        let tokens = known_tokens(ChainId::Arbitrum);
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn unknown_chain_empty() {
        let tokens = known_tokens(ChainId::Optimism);
        assert!(tokens.is_empty());
    }

    #[test]
    fn find_weth_by_address() {
        let token = find_by_address(ChainId::Ethereum, WETH_MAINNET.address);
        assert!(token.is_some());
        assert_eq!(token.unwrap().symbol, "WETH");
    }

    #[test]
    fn find_usdc_by_symbol() {
        let token = find_by_symbol(ChainId::Ethereum, "USDC");
        assert!(token.is_some());
        assert_eq!(token.unwrap().decimals, 6);
    }

    #[test]
    fn find_missing_token() {
        let token = find_by_symbol(ChainId::Ethereum, "SHIB");
        assert!(token.is_none());
    }

    #[test]
    fn token_decimals_correct() {
        assert_eq!(WETH_MAINNET.decimals, 18);
        assert_eq!(USDC_MAINNET.decimals, 6);
        assert_eq!(USDT_MAINNET.decimals, 6);
        assert_eq!(DAI_MAINNET.decimals, 18);
        assert_eq!(WBTC_MAINNET.decimals, 8);
    }

    #[test]
    fn arbitrum_weth_different_address() {
        assert_ne!(WETH_MAINNET.address, WETH_ARBITRUM.address);
    }
}
