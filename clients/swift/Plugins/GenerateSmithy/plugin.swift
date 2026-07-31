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
            .appending("..")
            .appending("protocol")
            .appending("generate.ts")
        let outputDirectory = context.pluginWorkDirectory.appending("Generated")
        let output = outputDirectory.appending("SmithyAPI.swift")
        let path = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin"

        return [
            .prebuildCommand(
                displayName: "Generate OpenKache Smithy Swift declarations",
                executable: Path("/usr/bin/env"),
                arguments: ["bun", generator.string],
                environment: [
                    "OPENKACHE_GENERATION_TARGET": "swift",
                    "OPENKACHE_SWIFT_API_OUTPUT": output.string,
                    "PATH": path,
                ],
                outputFilesDirectory: outputDirectory
            ),
        ]
    }
}
