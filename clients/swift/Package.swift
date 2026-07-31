// swift-tools-version: 5.10

import PackageDescription

let package = Package(
    name: "OpenKache",
    products: [
        .library(name: "OpenKache", targets: ["OpenKache"]),
    ],
    targets: [
        .target(
            name: "OpenKache",
            linkerSettings: [
                .linkedLibrary("openkache_client"),
            ]
        ),
    ]
)
