// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "FoxBotMacProbe",
    platforms: [.macOS("26.0")],
    products: [
        .executable(name: "foxbot-macos-probe", targets: ["ProbeCLI"]),
        .executable(name: "foxbot-macos-ocr", targets: ["OCRCLI"]),
        .executable(name: "foxbot-macos-draft", targets: ["DraftCLI"]),
        .executable(name: "foxbot-macos-send-gate", targets: ["SendGateCLI"])
    ],
    targets: [
        .target(name: "ProbeKit"),
        .executableTarget(name: "ProbeCLI", dependencies: ["ProbeKit"]),
        .testTarget(name: "ProbeKitTests", dependencies: ["ProbeKit"]),
        .target(name: "OCRKit", dependencies: ["ProbeKit"]),
        .executableTarget(name: "OCRCLI", dependencies: ["OCRKit", "ProbeKit"]),
        .executableTarget(name: "DraftCLI", dependencies: ["OCRKit", "ProbeKit"]),
        .executableTarget(name: "SendGateCLI", dependencies: ["OCRKit", "ProbeKit"]),
        .testTarget(name: "OCRKitTests", dependencies: ["OCRKit", "ProbeKit"])
    ]
)
