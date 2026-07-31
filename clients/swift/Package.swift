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
                .linkedLibrary("openkache_client_core"),
            ],
            plugins: [
                .plugin(name: "GenerateSmithy"),
            ]
        ),
        .plugin(
            name: "GenerateSmithy",
            capability: .buildTool(),
            path: "Plugins/GenerateSmithy"
        ),
    ]
)
