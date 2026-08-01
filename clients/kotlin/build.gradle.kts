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
