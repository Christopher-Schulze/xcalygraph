// swift-tools-version:5.9
import PackageDescription

let package = Package(name: "Demo", targets: [.target(name: "Store"), .executableTarget(name: "App", dependencies: ["Store"]), .testTarget(name: "StoreTests", dependencies: ["Store"])])
