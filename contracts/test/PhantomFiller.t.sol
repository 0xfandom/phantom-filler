// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import {PhantomFiller} from "../src/PhantomFiller.sol";
import {MockReactor} from "./mocks/MockReactor.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {SignedOrder, ResolvedOrder} from "../src/types/OrderTypes.sol";

/// @title PhantomFillerTest
/// @notice Comprehensive tests for PhantomFiller contract.
contract PhantomFillerTest is Test {
    PhantomFiller public filler;
    MockReactor public reactor;
    MockERC20 public token;

    address public owner = address(0xA1);
    address public alice = address(0xA2);
    address public bob = address(0xA3);

    event ReactorAdded(address indexed reactor);
    event ReactorRemoved(address indexed reactor);
    event FillerAuthorized(address indexed filler);
    event FillerDeauthorized(address indexed filler);
    event FillExecuted(address indexed reactor, address indexed filler);
    event TokenWithdrawn(address indexed token, address indexed to, uint256 amount);

    function setUp() public {
        vm.startPrank(owner);
        filler = new PhantomFiller(owner);
        reactor = new MockReactor();
        token = new MockERC20("Test Token", "TST", 18);
        vm.stopPrank();
    }

    // ─── Constructor ────────────────────────────────────────────────

    function test_constructor_setsOwner() public view {
        assertEq(filler.owner(), owner);
    }

    function test_constructor_authorizesOwnerAsFiller() public view {
        assertTrue(filler.authorizedFillers(owner));
    }

    // ─── addReactor ─────────────────────────────────────────────────

    function test_addReactor_success() public {
        vm.prank(owner);
        vm.expectEmit(true, false, false, false);
        emit ReactorAdded(address(reactor));
        filler.addReactor(address(reactor));

        assertTrue(filler.whitelistedReactors(address(reactor)));
    }

    function test_addReactor_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.addReactor(address(reactor));
    }

    function test_addReactor_revertsOnZeroAddress() public {
        vm.prank(owner);
        vm.expectRevert(PhantomFiller.ZeroAddress.selector);
        filler.addReactor(address(0));
    }

    // ─── removeReactor ──────────────────────────────────────────────

    function test_removeReactor_success() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.expectEmit(true, false, false, false);
        emit ReactorRemoved(address(reactor));
        filler.removeReactor(address(reactor));
        vm.stopPrank();

        assertFalse(filler.whitelistedReactors(address(reactor)));
    }

    function test_removeReactor_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.removeReactor(address(reactor));
    }

    // ─── authorizeFiller ────────────────────────────────────────────

    function test_authorizeFiller_success() public {
        vm.prank(owner);
        vm.expectEmit(true, false, false, false);
        emit FillerAuthorized(alice);
        filler.authorizeFiller(alice);

        assertTrue(filler.authorizedFillers(alice));
    }

    function test_authorizeFiller_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.authorizeFiller(bob);
    }

    function test_authorizeFiller_revertsOnZeroAddress() public {
        vm.prank(owner);
        vm.expectRevert(PhantomFiller.ZeroAddress.selector);
        filler.authorizeFiller(address(0));
    }

    // ─── deauthorizeFiller ──────────────────────────────────────────

    function test_deauthorizeFiller_success() public {
        vm.startPrank(owner);
        filler.authorizeFiller(alice);
        vm.expectEmit(true, false, false, false);
        emit FillerDeauthorized(alice);
        filler.deauthorizeFiller(alice);
        vm.stopPrank();

        assertFalse(filler.authorizedFillers(alice));
    }

    function test_deauthorizeFiller_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.deauthorizeFiller(bob);
    }

    // ─── approveToken ───────────────────────────────────────────────

    function test_approveToken_success() public {
        vm.prank(owner);
        filler.approveToken(address(token), address(reactor), 1000 ether);

        assertEq(token.allowance(address(filler), address(reactor)), 1000 ether);
    }

    function test_approveToken_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.approveToken(address(token), address(reactor), 1000 ether);
    }

    // ─── withdrawToken ──────────────────────────────────────────────

    function test_withdrawToken_success() public {
        token.mint(address(filler), 500 ether);

        vm.prank(owner);
        vm.expectEmit(true, true, false, true);
        emit TokenWithdrawn(address(token), alice, 200 ether);
        filler.withdrawToken(address(token), alice, 200 ether);

        assertEq(token.balanceOf(alice), 200 ether);
        assertEq(token.balanceOf(address(filler)), 300 ether);
    }

    function test_withdrawToken_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.withdrawToken(address(token), alice, 100 ether);
    }

    function test_withdrawToken_revertsOnZeroAddress() public {
        vm.prank(owner);
        vm.expectRevert(PhantomFiller.ZeroAddress.selector);
        filler.withdrawToken(address(token), address(0), 100 ether);
    }

    // ─── withdrawEth ────────────────────────────────────────────────

    function test_withdrawEth_success() public {
        vm.deal(address(filler), 10 ether);

        vm.prank(owner);
        filler.withdrawEth(payable(alice), 3 ether);

        assertEq(alice.balance, 3 ether);
        assertEq(address(filler).balance, 7 ether);
    }

    function test_withdrawEth_revertsIfNotOwner() public {
        vm.prank(alice);
        vm.expectRevert();
        filler.withdrawEth(payable(alice), 1 ether);
    }

    function test_withdrawEth_revertsOnZeroAddress() public {
        vm.prank(owner);
        vm.expectRevert(PhantomFiller.ZeroAddress.selector);
        filler.withdrawEth(payable(address(0)), 1 ether);
    }

    // ─── fill ───────────────────────────────────────────────────────

    function test_fill_success() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(owner);
        vm.expectEmit(true, true, false, false);
        emit FillExecuted(address(reactor), owner);
        filler.fill(address(reactor), order, "");

        assertEq(reactor.executeCallCount(), 1);
    }

    function test_fill_revertsIfUnauthorizedFiller() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(alice);
        vm.expectRevert(PhantomFiller.UnauthorizedFiller.selector);
        filler.fill(address(reactor), order, "");
    }

    function test_fill_revertsIfUnauthorizedReactor() public {
        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(owner);
        vm.expectRevert(PhantomFiller.UnauthorizedReactor.selector);
        filler.fill(address(reactor), order, "");
    }

    // ─── fillWithCallback ───────────────────────────────────────────

    function test_fillWithCallback_success() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        reactor.setCallback(true);

        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(owner);
        vm.expectEmit(true, true, false, false);
        emit FillExecuted(address(reactor), owner);
        filler.fillWithCallback(address(reactor), order, "");

        assertEq(reactor.executeWithCallbackCallCount(), 1);
    }

    function test_fillWithCallback_revertsIfUnauthorizedFiller() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(alice);
        vm.expectRevert(PhantomFiller.UnauthorizedFiller.selector);
        filler.fillWithCallback(address(reactor), order, "");
    }

    function test_fillWithCallback_revertsIfUnauthorizedReactor() public {
        SignedOrder memory order = SignedOrder({order: hex"deadbeef", sig: hex"cafebabe"});

        vm.prank(owner);
        vm.expectRevert(PhantomFiller.UnauthorizedReactor.selector);
        filler.fillWithCallback(address(reactor), order, "");
    }

    // ─── fillBatch ──────────────────────────────────────────────────

    function test_fillBatch_success() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        SignedOrder[] memory orders = new SignedOrder[](2);
        orders[0] = SignedOrder({order: hex"aa", sig: hex"bb"});
        orders[1] = SignedOrder({order: hex"cc", sig: hex"dd"});

        vm.prank(owner);
        vm.expectEmit(true, true, false, false);
        emit FillExecuted(address(reactor), owner);
        filler.fillBatch(address(reactor), orders, "");

        assertEq(reactor.executeBatchCallCount(), 1);
    }

    function test_fillBatch_revertsIfUnauthorizedFiller() public {
        vm.startPrank(owner);
        filler.addReactor(address(reactor));
        vm.stopPrank();

        SignedOrder[] memory orders = new SignedOrder[](1);
        orders[0] = SignedOrder({order: hex"aa", sig: hex"bb"});

        vm.prank(alice);
        vm.expectRevert(PhantomFiller.UnauthorizedFiller.selector);
        filler.fillBatch(address(reactor), orders, "");
    }

    function test_fillBatch_revertsIfUnauthorizedReactor() public {
        SignedOrder[] memory orders = new SignedOrder[](1);
        orders[0] = SignedOrder({order: hex"aa", sig: hex"bb"});

        vm.prank(owner);
        vm.expectRevert(PhantomFiller.UnauthorizedReactor.selector);
        filler.fillBatch(address(reactor), orders, "");
    }

    // ─── reactorCallback ────────────────────────────────────────────

    function test_reactorCallback_revertsIfNotWhitelistedReactor() public {
        vm.prank(alice);
        vm.expectRevert(PhantomFiller.UnauthorizedReactor.selector);
        filler.reactorCallback(new ResolvedOrder[](0), "");
    }

    // ─── receive ────────────────────────────────────────────────────

    function test_receive_acceptsEth() public {
        vm.deal(alice, 5 ether);
        vm.prank(alice);
        (bool success,) = address(filler).call{value: 1 ether}("");
        assertTrue(success);
        assertEq(address(filler).balance, 1 ether);
    }
}
