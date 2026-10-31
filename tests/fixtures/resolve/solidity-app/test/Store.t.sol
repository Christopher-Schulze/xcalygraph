pragma solidity ^0.8.20;
import "../src/Store.sol";
contract StoreTest {
    Store store;
    function testOpen() public {
        store = new Store();
        store.open(1);
    }
}
