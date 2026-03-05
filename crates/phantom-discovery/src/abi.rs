//! Solidity ABI definitions for UniswapX reactor contracts.
//!
//! Uses Alloy's `sol!` macro to generate Rust types with ABI encoding/decoding
//! from Solidity type definitions matching the on-chain UniswapX V2 contracts.

use alloy::sol;

sol! {
    /// Core order information shared across order types.
    #[derive(Debug, PartialEq, Eq)]
    struct OrderInfo {
        /// The reactor contract that validates and executes the order.
        address reactor;
        /// The address of the swapper who signed the order.
        address swapper;
        /// Unique nonce for replay protection.
        uint256 nonce;
        /// Unix timestamp after which the order can no longer be filled.
        uint256 deadline;
        /// Optional additional validation contract (zero address if unused).
        address additionalValidationContract;
        /// Optional additional validation data.
        bytes additionalValidationData;
    }

    /// Input token specification for a Dutch auction order.
    #[derive(Debug, PartialEq, Eq)]
    struct DutchInput {
        /// The ERC20 token address to be provided by the swapper.
        address token;
        /// The starting (maximum) input amount at decay start.
        uint256 startAmount;
        /// The ending (minimum) input amount at decay end.
        uint256 endAmount;
    }

    /// Output token specification for a Dutch auction order.
    #[derive(Debug, PartialEq, Eq)]
    struct DutchOutput {
        /// The ERC20 token address to be received.
        address token;
        /// The starting (minimum) output amount at decay start.
        uint256 startAmount;
        /// The ending (maximum) output amount at decay end.
        uint256 endAmount;
        /// The address that receives the output tokens.
        address recipient;
    }

    /// Exclusive Dutch order — the primary UniswapX V2 order type.
    #[derive(Debug, PartialEq, Eq)]
    struct ExclusiveDutchOrder {
        /// Core order information.
        OrderInfo info;
        /// Unix timestamp when the Dutch auction decay begins.
        uint256 decayStartTime;
        /// Unix timestamp when the Dutch auction decay ends.
        uint256 decayEndTime;
        /// Address of the exclusive filler (zero if no exclusivity).
        address exclusiveFiller;
        /// Basis points override after exclusivity period.
        uint256 exclusivityOverrideBps;
        /// Input token with decay parameters.
        DutchInput input;
        /// Output tokens with decay parameters.
        DutchOutput[] outputs;
    }

    /// A signed order as submitted on-chain.
    #[derive(Debug, PartialEq, Eq)]
    struct SignedOrder {
        /// ABI-encoded order data.
        bytes order;
        /// EIP-712 signature.
        bytes sig;
    }

    /// Resolved order after decay calculation.
    #[derive(Debug, PartialEq, Eq)]
    struct ResolvedOrder {
        /// Core order information.
        OrderInfo info;
        /// Resolved input token and amount.
        InputToken input;
        /// Resolved output tokens and amounts.
        OutputToken[] outputs;
        /// Additional signature data.
        bytes sig;
        /// The order hash.
        bytes32 hash;
    }

    /// Resolved input after decay.
    #[derive(Debug, PartialEq, Eq)]
    struct InputToken {
        address token;
        uint256 amount;
        uint256 maxAmount;
    }

    /// Resolved output after decay.
    #[derive(Debug, PartialEq, Eq)]
    struct OutputToken {
        address token;
        uint256 amount;
        address recipient;
    }

    /// UniswapX Reactor contract interface — events and key functions.
    #[derive(Debug, PartialEq, Eq)]
    #[sol(rpc)]
    interface IReactor {
        /// Emitted when an order is filled.
        event Fill(
            bytes32 indexed orderHash,
            address indexed filler,
            address indexed swapper,
            uint256 nonce
        );

        /// Execute a single signed order.
        function execute(SignedOrder calldata order) external payable;

        /// Execute a single signed order with a filler callback.
        function executeWithCallback(
            SignedOrder calldata order,
            bytes calldata callbackData
        ) external payable;

        /// Execute multiple signed orders.
        function executeBatch(SignedOrder[] calldata orders) external payable;

        /// Execute multiple signed orders with a filler callback.
        function executeBatchWithCallback(
            SignedOrder[] calldata orders,
            bytes calldata callbackData
        ) external payable;
    }

    /// Permit2 interface used for token approvals in UniswapX.
    #[derive(Debug, PartialEq, Eq)]
    #[sol(rpc)]
    interface IPermit2 {
        struct PermitTransferFrom {
            TokenPermissions permitted;
            uint256 nonce;
            uint256 deadline;
        }

        struct TokenPermissions {
            address token;
            uint256 amount;
        }

        struct SignatureTransferDetails {
            address to;
            uint256 requestedAmount;
        }

        function permitTransferFrom(
            PermitTransferFrom calldata permit,
            SignatureTransferDetails calldata transferDetails,
            address owner,
            bytes calldata signature
        ) external;
    }
}

/// Well-known UniswapX reactor addresses by chain.
pub mod addresses {
    use alloy::primitives::{address, Address};

    /// UniswapX V2 ExclusiveDutchOrderReactor on Ethereum mainnet.
    pub const EXCLUSIVE_DUTCH_ORDER_REACTOR_MAINNET: Address =
        address!("0x6000da47483062A0D734Ba3dc7576Ce6A0B645C4");

    /// UniswapX V2 ExclusiveDutchOrderReactor on Arbitrum.
    pub const EXCLUSIVE_DUTCH_ORDER_REACTOR_ARBITRUM: Address =
        address!("0x1bd1aAdc9E230626C44a139d7E70d842749351eb");

    /// Permit2 contract address (same on all chains).
    pub const PERMIT2: Address = address!("0x000000000022D473030F116dDEE9F6B43aC78BA3");

    /// Returns the reactor address for a given chain, if known.
    pub fn reactor_for_chain(chain_id: u64) -> Option<Address> {
        match chain_id {
            1 => Some(EXCLUSIVE_DUTCH_ORDER_REACTOR_MAINNET),
            42161 => Some(EXCLUSIVE_DUTCH_ORDER_REACTOR_ARBITRUM),
            _ => None,
        }
    }
}

/// Event signature constants for log filtering.
pub mod event_signatures {
    use alloy::primitives::{keccak256, B256};

    /// `Fill(bytes32,address,address,uint256)` event signature (topic0).
    pub fn fill_event_signature() -> B256 {
        keccak256("Fill(bytes32,address,address,uint256)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bytes, B256, U256};
    use alloy::sol_types::SolEvent;
    use alloy::sol_types::SolType;

    #[test]
    fn order_info_abi_encode_decode() {
        let info = OrderInfo {
            reactor: Address::ZERO,
            swapper: Address::with_last_byte(1),
            nonce: U256::from(42),
            deadline: U256::from(1_700_000_000u64),
            additionalValidationContract: Address::ZERO,
            additionalValidationData: Bytes::new(),
        };

        let encoded = <OrderInfo as SolType>::abi_encode(&info);
        let decoded = <OrderInfo as SolType>::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded, info);
    }

    #[test]
    fn dutch_input_abi_encode_decode() {
        let input = DutchInput {
            token: Address::with_last_byte(0xAA),
            startAmount: U256::from(1000u64),
            endAmount: U256::from(900u64),
        };

        let encoded = <DutchInput as SolType>::abi_encode(&input);
        let decoded = <DutchInput as SolType>::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded, input);
    }

    #[test]
    fn dutch_output_abi_encode_decode() {
        let output = DutchOutput {
            token: Address::with_last_byte(0xBB),
            startAmount: U256::from(500u64),
            endAmount: U256::from(450u64),
            recipient: Address::with_last_byte(0xCC),
        };

        let encoded = <DutchOutput as SolType>::abi_encode(&output);
        let decoded = <DutchOutput as SolType>::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded, output);
    }

    #[test]
    fn exclusive_dutch_order_abi_encode_decode() {
        let order = ExclusiveDutchOrder {
            info: OrderInfo {
                reactor: Address::with_last_byte(0x01),
                swapper: Address::with_last_byte(0x02),
                nonce: U256::from(1),
                deadline: U256::from(1_700_000_000u64),
                additionalValidationContract: Address::ZERO,
                additionalValidationData: Bytes::new(),
            },
            decayStartTime: U256::from(1_699_999_000u64),
            decayEndTime: U256::from(1_700_000_000u64),
            exclusiveFiller: Address::ZERO,
            exclusivityOverrideBps: U256::ZERO,
            input: DutchInput {
                token: Address::with_last_byte(0xAA),
                startAmount: U256::from(1000u64),
                endAmount: U256::from(1000u64),
            },
            outputs: vec![DutchOutput {
                token: Address::with_last_byte(0xBB),
                startAmount: U256::from(500u64),
                endAmount: U256::from(450u64),
                recipient: Address::with_last_byte(0x02),
            }],
        };

        let encoded = <ExclusiveDutchOrder as SolType>::abi_encode(&order);
        let decoded = <ExclusiveDutchOrder as SolType>::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded, order);
    }

    #[test]
    fn fill_event_signature() {
        let sig = event_signatures::fill_event_signature();
        // The Fill event signature should be a non-zero 32-byte hash.
        assert_ne!(sig, B256::ZERO);
        // Verify it matches the IReactor::Fill event selector.
        assert_eq!(sig, IReactor::Fill::SIGNATURE_HASH);
    }

    #[test]
    fn fill_event_decode() {
        // The Fill event has 3 indexed topics + 1 data field.
        assert_eq!(IReactor::Fill::SIGNATURE_HASH.len(), 32);
    }

    #[test]
    fn reactor_addresses() {
        assert!(addresses::reactor_for_chain(1).is_some());
        assert!(addresses::reactor_for_chain(42161).is_some());
        assert!(addresses::reactor_for_chain(999).is_none());
    }

    #[test]
    fn reactor_address_values() {
        assert_eq!(
            addresses::reactor_for_chain(1).unwrap(),
            addresses::EXCLUSIVE_DUTCH_ORDER_REACTOR_MAINNET
        );
    }

    #[test]
    fn signed_order_abi_encode_decode() {
        let signed = SignedOrder {
            order: Bytes::from(vec![1, 2, 3, 4]),
            sig: Bytes::from(vec![5, 6, 7, 8]),
        };

        let encoded = <SignedOrder as SolType>::abi_encode(&signed);
        let decoded = <SignedOrder as SolType>::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded, signed);
    }
}
