// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "tauri-plugin-share-sheet",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "tauri-plugin-share-sheet",
            type: .static,
            targets: ["tauri-plugin-share-sheet"])
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
        .package(url: "https://github.com/Brendonovich/swift-rs", branch: "main")
    ],
    targets: [
        .target(
            name: "tauri-plugin-share-sheet",
            dependencies: [
                .product(name: "SwiftRs", package: "swift-rs"),
                .byName(name: "Tauri")
            ],
            path: "Sources")
    ]
)
