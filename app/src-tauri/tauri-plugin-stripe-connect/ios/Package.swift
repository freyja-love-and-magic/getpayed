// swift-tools-version:5.5
import PackageDescription

// Deliberately no Stripe dependency — see src/lib.rs for why the SDK lives in
// the app target instead.
let package = Package(
    name: "tauri-plugin-stripe-connect",
    platforms: [
        .iOS(.v15)
    ],
    products: [
        .library(
            name: "tauri-plugin-stripe-connect",
            type: .static,
            targets: ["tauri-plugin-stripe-connect"])
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
        .package(url: "https://github.com/Brendonovich/swift-rs", branch: "main")
    ],
    targets: [
        .target(
            name: "tauri-plugin-stripe-connect",
            dependencies: [
                .product(name: "SwiftRs", package: "swift-rs"),
                .byName(name: "Tauri")
            ],
            path: "Sources")
    ]
)
