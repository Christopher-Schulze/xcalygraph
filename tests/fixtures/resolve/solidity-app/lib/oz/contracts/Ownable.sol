pragma solidity ^0.8.20;
abstract contract Ownable {
    modifier onlyOwner() { _; }
    function owner() public view returns (address) { return address(0); }
}
