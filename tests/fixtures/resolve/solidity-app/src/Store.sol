pragma solidity ^0.8.20;
import "@oz/contracts/Ownable.sol";
import "./Math.sol";
interface IStore { function open(uint id) external returns (uint); }
contract Store is Ownable, IStore {
    event Opened(uint id);
    function open(uint id) external onlyOwner returns (uint) {
        uint n = Math.add(id, 1);
        emit Opened(n);
        return check(n);
    }
    function check(uint x) internal pure returns (uint) { return x; }
}
