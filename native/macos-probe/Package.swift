// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "FoxBotMacProbe",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "foxbot-macos-probe", targets: ["ProbeCLI"])],
    targets: [
        .target(name: "ProbeKit"),
        .executableTarget(name: "ProbeCLI", dependencies: ["ProbeKit"]),
        .testTarget(name: "ProbeKitTests", dependencies: ["ProbeKit"])
    ]
)
