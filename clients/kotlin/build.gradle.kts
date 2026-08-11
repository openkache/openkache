plugins {
    kotlin("jvm") version "2.2.0"
    `java-library`
}

group = "io.openkache"
version = "0.1.0-SNAPSHOT"

description = "Thin Kotlin binding for the OpenKache Rust client"

repositories {
    mavenCentral()
}

dependencies {
    implementation("net.java.dev.jna:jna:5.16.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
}

kotlin {
    jvmToolchain(21)
}

val generateSmithyContract by tasks.registering(Exec::class) {
    workingDir(projectDir)
    commandLine("bun", "../generate.ts")
    environment("OPENKACHE_GENERATION_TARGET", "kotlin")
}

tasks.named("compileKotlin") {
    dependsOn(generateSmithyContract)
}
