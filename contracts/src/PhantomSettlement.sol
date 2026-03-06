// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {ISettlement} from "./interfaces/ISettlement.sol";
import {FillResult} from "./types/OrderTypes.sol";

/// @title PhantomSettlement
/// @notice On-chain settlement ledger for recording and verifying fill results.
/// @dev Tracks all fills executed by the Phantom Filler system, providing
///      settlement verification, dispute flagging, and fill history.
contract PhantomSettlement is ISettlement, Ownable {
    // ─── Events ──────────────────────────────────────────────────────

    /// @notice Emitted when a recorder is authorized.
    event RecorderAdded(address indexed recorder);

    /// @notice Emitted when a recorder is deauthorized.
    event RecorderRemoved(address indexed recorder);

    // ─── Errors ──────────────────────────────────────────────────────

    /// @notice Caller is not an authorized recorder.
    error UnauthorizedRecorder();

    /// @notice The order has already been settled.
    error AlreadySettled();

    /// @notice The order has not been settled.
    error NotSettled();

    /// @notice The address is the zero address.
    error ZeroAddress();

    // ─── Types ───────────────────────────────────────────────────────

    /// @notice On-chain settlement record for a filled order.
    struct Settlement {
        /// @dev The filler address that executed the fill.
        address filler;
        /// @dev The actual input amount transferred from the swapper.
        uint256 inputAmount;
        /// @dev The actual output amount delivered to the recipient.
        uint256 outputAmount;
        /// @dev The block timestamp when the fill was recorded.
        uint256 settledAt;
        /// @dev Whether this settlement has been disputed.
        bool disputed;
    }

    // ─── State ───────────────────────────────────────────────────────

    /// @notice Authorized addresses that can record settlements.
    mapping(address => bool) public authorizedRecorders;

    /// @notice Settlement records keyed by order hash.
    mapping(bytes32 => Settlement) internal _settlements;

    /// @notice Total number of recorded settlements.
    uint256 public override totalSettlements;

    // ─── Constructor ─────────────────────────────────────────────────

    /// @notice Initializes the settlement contract with the owner.
    /// @param _owner The initial owner of the contract.
    constructor(address _owner) Ownable(_owner) {
        authorizedRecorders[_owner] = true;
        emit RecorderAdded(_owner);
    }

    // ─── Modifiers ───────────────────────────────────────────────────

    /// @dev Restricts access to authorized recorder addresses.
    modifier onlyRecorder() {
        _checkRecorder();
        _;
    }

    function _checkRecorder() internal view {
        if (!authorizedRecorders[msg.sender]) revert UnauthorizedRecorder();
    }

    // ─── Admin Functions ─────────────────────────────────────────────

    /// @notice Authorizes an address to record settlements.
    /// @param recorder The address to authorize.
    function addRecorder(address recorder) external onlyOwner {
        if (recorder == address(0)) revert ZeroAddress();
        authorizedRecorders[recorder] = true;
        emit RecorderAdded(recorder);
    }

    /// @notice Deauthorizes an address from recording settlements.
    /// @param recorder The address to deauthorize.
    function removeRecorder(address recorder) external onlyOwner {
        authorizedRecorders[recorder] = false;
        emit RecorderRemoved(recorder);
    }

    // ─── Settlement Functions ────────────────────────────────────────

    /// @inheritdoc ISettlement
    function recordFill(FillResult calldata result) external onlyRecorder {
        if (_settlements[result.orderHash].settledAt != 0) revert AlreadySettled();

        _settlements[result.orderHash] = Settlement({
            filler: result.filler,
            inputAmount: result.inputAmount,
            outputAmount: result.outputAmount,
            settledAt: block.timestamp,
            disputed: false
        });

        unchecked {
            ++totalSettlements;
        }

        emit FillSettled(result.orderHash, result.filler, result.inputAmount, result.outputAmount);
    }

    /// @inheritdoc ISettlement
    function getSettlement(bytes32 orderHash)
        external
        view
        returns (bool settled, address filler, uint256 settledAt)
    {
        Settlement storage s = _settlements[orderHash];
        settled = s.settledAt != 0;
        filler = s.filler;
        settledAt = s.settledAt;
    }

    /// @inheritdoc ISettlement
    function isFilled(bytes32 orderHash) external view returns (bool) {
        return _settlements[orderHash].settledAt != 0;
    }

    /// @notice Returns the full settlement record for an order.
    /// @param orderHash The order hash to query.
    /// @return The settlement record.
    function getFullSettlement(bytes32 orderHash) external view returns (Settlement memory) {
        if (_settlements[orderHash].settledAt == 0) revert NotSettled();
        return _settlements[orderHash];
    }

    // ─── Dispute Functions ───────────────────────────────────────────

    /// @notice Flags a settlement as disputed.
    /// @dev Only the owner can dispute settlements. Disputed settlements
    ///      require off-chain resolution.
    /// @param orderHash The order hash to dispute.
    function disputeFill(bytes32 orderHash) external onlyOwner {
        if (_settlements[orderHash].settledAt == 0) revert NotSettled();
        _settlements[orderHash].disputed = true;
        emit FillDisputed(orderHash, msg.sender);
    }

    /// @notice Resolves a disputed settlement.
    /// @param orderHash The order hash to resolve.
    function resolveDispute(bytes32 orderHash) external onlyOwner {
        if (_settlements[orderHash].settledAt == 0) revert NotSettled();
        _settlements[orderHash].disputed = false;
    }

    /// @notice Checks whether a settlement is disputed.
    /// @param orderHash The order hash to check.
    /// @return True if the settlement is disputed.
    function isDisputed(bytes32 orderHash) external view returns (bool) {
        return _settlements[orderHash].disputed;
    }
}
