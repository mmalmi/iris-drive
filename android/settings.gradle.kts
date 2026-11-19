pluginManagement {
    repositories {
        google()
        // Kotlin plugin metadata is pinned to the Maven Central artifacts.
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "IrisDriveAndroid"
include(":app")
