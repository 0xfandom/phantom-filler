// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IReactor} from "../../src/interfaces/IReactor.sol";
import {IFiller} from "../../src/interfaces/IFiller.sol";
import {SignedOrder, ResolvedOrder, InputToken, OutputToken} from "../../src/types/OrderTypes.sol";

/// @title MockReactor
/// @notice Minimal reactor mock for testing filler interactions.
/// @dev Tracks calls and optionally invokes the filler callback.
contract MockReactor is IReactor {
    uint256 public executeCallCount;
    uint256 public executeBatchCallCount;
    uint256 public executeWithCallbackCallCount;

    bool public shouldCallback;

    function setCallback(bool _shouldCallback) external {
        shouldCallback = _shouldCallback;
    }

    function execute(SignedOrder calldata, bytes calldata) external override {
        ++executeCallCount;
    }

    function executeBatch(SignedOrder[] calldata, bytes calldata) external override {
        ++executeBatchCallCount;
    }

    function executeWithCallback(SignedOrder calldata, bytes calldata fillerData) external override {
        ++executeWithCallbackCallCount;

        if (shouldCallback) {
            ResolvedOrder[] memory orders = new ResolvedOrder[](0);
            IFiller(msg.sender).reactorCallback(orders, fillerData);
        }
    }

    function resolve(SignedOrder calldata) external pure override returns (ResolvedOrder memory) {
        InputToken memory input = InputToken({token: address(0), amount: 0, maxAmount: 0});
        OutputToken[] memory outputs = new OutputToken[](0);
        return ResolvedOrder({
            signer: address(0),
            input: input,
            outputs: outputs,
            orderHash: bytes32(0),
            deadline: 0
        });
    }
}
