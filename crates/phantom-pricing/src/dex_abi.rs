//! Solidity ABI definitions for DEX price query contracts.
//!
//! Provides generated Rust types for interacting with Uniswap V3 Quoter,
//! Curve StableSwap pools, and Balancer V2 Vault contracts.
//! Uses `SolCall` encoding/decoding with raw `eth_call` to support
//! trait-object providers.

use alloy::sol;

sol! {
    /// Uniswap V3 QuoterV2 interface for price quotes.
    #[derive(Debug, PartialEq, Eq)]
    interface IQuoterV2 {
        struct QuoteExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint256 amountIn;
            uint24 fee;
            uint160 sqrtPriceLimitX96;
        }

        /// Returns the amount out for a given exact input swap without executing it.
        function quoteExactInputSingle(
            QuoteExactInputSingleParams memory params
        ) external returns (
            uint256 amountOut,
            uint160 sqrtPriceX96After,
            uint32 initializedTicksCrossed,
            uint256 gasEstimate
        );
    }

    /// Curve StableSwap pool interface for price queries.
    #[derive(Debug, PartialEq, Eq)]
    interface ICurvePool {
        /// Returns the amount of coin `j` received for swapping `dx` of coin `i`.
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);

        /// Returns the address of coin at index `i`.
        function coins(uint256 i) external view returns (address);
    }

    /// Balancer V2 Vault interface for price queries.
    #[derive(Debug, PartialEq, Eq)]
    interface IBalancerVault {
        enum SwapKind {
            GIVEN_IN,
            GIVEN_OUT
        }

        struct SingleSwap {
            bytes32 poolId;
            SwapKind kind;
            address assetIn;
            address assetOut;
            uint256 amount;
            bytes userData;
        }

        struct FundManagement {
            address sender;
            bool fromInternalBalance;
            address payable recipient;
            bool toInternalBalance;
        }

        /// Simulates a single swap and returns the output amount.
        function querySwap(
            SingleSwap memory singleSwap,
            FundManagement memory funds
        ) external returns (uint256);
    }
}

/// Well-known DEX contract addresses by chain.
pub mod addresses {
    use alloy::primitives::{address, Address};

    /// Uniswap V3 QuoterV2 on Ethereum mainnet.
    pub const UNISWAP_V3_QUOTER_MAINNET: Address =
        address!("0x61fFE014bA17989E743c5F6cB21bF9697530B21e");

    /// Uniswap V3 QuoterV2 on Arbitrum.
    pub const UNISWAP_V3_QUOTER_ARBITRUM: Address =
        address!("0x61fFE014bA17989E743c5F6cB21bF9697530B21e");

    /// Curve 3pool on Ethereum mainnet.
    pub const CURVE_3POOL_MAINNET: Address = address!("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7");

    /// Balancer V2 Vault (same on all chains).
    pub const BALANCER_VAULT: Address = address!("0xBA12222222228d8Ba445958a75a0704d566BF2C8");

    /// Returns the Uniswap V3 QuoterV2 address for a given chain, if known.
    pub fn uniswap_v3_quoter(chain_id: u64) -> Option<Address> {
        match chain_id {
            1 => Some(UNISWAP_V3_QUOTER_MAINNET),
            42161 => Some(UNISWAP_V3_QUOTER_ARBITRUM),
            _ => None,
        }
    }
}

/// Common Uniswap V3 fee tiers.
pub mod fee_tiers {
    /// 0.01% fee tier (1 basis point) — stablecoin pairs.
    pub const FEE_100: u32 = 100;
    /// 0.05% fee tier (5 basis points) — stable-like pairs.
    pub const FEE_500: u32 = 500;
    /// 0.3% fee tier (30 basis points) — standard pairs.
    pub const FEE_3000: u32 = 3000;
    /// 1% fee tier (100 basis points) — exotic pairs.
    pub const FEE_10000: u32 = 10000;

    /// All standard fee tiers in ascending order.
    pub const ALL_FEE_TIERS: [u32; 4] = [FEE_100, FEE_500, FEE_3000, FEE_10000];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniswap_v3_quoter_mainnet() {
        assert!(addresses::uniswap_v3_quoter(1).is_some());
        assert_eq!(
            addresses::uniswap_v3_quoter(1).unwrap(),
            addresses::UNISWAP_V3_QUOTER_MAINNET
        );
    }

    #[test]
    fn uniswap_v3_quoter_unknown_chain() {
        assert!(addresses::uniswap_v3_quoter(999).is_none());
    }

    #[test]
    fn fee_tiers_ascending() {
        let tiers = fee_tiers::ALL_FEE_TIERS;
        for i in 1..tiers.len() {
            assert!(tiers[i] > tiers[i - 1]);
        }
    }

    #[test]
    fn fee_tier_values() {
        assert_eq!(fee_tiers::FEE_100, 100);
        assert_eq!(fee_tiers::FEE_500, 500);
        assert_eq!(fee_tiers::FEE_3000, 3000);
        assert_eq!(fee_tiers::FEE_10000, 10000);
    }
}
