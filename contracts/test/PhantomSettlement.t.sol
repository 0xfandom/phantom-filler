// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import {PhantomSettlement} from "../src/PhantomSettlement.sol";
import {FillResult} from "../src/types/OrderTypes.sol";

/// @title PhantomSettlementTest
/// @notice Comprehensive tests for PhantomSettlement contract.
contract PhantomSettlementTest is Test {
    PhantomSettlement public settlement;

    address public owner = address(0xB1);
    address public recorder = address(0xB2);
    address public alice = address(0xB3);

    event RecorderAdded(address indexed recorder);
    event RecorderRemoved(address indexed recorder);
    event FillSettled(bytes32 indexed orderHash, address indexed filler, uint256 inputAmount, uint256 outputAmount);
    event FillDisputed(bytes32 indexed orderHash, address indexed disputedBy);

    function setUp() public {
        vm.prank(owner);
        settlement = new PhantomSettlement(owner);
    }

    // ─── Helper ─────────────────────────────────────────────────────

    function _makeFillResult(bytes32 orderHash, address filler) internal pure returns (FillResult memory) {
        return FillResult({
            orderHash: orderHash,
            filler: filler,
            inputAmount: 1 ether,
            outputAmount: 2000e6,
            filledAt: 1700000000
        });
    }

    function _recordSampleFill(bytes32 orderHash) internal {
        FillResult memory result = _makeFillResult(orderHash, alice);
        vm.prank(owner);
        settlement.recordFill(result);
    }

    // ─── Constructor ────────────────────────────────────────────────

    function test_constructor_setsOwner() public view {
        assertEq(settlement.owner(), owner);
    }

    function test_constructor_authorizesOwnerAsRecorder() public view {
        assertTrue(settlement.authorizedRecorders(owner));
    }

    function test_constructor_initialTotalSettlementsIsZero() public view {
        assertEq(settlement.totalSettlements(), 0);
    }

    // ─── addRecorder ────────────────────────────────────────────────

    function test_addRecorder_success() public {
        vm.prank(owner);
        vm.expectEmit(true, false, false, false);
        emit RecorderAdded(recorder);
        settlement.addRecorder(recorder);

        assertTrue(settlement.authorizedRecorders(recorder));
    }

    function test_addRecorder_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        settlement.addRecorder(recorder);
    }

    function test_addRecorder_revertsOnZeroAddress() public {
        vm.prank(owner);
        vm.expectRevert(PhantomSettlement.ZeroAddress.selector);
        settlement.addRecorder(address(0));
    }

    // ─── removeRecorder ─────────────────────────────────────────────

    function test_removeRecorder_success() public {
        vm.startPrank(owner);
        settlement.addRecorder(recorder);
        vm.expectEmit(true, false, false, false);
        emit RecorderRemoved(recorder);
        settlement.removeRecorder(recorder);
        vm.stopPrank();

        assertFalse(settlement.authorizedRecorders(recorder));
    }

    function test_removeRecorder_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        settlement.removeRecorder(recorder);
    }

    // ─── recordFill ─────────────────────────────────────────────────

    function test_recordFill_success() public {
        bytes32 orderHash = keccak256("order1");
        FillResult memory result = _makeFillResult(orderHash, alice);

        vm.prank(owner);
        vm.expectEmit(true, true, false, true);
        emit FillSettled(orderHash, alice, 1 ether, 2000e6);
        settlement.recordFill(result);

        assertTrue(settlement.isFilled(orderHash));
        assertEq(settlement.totalSettlements(), 1);
    }

    function test_recordFill_revertsIfUnauthorizedRecorder() public {
        FillResult memory result = _makeFillResult(keccak256("order1"), alice);

        vm.prank(alice);
        vm.expectRevert(PhantomSettlement.UnauthorizedRecorder.selector);
        settlement.recordFill(result);
    }

    function test_recordFill_revertsIfAlreadySettled() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        FillResult memory result = _makeFillResult(orderHash, alice);
        vm.prank(owner);
        vm.expectRevert(PhantomSettlement.AlreadySettled.selector);
        settlement.recordFill(result);
    }

    function test_recordFill_multipleOrders() public {
        _recordSampleFill(keccak256("order1"));
        _recordSampleFill(keccak256("order2"));
        _recordSampleFill(keccak256("order3"));

        assertEq(settlement.totalSettlements(), 3);
    }

    function test_recordFill_authorizedRecorder() public {
        vm.prank(owner);
        settlement.addRecorder(recorder);

        FillResult memory result = _makeFillResult(keccak256("order1"), alice);
        vm.prank(recorder);
        settlement.recordFill(result);

        assertEq(settlement.totalSettlements(), 1);
    }

    // ─── getSettlement ──────────────────────────────────────────────

    function test_getSettlement_returnsFalseIfNotSettled() public view {
        (bool settled, address filler, uint256 settledAt) = settlement.getSettlement(keccak256("unknown"));
        assertFalse(settled);
        assertEq(filler, address(0));
        assertEq(settledAt, 0);
    }

    function test_getSettlement_returnsCorrectData() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        (bool settled, address filler, uint256 settledAt) = settlement.getSettlement(orderHash);
        assertTrue(settled);
        assertEq(filler, alice);
        assertGt(settledAt, 0);
    }

    // ─── isFilled ───────────────────────────────────────────────────

    function test_isFilled_falseIfNotRecorded() public view {
        assertFalse(settlement.isFilled(keccak256("nope")));
    }

    function test_isFilled_trueAfterRecording() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);
        assertTrue(settlement.isFilled(orderHash));
    }

    // ─── getFullSettlement ──────────────────────────────────────────

    function test_getFullSettlement_success() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        PhantomSettlement.Settlement memory s = settlement.getFullSettlement(orderHash);
        assertEq(s.filler, alice);
        assertEq(s.inputAmount, 1 ether);
        assertEq(s.outputAmount, 2000e6);
        assertGt(s.settledAt, 0);
        assertFalse(s.disputed);
    }

    function test_getFullSettlement_revertsIfNotSettled() public {
        vm.expectRevert(PhantomSettlement.NotSettled.selector);
        settlement.getFullSettlement(keccak256("unknown"));
    }

    // ─── disputeFill ────────────────────────────────────────────────

    function test_disputeFill_success() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        vm.prank(owner);
        vm.expectEmit(true, true, false, false);
        emit FillDisputed(orderHash, owner);
        settlement.disputeFill(orderHash);

        assertTrue(settlement.isDisputed(orderHash));
    }

    function test_disputeFill_revertsIfNotOwner() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        vm.prank(alice);
        vm.expectRevert();
        settlement.disputeFill(orderHash);
    }

    function test_disputeFill_revertsIfNotSettled() public {
        vm.prank(owner);
        vm.expectRevert(PhantomSettlement.NotSettled.selector);
        settlement.disputeFill(keccak256("unknown"));
    }

    // ─── resolveDispute ─────────────────────────────────────────────

    function test_resolveDispute_success() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        vm.startPrank(owner);
        settlement.disputeFill(orderHash);
        assertTrue(settlement.isDisputed(orderHash));

        settlement.resolveDispute(orderHash);
        vm.stopPrank();

        assertFalse(settlement.isDisputed(orderHash));
    }

    function test_resolveDispute_revertsIfNotOwner() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        vm.prank(owner);
        settlement.disputeFill(orderHash);

        vm.prank(alice);
        vm.expectRevert();
        settlement.resolveDispute(orderHash);
    }

    function test_resolveDispute_revertsIfNotSettled() public {
        vm.prank(owner);
        vm.expectRevert(PhantomSettlement.NotSettled.selector);
        settlement.resolveDispute(keccak256("unknown"));
    }

    // ─── isDisputed ─────────────────────────────────────────────────

    function test_isDisputed_falseByDefault() public view {
        assertFalse(settlement.isDisputed(keccak256("anything")));
    }

    function test_isDisputed_trueAfterDispute() public {
        bytes32 orderHash = keccak256("order1");
        _recordSampleFill(orderHash);

        vm.prank(owner);
        settlement.disputeFill(orderHash);

        assertTrue(settlement.isDisputed(orderHash));
    }
}
