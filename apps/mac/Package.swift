// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "SjelMac",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "SjelRelay",
            targets: ["SjelRelay"]
        ),
        .executable(
            name: "SjelMacApp",
            targets: ["SjelMacApp"]
        )
    ],
    targets: [
        .target(
            name: "SjelRelay",
            path: "Sources/SjelRelay"
        ),
        .executableTarget(
            name: "SjelMacApp",
            dependencies: ["SjelRelay"],
            path: "Sources/SjelMacApp"
        ),
        .testTarget(
            name: "SjelRelayTests",
            dependencies: ["SjelRelay"],
            path: "Tests/SjelRelayTests"
        )
    ]
)
