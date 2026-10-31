pragma solidity ^0.8.20;
import "./Base.sol";
import {IStore} from "@oz/contracts/IStore.sol";
library Math { function add(uint a, uint b) internal pure returns (uint) { return a + b; } }
contract Store is Base, IStore {
    using Math for uint;
    mapping(uint => uint) rows;
    event Opened(uint id);
    modifier onlyOwner() { _; }
    function open(uint id) external onlyOwner returns (uint) {
        uint n = Math.add(id, 1);
        emit Opened(n);
        helper(n);
        return rows[id].add(2);
    }
    function helper(uint x) internal {}
}
