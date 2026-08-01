import org.gradle.api.tasks.Exec

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

kotlin {
    jvmToolchain(21)
}

sourceSets {
    main {
        java.srcDir("../java/src/main/java")
    }
}

tasks.withType<JavaCompile>().configureEach {
    options.compilerArgs.add("--enable-preview")
}

val generateSmithyContracts by tasks.registering(Exec::class) {
    commandLine("bun", "../generate.ts")
    environment("OPENKACHE_GENERATION_TARGET", "kotlin")
}

tasks.named("compileKotlin") {
    dependsOn(generateSmithyContracts)
}

tasks.named("compileJava") {
    dependsOn(generateSmithyContracts)
}
