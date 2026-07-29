// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "OpenKache",
    products: [
        .library(name: "OpenKache", targets: ["OpenKache"]),
    ],
    targets: [
        .target(name: "OpenKache"),
    ]
)
