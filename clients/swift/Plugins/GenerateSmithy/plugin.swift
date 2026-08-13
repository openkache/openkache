import PackagePlugin
import Foundation

@main
struct GenerateSmithyPlugin: BuildToolPlugin {
    func createBuildCommands(
        context: PluginContext,
        target: Target
    ) async throws -> [Command] {
        guard target is SourceModuleTarget else {
            return []
        }

        let generator = context.package.directory
            .appending("..")
            .appending("generate.ts")
        let outputDirectory = context.pluginWorkDirectory.appending("Generated")
        let output = outputDirectory.appending("SmithyAPI.swift")
        let operationsOutput = outputDirectory.appending("SmithyOperations.swift")
        let nativeOutput = outputDirectory.appending("SmithyNativeABI.swift")
        let path = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin"

        return [
            .prebuildCommand(
                displayName: "Generate OpenKache Smithy Swift declarations",
                executable: Path("/usr/bin/env"),
                arguments: ["bun", generator.string],
                environment: [
                    "OPENKACHE_GENERATION_TARGET": "swift",
                    "OPENKACHE_SWIFT_API_OUTPUT": output.string,
                    "OPENKACHE_SWIFT_OPERATIONS_OUTPUT": operationsOutput.string,
                    "OPENKACHE_SWIFT_NATIVE_ABI_OUTPUT": nativeOutput.string,
                    "PATH": path,
                ],
                outputFilesDirectory: outputDirectory
            ),
        ]
    }
}
